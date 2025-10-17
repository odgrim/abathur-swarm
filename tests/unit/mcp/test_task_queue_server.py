"""Unit tests for Task Queue MCP Server.

Tests cover all MCP tool handlers with mocked TaskQueueService to isolate
MCP server logic from business logic.

Test Categories:
1. Server Initialization
2. task_enqueue handler - input validation, success/error cases
3. task_get handler - success, not found, invalid UUID
4. task_list handler - filtering, pagination, validation
5. task_queue_status handler - statistics aggregation
6. task_cancel handler - cancellation and cascade
7. task_execution_plan handler - topological sort
8. Input Validation - all parameter types
9. Error Handling - all error types and formatting

Coverage Target: >90% for MCP server code
"""

from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.services.dependency_resolver import CircularDependencyError
from abathur.services.task_queue_service import TaskNotFoundError, TaskQueueError


# Mock the AbathurTaskQueueServer class (implementation doesn't exist yet)
class MockAbathurTaskQueueServer:
    """Mock MCP server for testing tool handlers."""

    def __init__(self, db_path: Path):
        self.db_path = db_path
        self.db = None
        self.task_queue_service = None
        self.dependency_resolver = None
        self.priority_calculator = None
        self.server = MagicMock()
        self.server.name = "abathur-task-queue"

    async def _initialize_services(self):
        """Mock service initialization."""
        # Services will be mocked in tests
        pass

    async def _handle_task_enqueue(self, arguments: dict) -> dict:
        """Handle task_enqueue tool invocation.

        Input validation and delegation to TaskQueueService.
        """
        # Required parameters
        if "description" not in arguments:
            return {"error": "ValidationError", "message": "description is required"}
        if "source" not in arguments:
            return {"error": "ValidationError", "message": "source is required"}

        description = arguments["description"]
        source = arguments["source"]

        # Optional parameters with defaults
        agent_type = arguments.get("agent_type", "requirements-gatherer")
        base_priority = arguments.get("base_priority", 5)
        prerequisites = arguments.get("prerequisites", [])
        deadline = arguments.get("deadline")
        estimated_duration_seconds = arguments.get("estimated_duration_seconds")
        session_id = arguments.get("session_id")
        input_data = arguments.get("input_data", {})
        parent_task_id = arguments.get("parent_task_id")

        # Validate priority range
        if not isinstance(base_priority, int) or not 0 <= base_priority <= 10:
            return {
                "error": "ValidationError",
                "message": f"base_priority must be an integer in range [0, 10], got {base_priority}",
            }

        # Validate agent_type - reject generic/invalid agent types
        invalid_agent_types = [
            "general-purpose",
            "general",
            "python-backend-developer",
            "implementation-specialist",
            "backend-developer",
            "frontend-developer",
            "developer",
        ]
        if agent_type.lower() in invalid_agent_types:
            return {
                "error": "ValidationError",
                "message": (
                    f"Invalid agent_type: '{agent_type}'. "
                    "Generic agent types are not allowed. "
                    "You must use a hyperspecialized agent type from the agent registry. "
                    "Valid examples: 'requirements-gatherer', 'task-planner', "
                    "'technical-requirements-specialist', 'agent-creator', etc. "
                    "If you need a specialized implementation agent, ensure it was created "
                    "by the agent-creator first."
                ),
            }

        # Validate source enum
        valid_sources = ["human", "agent_requirements", "agent_planner", "agent_implementation"]
        if source not in valid_sources:
            return {
                "error": "ValidationError",
                "message": f"Invalid source: {source}. Must be one of {valid_sources}",
            }

        # Parse UUIDs
        try:
            prerequisite_uuids = [UUID(pid) for pid in prerequisites]
        except (ValueError, AttributeError) as e:
            return {"error": "ValidationError", "message": f"Invalid prerequisite UUID: {e}"}

        parent_uuid = None
        if parent_task_id:
            try:
                parent_uuid = UUID(parent_task_id)
            except (ValueError, AttributeError) as e:
                return {"error": "ValidationError", "message": f"Invalid parent_task_id UUID: {e}"}

        # Parse deadline
        deadline_dt = None
        if deadline:
            try:
                deadline_dt = datetime.fromisoformat(deadline.replace("Z", "+00:00"))
            except ValueError as e:
                return {"error": "ValidationError", "message": f"Invalid deadline format: {e}"}

        # Call service
        try:
            task = await self.task_queue_service.enqueue_task(
                description=description,
                source=TaskSource(source),
                parent_task_id=parent_uuid,
                prerequisites=prerequisite_uuids,
                base_priority=base_priority,
                deadline=deadline_dt,
                estimated_duration_seconds=estimated_duration_seconds,
                agent_type=agent_type,
                session_id=session_id,
                input_data=input_data,
            )

            return {
                "task_id": str(task.id),
                "status": task.status.value,
                "calculated_priority": task.calculated_priority,
                "dependency_depth": task.dependency_depth,
                "submitted_at": task.submitted_at.isoformat(),
            }

        except ValueError as e:
            return {"error": "ValidationError", "message": str(e)}
        except CircularDependencyError as e:
            return {"error": "CircularDependencyError", "message": str(e)}
        except TaskQueueError as e:
            return {"error": "TaskQueueError", "message": str(e)}
        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_get(self, arguments: dict) -> dict:
        """Handle task_get tool invocation."""
        if "task_id" not in arguments:
            return {"error": "ValidationError", "message": "task_id is required"}

        task_id_str = arguments["task_id"]

        # Parse and validate UUID
        try:
            task_id = UUID(task_id_str)
        except (ValueError, AttributeError):
            return {"error": "ValidationError", "message": f"Invalid UUID format: {task_id_str}"}

        # Get task from database
        try:
            task = await self.db.get_task(task_id)

            if not task:
                return {
                    "error": "NotFoundError",
                    "message": f"Task {task_id} not found",
                }

            return self._serialize_task(task)

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_list(self, arguments: dict) -> dict:
        """Handle task_list tool invocation."""
        # Optional filters
        status_filter = arguments.get("status")
        limit = arguments.get("limit", 50)
        source_filter = arguments.get("source")
        agent_type_filter = arguments.get("agent_type")

        # Validate limit
        if not isinstance(limit, int) or limit < 1:
            return {
                "error": "ValidationError",
                "message": f"limit must be positive integer, got {limit}",
            }

        if limit > 500:
            return {"error": "ValidationError", "message": f"limit cannot exceed 500, got {limit}"}

        # Validate status enum
        if status_filter:
            valid_statuses = [
                "pending",
                "blocked",
                "ready",
                "running",
                "completed",
                "failed",
                "cancelled",
            ]
            if status_filter not in valid_statuses:
                return {
                    "error": "ValidationError",
                    "message": f"Invalid status: {status_filter}. Must be one of {valid_statuses}",
                }

        # Validate source enum
        if source_filter:
            valid_sources = ["human", "agent_requirements", "agent_planner", "agent_implementation"]
            if source_filter not in valid_sources:
                return {
                    "error": "ValidationError",
                    "message": f"Invalid source: {source_filter}. Must be one of {valid_sources}",
                }

        # Query database
        try:
            # Build query filters
            filters = {}
            if status_filter:
                filters["status"] = TaskStatus(status_filter)
            if source_filter:
                filters["source"] = TaskSource(source_filter)
            if agent_type_filter:
                filters["agent_type"] = agent_type_filter

            tasks = await self.db.list_tasks(limit=limit, **filters)

            return {"tasks": [self._serialize_task(task) for task in tasks]}

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_queue_status(self, arguments: dict) -> dict:
        """Handle task_queue_status tool invocation."""
        try:
            status = await self.task_queue_service.get_queue_status()

            # Serialize datetime fields
            result = {**status}
            if status.get("oldest_pending"):
                result["oldest_pending"] = status["oldest_pending"].isoformat()
            if status.get("newest_task"):
                result["newest_task"] = status["newest_task"].isoformat()

            return result

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_cancel(self, arguments: dict) -> dict:
        """Handle task_cancel tool invocation."""
        if "task_id" not in arguments:
            return {"error": "ValidationError", "message": "task_id is required"}

        task_id_str = arguments["task_id"]

        # Parse and validate UUID
        try:
            task_id = UUID(task_id_str)
        except (ValueError, AttributeError):
            return {"error": "ValidationError", "message": f"Invalid UUID format: {task_id_str}"}

        # Cancel task
        try:
            cancelled_ids = await self.task_queue_service.cancel_task(task_id)

            # First ID is the requested task, rest are cascaded
            return {
                "cancelled_task_id": str(cancelled_ids[0]) if cancelled_ids else str(task_id),
                "cascaded_task_ids": [str(tid) for tid in cancelled_ids[1:]],
                "total_cancelled": len(cancelled_ids),
            }

        except TaskNotFoundError as e:
            return {"error": "NotFoundError", "message": str(e)}
        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_execution_plan(self, arguments: dict) -> dict:
        """Handle task_execution_plan tool invocation."""
        if "task_ids" not in arguments:
            return {"error": "ValidationError", "message": "task_ids is required"}

        task_ids_str = arguments["task_ids"]

        if not isinstance(task_ids_str, list):
            return {"error": "ValidationError", "message": "task_ids must be an array"}

        # Parse UUIDs
        try:
            task_ids = [UUID(tid) for tid in task_ids_str]
        except (ValueError, AttributeError) as e:
            return {"error": "ValidationError", "message": f"Invalid UUID in task_ids: {e}"}

        # Get execution plan
        try:
            batches = await self.task_queue_service.get_task_execution_plan(task_ids)

            # Calculate max parallelism
            max_parallelism = max(len(batch) for batch in batches) if batches else 0

            return {
                "batches": [[str(tid) for tid in batch] for batch in batches],
                "total_batches": len(batches),
                "max_parallelism": max_parallelism,
            }

        except CircularDependencyError as e:
            return {"error": "CircularDependencyError", "message": str(e)}
        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    def _serialize_task(self, task: Task) -> dict:
        """Serialize Task object to JSON-compatible dict."""
        return {
            "id": str(task.id),
            "prompt": task.prompt,
            "agent_type": task.agent_type,
            "priority": task.priority,
            "status": task.status.value,
            "calculated_priority": task.calculated_priority,
            "dependency_depth": task.dependency_depth,
            "source": task.source.value,
            "parent_task_id": str(task.parent_task_id) if task.parent_task_id else None,
            "session_id": task.session_id,
            "submitted_at": task.submitted_at.isoformat(),
            "started_at": task.started_at.isoformat() if task.started_at else None,
            "completed_at": task.completed_at.isoformat() if task.completed_at else None,
            "deadline": task.deadline.isoformat() if task.deadline else None,
            "estimated_duration_seconds": task.estimated_duration_seconds,
            "input_data": task.input_data,
            "result_data": task.result_data,
            "error_message": task.error_message,
        }


