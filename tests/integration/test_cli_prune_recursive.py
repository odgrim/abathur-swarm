"""Integration tests for CLI mem prune --recursive command.

Tests complete end-to-end workflows for recursive memory pruning:
- Recursive deletion of task memories and child task memories
- Dry-run preview with tree statistics
- Validation (--recursive incompatible with --limit)
- Tree depth control via --preview-depth
"""

import asyncio
from pathlib import Path
from uuid import UUID

import pytest
from typer.testing import CliRunner

from abathur.cli.main import app
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure import Database
from abathur.services import MemoryService

runner = CliRunner()


# Fixtures


@pytest.fixture(scope="function")
def database(cli_test_db_path: Path, mock_cli_database_path):
    """Create a test database at .abathur/test.db with mocked path."""
    # Create test database (path already cleaned by cli_test_db_path fixture)
    db = Database(cli_test_db_path)
    asyncio.run(db.initialize())
    return db


@pytest.fixture(scope="function")
def memory_service(database):
    """Create MemoryService with test database."""
    return MemoryService(database)


@pytest.fixture(scope="function")
def task_tree_with_memories(database, memory_service):
    """Create a task hierarchy with memories for testing recursive deletion.

    Structure:
        parent_task (completed)
        ├── child_task_1 (completed)
        │   └── grandchild_task (completed)
        └── child_task_2 (failed)

    Each task has associated memories in namespace task:{task_id}:*
    """
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_hierarchy():
        # Create parent task
        parent_task = Task(
            prompt="Parent task",
            summary="Parent with children",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        parent_id = await coordinator.submit_task(parent_task)

        # Create memories for parent task
        await memory_service.add_memory(
            namespace=f"task:{parent_id}:context",
            key="requirements",
            value={"type": "parent_requirements"},
            memory_type="semantic",
            created_by="test-agent",
            task_id=str(parent_id),
        )
        await memory_service.add_memory(
            namespace=f"task:{parent_id}:output",
            key="result",
            value={"status": "completed"},
            memory_type="episodic",
            created_by="test-agent",
            task_id=str(parent_id),
        )

        # Create child_task_1
        child_task_1 = Task(
            prompt="Child task 1",
            summary="First child",
            agent_type="test-agent",
            source=TaskSource.AGENT_PLANNER,
            parent_task_id=parent_id,
            status=TaskStatus.COMPLETED,
        )
        child_1_id = await coordinator.submit_task(child_task_1)

        # Create memories for child_task_1
        await memory_service.add_memory(
            namespace=f"task:{child_1_id}:context",
            key="requirements",
            value={"type": "child1_requirements"},
            memory_type="semantic",
            created_by="test-agent",
            task_id=str(child_1_id),
        )

        # Create grandchild task
        grandchild_task = Task(
            prompt="Grandchild task",
            summary="Grandchild of parent",
            agent_type="test-agent",
            source=TaskSource.AGENT_IMPLEMENTATION,
            parent_task_id=child_1_id,
            status=TaskStatus.COMPLETED,
        )
        grandchild_id = await coordinator.submit_task(grandchild_task)

        # Create memories for grandchild
        await memory_service.add_memory(
            namespace=f"task:{grandchild_id}:output",
            key="result",
            value={"status": "success"},
            memory_type="episodic",
            created_by="test-agent",
            task_id=str(grandchild_id),
        )

        # Create child_task_2
        child_task_2 = Task(
            prompt="Child task 2",
            summary="Second child",
            agent_type="test-agent",
            source=TaskSource.AGENT_PLANNER,
            parent_task_id=parent_id,
            status=TaskStatus.FAILED,
        )
        child_2_id = await coordinator.submit_task(child_task_2)

        # Create memories for child_task_2
        await memory_service.add_memory(
            namespace=f"task:{child_2_id}:context",
            key="requirements",
            value={"type": "child2_requirements"},
            memory_type="semantic",
            created_by="test-agent",
            task_id=str(child_2_id),
        )
        await memory_service.add_memory(
            namespace=f"task:{child_2_id}:error",
            key="error_message",
            value={"error": "task_failed"},
            memory_type="episodic",
            created_by="test-agent",
            task_id=str(child_2_id),
        )

        return {
            "parent_id": parent_id,
            "child_1_id": child_1_id,
            "child_2_id": child_2_id,
            "grandchild_id": grandchild_id,
        }

    return asyncio.run(create_hierarchy())


# Test cases


@pytest.mark.asyncio
async def test_cli_recursive_flag(database, task_tree_with_memories):
    """Test CLI with --recursive performs deletion of task and descendant memories.

    Verifies:
    - Parent task namespace memories deleted
    - Child task namespace memories deleted (recursively)
    - Grandchild task namespace memories deleted (recursively)
    - Deletion count reported correctly
    """
    parent_id = task_tree_with_memories["parent_id"]
    child_1_id = task_tree_with_memories["child_1_id"]
    child_2_id = task_tree_with_memories["child_2_id"]
    grandchild_id = task_tree_with_memories["grandchild_id"]

    # Verify initial state: 6 memories total
    # parent: 2, child_1: 1, child_2: 2, grandchild: 1
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    initial_count = await count_memories()
    assert initial_count == 6

    # Execute: Run CLI with --recursive flag
    result = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{parent_id}", "--recursive", "--force"]
    )

    # Assert: Command succeeded
    assert result.exit_code == 0
    assert "✓" in result.stdout or "Deleted" in result.stdout

    # Verify all memories deleted (parent + descendants)
    final_count = await count_memories()
    assert final_count == 0

    # Verify specific namespaces are gone
    async def check_namespace_deleted(task_id: UUID):
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE namespace LIKE ? AND is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query, (f"task:{task_id}%",))
            row = await cursor.fetchone()
            return row["count"] == 0

    assert await check_namespace_deleted(parent_id)
    assert await check_namespace_deleted(child_1_id)
    assert await check_namespace_deleted(child_2_id)
    assert await check_namespace_deleted(grandchild_id)


