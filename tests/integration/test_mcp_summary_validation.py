"""Integration tests for MCP summary field validation.

Tests comprehensive validation including:
- Whitespace trimming
- Empty/whitespace-only rejection
- Max length enforcement
- Unicode character handling
"""

from uuid import UUID

import pytest

from abathur.infrastructure.database import Database
from abathur.mcp.task_queue_server import AbathurTaskQueueServer
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService


@pytest.fixture
async def mcp_server(temp_db_path):
    """Create and initialize MCP server for testing."""
    server = AbathurTaskQueueServer(temp_db_path)
    server._db = Database(temp_db_path)
    await server._db.initialize()

    # Initialize services
    server._dependency_resolver = DependencyResolver(server._db)
    server._priority_calculator = PriorityCalculator(server._dependency_resolver)
    server._task_queue_service = TaskQueueService(
        server._db,
        server._dependency_resolver,
        server._priority_calculator,
    )

    return server


@pytest.mark.asyncio
async def test_mcp_rejects_whitespace_only_summary(mcp_server):
    """Test that whitespace-only summary is rejected."""
    # Test whitespace-only summary
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task",
            "source": "human",
            "summary": "   ",  # Only whitespace
        }
    )

    assert result["error"] == "ValidationError"
    assert "empty or whitespace-only" in result["message"]


@pytest.mark.asyncio
async def test_mcp_rejects_empty_summary_after_trim(mcp_server):
    """Test that empty string summary is rejected after trimming."""
    # Test empty summary after strip
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task",
            "source": "human",
            "summary": "",  # Empty string
        }
    )

    assert result["error"] == "ValidationError"
    assert "empty or whitespace-only" in result["message"]


@pytest.mark.asyncio
async def test_mcp_trims_whitespace(mcp_server):
    """Test that leading/trailing whitespace is trimmed from summary."""
    # Test whitespace trimming
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task with whitespace",
            "source": "human",
            "summary": "  Test Summary  ",  # Leading/trailing whitespace
        }
    )

    # Should succeed (not an error)
    assert "error" not in result
    assert "task_id" in result

    # Verify the summary was trimmed in the database
    task = await mcp_server._db.get_task(UUID(result["task_id"]))
    assert task.summary == "Test Summary"  # Whitespace trimmed


@pytest.mark.asyncio
async def test_mcp_rejects_141_char_summary(mcp_server):
    """Test that summary exceeding 140 characters is rejected."""
    # Test 141 characters (exceeds max)
    long_summary = "a" * 141
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task",
            "source": "human",
            "summary": long_summary,
        }
    )

    assert result["error"] == "ValidationError"
    assert "must not exceed 140 characters" in result["message"]


@pytest.mark.asyncio
async def test_mcp_accepts_140_char_summary(mcp_server):
    """Test that summary with exactly 140 characters is accepted."""
    # Test exactly 140 characters
    exact_summary = "a" * 140
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task",
            "source": "human",
            "summary": exact_summary,
        }
    )

    assert "error" not in result
    assert "task_id" in result


@pytest.mark.asyncio
async def test_mcp_handles_unicode_characters(mcp_server):
    """Test that Unicode characters are handled correctly in summary."""
    # Test Unicode characters (emoji, accents, etc.)
    unicode_summary = "Test task with emoji 🎉 and accents éàü"
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task with unicode",
            "source": "human",
            "summary": unicode_summary,
        }
    )

    assert "error" not in result
    assert "task_id" in result

    # Verify the summary was stored correctly
    task = await mcp_server._db.get_task(UUID(result["task_id"]))
    assert task.summary == unicode_summary


@pytest.mark.asyncio
async def test_mcp_unicode_length_validation(mcp_server):
    """Test that Unicode multi-byte characters count correctly for length validation."""
    # Test that emoji counts as 1 character (not bytes)
    # 140 regular chars + 1 emoji = 141 characters (should fail)
    emoji_summary = "a" * 140 + "🎉"
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task",
            "source": "human",
            "summary": emoji_summary,
        }
    )

    # Should fail because len() counts characters, not bytes
    assert result["error"] == "ValidationError"
    assert "must not exceed 140 characters" in result["message"]


@pytest.mark.asyncio
async def test_mcp_none_summary_accepted(mcp_server):
    """Test that None summary is accepted (optional parameter)."""
    # Test None summary (should succeed and auto-generate)
    result = await mcp_server._handle_task_enqueue(
        {
            "description": "Test task without summary",
            "source": "human",
            # summary not provided (None)
        }
    )

    assert "error" not in result
    assert "task_id" in result

    # Verify summary was auto-generated
    task = await mcp_server._db.get_task(UUID(result["task_id"]))
    assert task.summary is not None  # Should be auto-generated
    assert len(task.summary) <= 140  # Should be within limits