# Fixtures


@pytest.fixture
def mock_server():
    """Create mock MCP server."""
    server = MockAbathurTaskQueueServer(Path(":memory:"))
    return server


@pytest.fixture
def mock_task_queue_service():
    """Create mock TaskQueueService."""
    service = AsyncMock()
    return service


@pytest.fixture
def mock_db():
    """Create mock Database."""
    db = AsyncMock()
    return db


@pytest.fixture
def sample_task():
    """Create sample task for testing."""
    task_id = uuid4()
    return Task(
        id=task_id,
        prompt="Test task description",
        agent_type="requirements-gatherer",
        priority=5,
        status=TaskStatus.READY,
        source=TaskSource.HUMAN,
        calculated_priority=7.5,
        dependency_depth=0,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )


# Server Initialization Tests


@pytest.mark.asyncio
async def test_server_initialization(mock_server) -> None:
    """Test that server initializes with correct name."""
    assert mock_server.server.name == "abathur-task-queue"
    assert mock_server.db_path == Path(":memory:")


# task_enqueue Handler Tests


@pytest.mark.asyncio
async def test_task_enqueue_success_minimal(
    mock_server, mock_task_queue_service, sample_task
) -> None:
    """Test successful task enqueue with minimal parameters."""
    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.enqueue_task.return_value = sample_task

    arguments = {
        "description": "Test task",
        "source": "human",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert "error" not in result
    assert result["task_id"] == str(sample_task.id)
    assert result["status"] == "ready"
    assert result["calculated_priority"] == 7.5
    assert result["dependency_depth"] == 0

    # Verify service was called with correct parameters
    mock_task_queue_service.enqueue_task.assert_called_once()
    call_kwargs = mock_task_queue_service.enqueue_task.call_args.kwargs
    assert call_kwargs["description"] == "Test task"
    assert call_kwargs["source"] == TaskSource.HUMAN
    assert call_kwargs["agent_type"] == "requirements-gatherer"  # Default
    assert call_kwargs["base_priority"] == 5  # Default


@pytest.mark.asyncio
async def test_task_enqueue_success_full_parameters(mock_server, mock_task_queue_service) -> None:
    """Test successful task enqueue with all parameters."""
    prereq_id = uuid4()
    parent_id = uuid4()
    task_id = uuid4()

    task = Task(
        id=task_id,
        prompt="Complex task",
        agent_type="planner",
        priority=8,
        status=TaskStatus.BLOCKED,
        source=TaskSource.AGENT_PLANNER,
        calculated_priority=9.2,
        dependency_depth=2,
        parent_task_id=parent_id,
        session_id="session-123",
        deadline=datetime(2025, 12, 31, tzinfo=timezone.utc),
        estimated_duration_seconds=3600,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.enqueue_task.return_value = task

    arguments = {
        "description": "Complex task",
        "source": "agent_planner",
        "agent_type": "planner",
        "base_priority": 8,
        "prerequisites": [str(prereq_id)],
        "parent_task_id": str(parent_id),
        "deadline": "2025-12-31T00:00:00Z",
        "estimated_duration_seconds": 3600,
        "session_id": "session-123",
        "input_data": {"key": "value"},
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert "error" not in result
    assert result["task_id"] == str(task_id)
    assert result["status"] == "blocked"
    assert result["dependency_depth"] == 2

    # Verify all parameters passed to service
    call_kwargs = mock_task_queue_service.enqueue_task.call_args.kwargs
    assert call_kwargs["agent_type"] == "planner"
    assert call_kwargs["base_priority"] == 8
    assert call_kwargs["prerequisites"] == [prereq_id]
    assert call_kwargs["parent_task_id"] == parent_id
    assert call_kwargs["session_id"] == "session-123"


@pytest.mark.asyncio
async def test_task_enqueue_missing_description(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with missing description."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "source": "human",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "description is required" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_missing_source(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with missing source."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "source is required" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_priority_range(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with priority out of range."""
    mock_server.task_queue_service = mock_task_queue_service

    # Test priority > 10
    arguments = {
        "description": "Test task",
        "source": "human",
        "base_priority": 11,
    }

    result = await mock_server._handle_task_enqueue(arguments)
    assert result["error"] == "ValidationError"
    assert "base_priority must be an integer in range [0, 10]" in result["message"]

    # Test priority < 0
    arguments["base_priority"] = -1
    result = await mock_server._handle_task_enqueue(arguments)
    assert result["error"] == "ValidationError"
    assert "[0, 10]" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_agent_type_general_purpose(
    mock_server, mock_task_queue_service
) -> None:
    """Test task enqueue fails with 'general-purpose' agent type."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
        "source": "human",
        "agent_type": "general-purpose",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid agent_type" in result["message"]
    assert "general-purpose" in result["message"]
    assert "Generic agent types are not allowed" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_agent_type_generic(
    mock_server, mock_task_queue_service
) -> None:
    """Test task enqueue fails with generic agent types."""
    mock_server.task_queue_service = mock_task_queue_service

    generic_types = [
        "general",
        "python-backend-developer",
        "implementation-specialist",
        "backend-developer",
        "frontend-developer",
        "developer",
    ]

    for agent_type in generic_types:
        arguments = {
            "description": "Test task",
            "source": "human",
            "agent_type": agent_type,
        }

        result = await mock_server._handle_task_enqueue(arguments)

        assert result["error"] == "ValidationError", f"Failed for {agent_type}"
        assert "Invalid agent_type" in result["message"], f"Failed for {agent_type}"
        assert (
            "Generic agent types are not allowed" in result["message"]
        ), f"Failed for {agent_type}"


@pytest.mark.asyncio
async def test_task_enqueue_valid_specialized_agent_types(
    mock_server, mock_task_queue_service, sample_task
) -> None:
    """Test task enqueue succeeds with valid specialized agent types."""
    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.enqueue_task.return_value = sample_task

    valid_types = [
        "requirements-gatherer",
        "task-planner",
        "technical-requirements-specialist",
        "agent-creator",
        "python-task-queue-domain-model-specialist",
        "python-repository-implementation-specialist",
    ]

    for agent_type in valid_types:
        arguments = {
            "description": "Test task",
            "source": "human",
            "agent_type": agent_type,
        }

        result = await mock_server._handle_task_enqueue(arguments)

        assert "error" not in result, f"Should succeed for {agent_type}: {result}"
        assert result["task_id"] == str(sample_task.id)


@pytest.mark.asyncio
async def test_task_enqueue_invalid_source(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with invalid source."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
        "source": "invalid_source",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid source" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_prerequisite_uuid(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with invalid prerequisite UUID."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
        "source": "human",
        "prerequisites": ["not-a-uuid"],
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid prerequisite UUID" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_parent_uuid(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with invalid parent_task_id UUID."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
        "source": "human",
        "parent_task_id": "not-a-uuid",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid parent_task_id UUID" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_invalid_deadline_format(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails with invalid deadline format."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {
        "description": "Test task",
        "source": "human",
        "deadline": "not-a-date",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid deadline format" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_circular_dependency(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails when circular dependency detected."""
    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.enqueue_task.side_effect = CircularDependencyError(
        "Circular dependency detected: A -> B -> C -> A"
    )

    arguments = {
        "description": "Task with circular dep",
        "source": "human",
        "prerequisites": [str(uuid4())],
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "CircularDependencyError"
    assert "Circular dependency detected" in result["message"]


@pytest.mark.asyncio
async def test_task_enqueue_prerequisite_not_found(mock_server, mock_task_queue_service) -> None:
    """Test task enqueue fails when prerequisite doesn't exist."""
    mock_server.task_queue_service = mock_task_queue_service
    prereq_id = uuid4()
    mock_task_queue_service.enqueue_task.side_effect = ValueError(
        f"Prerequisites not found: {{{prereq_id}}}"
    )

    arguments = {
        "description": "Task with missing prereq",
        "source": "human",
        "prerequisites": [str(prereq_id)],
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "ValidationError"
    assert "Prerequisites not found" in result["message"]


# task_get Handler Tests


@pytest.mark.asyncio
async def test_task_get_success(mock_server, mock_db, sample_task) -> None:
    """Test successful task retrieval by ID."""
    mock_server.db = mock_db
    mock_db.get_task.return_value = sample_task

    arguments = {"task_id": str(sample_task.id)}

    result = await mock_server._handle_task_get(arguments)

    assert "error" not in result
    assert result["id"] == str(sample_task.id)
    assert result["prompt"] == "Test task description"
    assert result["status"] == "ready"
    assert result["calculated_priority"] == 7.5


@pytest.mark.asyncio
async def test_task_get_not_found(mock_server, mock_db) -> None:
    """Test task_get returns error when task not found."""
    mock_server.db = mock_db
    mock_db.get_task.return_value = None

    task_id = uuid4()
    arguments = {"task_id": str(task_id)}

    result = await mock_server._handle_task_get(arguments)

    assert result["error"] == "NotFoundError"
    assert f"Task {task_id} not found" in result["message"]


@pytest.mark.asyncio
async def test_task_get_missing_task_id(mock_server, mock_db) -> None:
    """Test task_get fails with missing task_id."""
    mock_server.db = mock_db

    arguments = {}

    result = await mock_server._handle_task_get(arguments)

    assert result["error"] == "ValidationError"
    assert "task_id is required" in result["message"]


@pytest.mark.asyncio
async def test_task_get_invalid_uuid(mock_server, mock_db) -> None:
    """Test task_get fails with invalid UUID format."""
    mock_server.db = mock_db

    arguments = {"task_id": "not-a-uuid"}

    result = await mock_server._handle_task_get(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid UUID format" in result["message"]


# task_list Handler Tests


@pytest.mark.asyncio
async def test_task_list_success_no_filters(mock_server, mock_db) -> None:
    """Test successful task list with no filters."""
    task1 = Task(
        id=uuid4(),
        prompt="Task 1",
        status=TaskStatus.READY,
        source=TaskSource.HUMAN,
        calculated_priority=8.0,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )
    task2 = Task(
        id=uuid4(),
        prompt="Task 2",
        status=TaskStatus.COMPLETED,
        source=TaskSource.AGENT_PLANNER,
        calculated_priority=6.0,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    mock_server.db = mock_db
    mock_db.list_tasks.return_value = [task1, task2]

    arguments = {}

    result = await mock_server._handle_task_list(arguments)

    assert "error" not in result
    assert len(result["tasks"]) == 2
    assert result["tasks"][0]["id"] == str(task1.id)
    assert result["tasks"][1]["id"] == str(task2.id)


@pytest.mark.asyncio
async def test_task_list_with_status_filter(mock_server, mock_db) -> None:
    """Test task list with status filter."""
    ready_task = Task(
        id=uuid4(),
        prompt="Ready task",
        status=TaskStatus.READY,
        source=TaskSource.HUMAN,
        calculated_priority=7.0,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    mock_server.db = mock_db
    mock_db.list_tasks.return_value = [ready_task]

    arguments = {"status": "ready"}

    result = await mock_server._handle_task_list(arguments)

    assert "error" not in result
    assert len(result["tasks"]) == 1
    assert result["tasks"][0]["status"] == "ready"


@pytest.mark.asyncio
async def test_task_list_with_limit(mock_server, mock_db) -> None:
    """Test task list with custom limit."""
    mock_server.db = mock_db
    mock_db.list_tasks.return_value = []

    arguments = {"limit": 100}

    result = await mock_server._handle_task_list(arguments)

    assert "error" not in result
    mock_db.list_tasks.assert_called_once()
    assert mock_db.list_tasks.call_args.kwargs["limit"] == 100


@pytest.mark.asyncio
async def test_task_list_invalid_status(mock_server, mock_db) -> None:
    """Test task list fails with invalid status."""
    mock_server.db = mock_db

    arguments = {"status": "invalid_status"}

    result = await mock_server._handle_task_list(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid status" in result["message"]


@pytest.mark.asyncio
async def test_task_list_invalid_limit(mock_server, mock_db) -> None:
    """Test task list fails with invalid limit."""
    mock_server.db = mock_db

    # Test negative limit
    arguments = {"limit": -1}
    result = await mock_server._handle_task_list(arguments)
    assert result["error"] == "ValidationError"
    assert "limit must be positive integer" in result["message"]

    # Test limit exceeding maximum
    arguments = {"limit": 501}
    result = await mock_server._handle_task_list(arguments)
    assert result["error"] == "ValidationError"
    assert "limit cannot exceed 500" in result["message"]


@pytest.mark.asyncio
async def test_task_list_empty_result(mock_server, mock_db) -> None:
    """Test task list returns empty array when no matches."""
    mock_server.db = mock_db
    mock_db.list_tasks.return_value = []

    arguments = {"status": "running"}

    result = await mock_server._handle_task_list(arguments)

    assert "error" not in result
    assert result["tasks"] == []


# task_queue_status Handler Tests


@pytest.mark.asyncio
async def test_queue_status_success(mock_server, mock_task_queue_service) -> None:
    """Test successful queue status retrieval."""
    mock_server.task_queue_service = mock_task_queue_service

    status_data = {
        "total_tasks": 100,
        "pending": 5,
        "blocked": 10,
        "ready": 8,
        "running": 3,
        "completed": 70,
        "failed": 3,
        "cancelled": 1,
        "avg_priority": 6.5,
        "max_depth": 4,
        "oldest_pending": datetime(2025, 10, 10, 10, 0, 0, tzinfo=timezone.utc),
        "newest_task": datetime(2025, 10, 11, 15, 30, 0, tzinfo=timezone.utc),
    }
    mock_task_queue_service.get_queue_status.return_value = status_data

    arguments = {}

    result = await mock_server._handle_task_queue_status(arguments)

    assert "error" not in result
    assert result["total_tasks"] == 100
    assert result["ready"] == 8
    assert result["avg_priority"] == 6.5
    assert result["max_depth"] == 4
    assert "2025-10-10T10:00:00" in result["oldest_pending"]
    assert "2025-10-11T15:30:00" in result["newest_task"]


@pytest.mark.asyncio
async def test_queue_status_empty_queue(mock_server, mock_task_queue_service) -> None:
    """Test queue status with empty queue."""
    mock_server.task_queue_service = mock_task_queue_service

    status_data = {
        "total_tasks": 0,
        "pending": 0,
        "blocked": 0,
        "ready": 0,
        "running": 0,
        "completed": 0,
        "failed": 0,
        "cancelled": 0,
        "avg_priority": 0.0,
        "max_depth": 0,
        "oldest_pending": None,
        "newest_task": None,
    }
    mock_task_queue_service.get_queue_status.return_value = status_data

    arguments = {}

    result = await mock_server._handle_task_queue_status(arguments)

    assert "error" not in result
    assert result["total_tasks"] == 0
    assert result["oldest_pending"] is None
    assert result["newest_task"] is None


# task_cancel Handler Tests


@pytest.mark.asyncio
async def test_task_cancel_success_no_cascade(mock_server, mock_task_queue_service) -> None:
    """Test successful task cancellation with no dependents."""
    mock_server.task_queue_service = mock_task_queue_service

    task_id = uuid4()
    mock_task_queue_service.cancel_task.return_value = [task_id]

    arguments = {"task_id": str(task_id)}

    result = await mock_server._handle_task_cancel(arguments)

    assert "error" not in result
    assert result["cancelled_task_id"] == str(task_id)
    assert result["cascaded_task_ids"] == []
    assert result["total_cancelled"] == 1


@pytest.mark.asyncio
async def test_task_cancel_success_with_cascade(mock_server, mock_task_queue_service) -> None:
    """Test successful task cancellation with dependent tasks."""
    mock_server.task_queue_service = mock_task_queue_service

    task_id = uuid4()
    dependent1 = uuid4()
    dependent2 = uuid4()

    mock_task_queue_service.cancel_task.return_value = [task_id, dependent1, dependent2]

    arguments = {"task_id": str(task_id)}

    result = await mock_server._handle_task_cancel(arguments)

    assert "error" not in result
    assert result["cancelled_task_id"] == str(task_id)
    assert len(result["cascaded_task_ids"]) == 2
    assert str(dependent1) in result["cascaded_task_ids"]
    assert str(dependent2) in result["cascaded_task_ids"]
    assert result["total_cancelled"] == 3


@pytest.mark.asyncio
async def test_task_cancel_not_found(mock_server, mock_task_queue_service) -> None:
    """Test task cancel fails when task not found."""
    mock_server.task_queue_service = mock_task_queue_service

    task_id = uuid4()
    mock_task_queue_service.cancel_task.side_effect = TaskNotFoundError(f"Task {task_id} not found")

    arguments = {"task_id": str(task_id)}

    result = await mock_server._handle_task_cancel(arguments)

    assert result["error"] == "NotFoundError"
    assert "not found" in result["message"]


@pytest.mark.asyncio
async def test_task_cancel_missing_task_id(mock_server, mock_task_queue_service) -> None:
    """Test task cancel fails with missing task_id."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {}

    result = await mock_server._handle_task_cancel(arguments)

    assert result["error"] == "ValidationError"
    assert "task_id is required" in result["message"]


@pytest.mark.asyncio
async def test_task_cancel_invalid_uuid(mock_server, mock_task_queue_service) -> None:
    """Test task cancel fails with invalid UUID."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {"task_id": "not-a-uuid"}

    result = await mock_server._handle_task_cancel(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid UUID format" in result["message"]


# task_execution_plan Handler Tests


@pytest.mark.asyncio
async def test_execution_plan_success(mock_server, mock_task_queue_service) -> None:
    """Test successful execution plan calculation."""
    mock_server.task_queue_service = mock_task_queue_service

    task_a = uuid4()
    task_b = uuid4()
    task_c = uuid4()
    task_d = uuid4()

    # Execution plan: A → (B, C) → D
    batches = [[task_a], [task_b, task_c], [task_d]]
    mock_task_queue_service.get_task_execution_plan.return_value = batches

    arguments = {"task_ids": [str(task_a), str(task_b), str(task_c), str(task_d)]}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert "error" not in result
    assert len(result["batches"]) == 3
    assert len(result["batches"][0]) == 1  # Batch 0: [A]
    assert len(result["batches"][1]) == 2  # Batch 1: [B, C]
    assert len(result["batches"][2]) == 1  # Batch 2: [D]
    assert result["total_batches"] == 3
    assert result["max_parallelism"] == 2  # Max tasks in parallel


@pytest.mark.asyncio
async def test_execution_plan_empty_task_ids(mock_server, mock_task_queue_service) -> None:
    """Test execution plan with empty task_ids."""
    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.get_task_execution_plan.return_value = []

    arguments = {"task_ids": []}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert "error" not in result
    assert result["batches"] == []
    assert result["total_batches"] == 0
    assert result["max_parallelism"] == 0


@pytest.mark.asyncio
async def test_execution_plan_circular_dependency(mock_server, mock_task_queue_service) -> None:
    """Test execution plan fails with circular dependency."""
    mock_server.task_queue_service = mock_task_queue_service

    task_a = uuid4()
    task_b = uuid4()

    mock_task_queue_service.get_task_execution_plan.side_effect = CircularDependencyError(
        f"Circular dependency detected: {task_a} -> {task_b} -> {task_a}"
    )

    arguments = {"task_ids": [str(task_a), str(task_b)]}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert result["error"] == "CircularDependencyError"
    assert "Circular dependency detected" in result["message"]


@pytest.mark.asyncio
async def test_execution_plan_missing_task_ids(mock_server, mock_task_queue_service) -> None:
    """Test execution plan fails with missing task_ids."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert result["error"] == "ValidationError"
    assert "task_ids is required" in result["message"]


@pytest.mark.asyncio
async def test_execution_plan_invalid_task_ids_type(mock_server, mock_task_queue_service) -> None:
    """Test execution plan fails when task_ids is not an array."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {"task_ids": "not-an-array"}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert result["error"] == "ValidationError"
    assert "task_ids must be an array" in result["message"]


@pytest.mark.asyncio
async def test_execution_plan_invalid_uuid_in_array(mock_server, mock_task_queue_service) -> None:
    """Test execution plan fails with invalid UUID in task_ids."""
    mock_server.task_queue_service = mock_task_queue_service

    arguments = {"task_ids": [str(uuid4()), "not-a-uuid"]}

    result = await mock_server._handle_task_execution_plan(arguments)

    assert result["error"] == "ValidationError"
    assert "Invalid UUID in task_ids" in result["message"]


# Edge Cases and Error Handling Tests


@pytest.mark.asyncio
async def test_task_serialize_with_all_fields(mock_server) -> None:
    """Test task serialization includes all fields."""
    parent_id = uuid4()
    task = Task(
        id=uuid4(),
        prompt="Full task",
        agent_type="planner",
        priority=8,
        status=TaskStatus.COMPLETED,
        source=TaskSource.AGENT_PLANNER,
        calculated_priority=9.5,
        dependency_depth=3,
        parent_task_id=parent_id,
        session_id="session-xyz",
        input_data={"input": "data"},
        result_data={"result": "data"},
        error_message=None,
        deadline=datetime(2025, 12, 31, tzinfo=timezone.utc),
        estimated_duration_seconds=1800,
        submitted_at=datetime(2025, 10, 11, 10, 0, 0, tzinfo=timezone.utc),
        started_at=datetime(2025, 10, 11, 10, 5, 0, tzinfo=timezone.utc),
        completed_at=datetime(2025, 10, 11, 10, 35, 0, tzinfo=timezone.utc),
        last_updated_at=datetime(2025, 10, 11, 10, 35, 0, tzinfo=timezone.utc),
    )

    serialized = mock_server._serialize_task(task)

    assert serialized["id"] == str(task.id)
    assert serialized["prompt"] == "Full task"
    assert serialized["agent_type"] == "planner"
    assert serialized["priority"] == 8
    assert serialized["status"] == "completed"
    assert serialized["source"] == "agent_planner"
    assert serialized["calculated_priority"] == 9.5
    assert serialized["dependency_depth"] == 3
    assert serialized["parent_task_id"] == str(parent_id)
    assert serialized["session_id"] == "session-xyz"
    assert serialized["input_data"] == {"input": "data"}
    assert serialized["result_data"] == {"result": "data"}
    assert serialized["error_message"] is None
    assert "2025-12-31" in serialized["deadline"]
    assert serialized["estimated_duration_seconds"] == 1800
    assert "2025-10-11T10:00:00" in serialized["submitted_at"]
    assert "2025-10-11T10:05:00" in serialized["started_at"]
    assert "2025-10-11T10:35:00" in serialized["completed_at"]


@pytest.mark.asyncio
async def test_task_serialize_with_minimal_fields(mock_server) -> None:
    """Test task serialization with only required fields."""
    task = Task(
        id=uuid4(),
        prompt="Minimal task",
        status=TaskStatus.PENDING,
        source=TaskSource.HUMAN,
        submitted_at=datetime.now(timezone.utc),
        last_updated_at=datetime.now(timezone.utc),
    )

    serialized = mock_server._serialize_task(task)

    assert serialized["id"] == str(task.id)
    assert serialized["prompt"] == "Minimal task"
    assert serialized["parent_task_id"] is None
    assert serialized["session_id"] is None
    assert serialized["started_at"] is None
    assert serialized["completed_at"] is None
    assert serialized["deadline"] is None


@pytest.mark.asyncio
async def test_handler_exception_handling(mock_server, mock_task_queue_service) -> None:
    """Test that unexpected exceptions are caught and formatted."""
    mock_server.task_queue_service = mock_task_queue_service
    mock_task_queue_service.enqueue_task.side_effect = Exception("Unexpected database error")

    arguments = {
        "description": "Test task",
        "source": "human",
    }

    result = await mock_server._handle_task_enqueue(arguments)

    assert result["error"] == "InternalError"
    assert "Unexpected database error" in result["message"]


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