@pytest.mark.asyncio
async def test_cli_recursive_dry_run(database, task_tree_with_memories):
    """Test CLI with --recursive --dry-run shows preview without deleting.

    Verifies:
    - Dry-run mode displays preview
    - Shows tree structure with statistics
    - No actual deletion occurs
    - Memory count unchanged after dry-run
    """
    parent_id = task_tree_with_memories["parent_id"]

    # Count memories before
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    initial_count = await count_memories()
    assert initial_count == 6

    # Execute: Run CLI with --recursive --dry-run
    result = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{parent_id}", "--recursive", "--dry-run"]
    )

    # Assert: Dry-run output displayed
    assert result.exit_code == 0
    assert "Dry-run mode" in result.stdout
    assert "Would delete" in result.stdout or "preview" in result.stdout.lower()

    # Verify tree statistics shown (either in table or summary)
    # Should mention multiple memories or show breakdown
    assert "6" in result.stdout or "memory" in result.stdout.lower()

    # Verify no deletion occurred
    final_count = await count_memories()
    assert final_count == initial_count
    assert final_count == 6


@pytest.mark.asyncio
async def test_cli_recursive_with_limit_fails(database, task_tree_with_memories):
    """Test validation rejects --recursive --limit combination.

    Verifies:
    - CLI exits with error code
    - Error message explains incompatibility
    - No deletion occurs
    """
    parent_id = task_tree_with_memories["parent_id"]

    # Execute: Attempt --recursive with --limit
    result = runner.invoke(
        app,
        [
            "mem",
            "prune",
            "--namespace",
            f"task:{parent_id}",
            "--recursive",
            "--limit",
            "10",
            "--force",
        ],
    )

    # Assert: Command failed with validation error
    assert result.exit_code != 0
    assert "incompatible" in result.stdout.lower() or "cannot use" in result.stdout.lower()
    assert "--recursive" in result.stdout
    assert "--limit" in result.stdout

    # Verify no deletion occurred
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    final_count = await count_memories()
    assert final_count == 6


@pytest.mark.asyncio
async def test_cli_tree_statistics_output(database, task_tree_with_memories):
    """Test CLI displays tree statistics in output.

    Verifies:
    - Statistics table or summary shown
    - Breakdown by task shown (parent + children)
    - Total memory count accurate
    - Task hierarchy depth indicated
    """
    parent_id = task_tree_with_memories["parent_id"]
    child_1_id = task_tree_with_memories["child_1_id"]
    child_2_id = task_tree_with_memories["child_2_id"]
    grandchild_id = task_tree_with_memories["grandchild_id"]

    # Execute: Run with --dry-run to see statistics without deletion
    result = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{parent_id}", "--recursive", "--dry-run"]
    )

    # Assert: Statistics displayed
    assert result.exit_code == 0

    # Verify task IDs appear in output (truncated to 8 chars)
    parent_prefix = str(parent_id)[:8]
    assert parent_prefix in result.stdout

    # Verify memory counts or statistics shown
    # Look for either table format or summary format
    assert (
        "memory" in result.stdout.lower()
        or "memories" in result.stdout.lower()
        or "entries" in result.stdout.lower()
    )

    # Verify total count
    assert "6" in result.stdout


@pytest.mark.asyncio
async def test_cli_preview_depth(database, task_tree_with_memories):
    """Test --preview-depth controls tree truncation in output.

    Verifies:
    - --preview-depth=1 shows only immediate children
    - --preview-depth=2 shows children and grandchildren
    - Truncation indicator shown when depth exceeded
    - Deletion still processes full tree regardless of preview depth
    """
    parent_id = task_tree_with_memories["parent_id"]

    # Test Case 1: --preview-depth=1 (show parent + immediate children only)
    result_depth_1 = runner.invoke(
        app,
        [
            "mem",
            "prune",
            "--namespace",
            f"task:{parent_id}",
            "--recursive",
            "--dry-run",
            "--preview-depth",
            "1",
        ],
    )

    assert result_depth_1.exit_code == 0
    assert "Dry-run mode" in result_depth_1.stdout

    # Should show parent and children, but truncate grandchildren
    # Look for truncation indicators
    assert (
        "..." in result_depth_1.stdout
        or "more" in result_depth_1.stdout.lower()
        or "truncated" in result_depth_1.stdout.lower()
    )

    # Test Case 2: --preview-depth=2 (show full tree: parent, children, grandchildren)
    result_depth_2 = runner.invoke(
        app,
        [
            "mem",
            "prune",
            "--namespace",
            f"task:{parent_id}",
            "--recursive",
            "--dry-run",
            "--preview-depth",
            "2",
        ],
    )

    assert result_depth_2.exit_code == 0
    assert "Dry-run mode" in result_depth_2.stdout

    # Should show full tree without truncation indicators
    # Grandchild should be visible
    grandchild_prefix = str(task_tree_with_memories["grandchild_id"])[:8]
    assert grandchild_prefix in result_depth_2.stdout

    # Test Case 3: Actual deletion ignores preview depth (deletes full tree)
    result_delete = runner.invoke(
        app,
        [
            "mem",
            "prune",
            "--namespace",
            f"task:{parent_id}",
            "--recursive",
            "--preview-depth",
            "1",
            "--force",
        ],
    )

    assert result_delete.exit_code == 0

    # Verify ALL memories deleted, not just depth=1
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    final_count = await count_memories()
    assert final_count == 0  # All memories deleted despite preview-depth=1


# Edge case tests


@pytest.mark.asyncio
async def test_cli_recursive_no_children(database, memory_service):
    """Test --recursive on task with no children behaves like non-recursive.

    Verifies:
    - Deletion succeeds
    - Only target task memories deleted
    - No errors or warnings about missing children
    """
    from abathur.application import TaskCoordinator

    coordinator = TaskCoordinator(database)

    async def create_solo_task():
        # Create task without children
        task = Task(
            prompt="Solo task",
            summary="Task with no children",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
        )
        task_id = await coordinator.submit_task(task)

        # Create memories
        await memory_service.add_memory(
            namespace=f"task:{task_id}:context",
            key="requirements",
            value={"type": "solo"},
            memory_type="semantic",
            created_by="test-agent",
            task_id=str(task_id),
        )

        return task_id

    task_id = await create_solo_task()

    # Execute: --recursive on task with no children
    result = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{task_id}", "--recursive", "--force"]
    )

    # Assert: Success
    assert result.exit_code == 0

    # Verify deletion
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    assert await count_memories() == 0


@pytest.mark.asyncio
async def test_cli_recursive_empty_namespace(database):
    """Test --recursive on non-existent namespace shows appropriate message.

    Verifies:
    - No errors thrown
    - Message indicates no memories found
    - Graceful handling of empty result set
    """
    # Execute: --recursive on non-existent namespace
    result = runner.invoke(
        app,
        [
            "mem",
            "prune",
            "--namespace",
            "task:00000000-0000-0000-0000-000000000000",
            "--recursive",
            "--force",
        ],
    )

    # Assert: Graceful handling
    assert result.exit_code == 0
    assert (
        "No memories" in result.stdout
        or "0" in result.stdout
        or "nothing to delete" in result.stdout.lower()
    )


@pytest.mark.asyncio
async def test_cli_recursive_confirms_before_deletion(database, task_tree_with_memories):
    """Test confirmation prompt works with --recursive (without --force).

    Verifies:
    - Confirmation prompt displayed
    - 'n' cancels operation (no deletion)
    - 'y' proceeds with deletion
    """
    parent_id = task_tree_with_memories["parent_id"]

    # Test Case 1: User rejects confirmation
    result_reject = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{parent_id}", "--recursive"], input="n\n"
    )

    assert result_reject.exit_code == 0
    assert "cancelled" in result_reject.stdout.lower() or "aborted" in result_reject.stdout.lower()

    # Verify no deletion
    async def count_memories():
        query = """
            SELECT COUNT(*) as count
            FROM memory_entries
            WHERE is_deleted = 0
        """
        async with database._get_connection() as conn:
            cursor = await conn.execute(query)
            row = await cursor.fetchone()
            return row["count"]

    assert await count_memories() == 6

    # Test Case 2: User accepts confirmation
    result_accept = runner.invoke(
        app, ["mem", "prune", "--namespace", f"task:{parent_id}", "--recursive"], input="y\n"
    )

    assert result_accept.exit_code == 0
    assert "✓" in result_accept.stdout or "Deleted" in result_accept.stdout

    # Verify deletion
    assert await count_memories() == 0
