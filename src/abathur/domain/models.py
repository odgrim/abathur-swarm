"""Core domain models for Abathur."""

from datetime import datetime, timezone
from enum import Enum
from typing import Any
from uuid import UUID, uuid4

from pydantic import BaseModel, ConfigDict, Field


class TaskStatus(str, Enum):
    """Task lifecycle states."""

    PENDING = "pending"  # Submitted, dependencies not yet checked
    BLOCKED = "blocked"  # Waiting for dependencies
    READY = "ready"  # Dependencies met, ready for execution
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class TaskSource(str, Enum):
    """Origin of task submission."""

    HUMAN = "human"
    AGENT_REQUIREMENTS = "agent_requirements"
    AGENT_PLANNER = "agent_planner"
    AGENT_IMPLEMENTATION = "agent_implementation"


class DependencyType(str, Enum):
    """Type of dependency relationship."""

    SEQUENTIAL = "sequential"  # B depends on A completing
    PARALLEL = "parallel"  # C depends on A AND B both completing (AND logic)


class Task(BaseModel):
    """Represents a unit of work in the task queue."""

    id: UUID = Field(default_factory=uuid4)
    prompt: str  # The actual instruction/task to execute
    agent_type: str = "general"  # Agent definition to use (defaults to general)
    priority: int = Field(default=5, ge=0, le=10)
    status: TaskStatus = Field(default=TaskStatus.PENDING)
    input_data: dict[str, Any] = Field(default_factory=dict)
    result_data: dict[str, Any] | None = None
    error_message: str | None = None
    retry_count: int = Field(default=0, ge=0)
    max_retries: int = Field(default=3, ge=0)
    max_execution_timeout_seconds: int = Field(default=3600, ge=60)  # Default 1 hour
    submitted_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    started_at: datetime | None = None
    completed_at: datetime | None = None
    last_updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    created_by: str | None = None
    parent_task_id: UUID | None = None
    dependencies: list[UUID] = Field(default_factory=list)
    session_id: str | None = None  # Link to session for memory context

    # NEW: Source tracking
    source: TaskSource = Field(default=TaskSource.HUMAN)

    # NEW: Dependency type
    dependency_type: DependencyType = Field(default=DependencyType.SEQUENTIAL)

    # NEW: Priority calculation fields
    calculated_priority: float = Field(default=5.0)
    deadline: datetime | None = None
    estimated_duration_seconds: int | None = None
    dependency_depth: int = Field(default=0)

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
            datetime: lambda v: v.isoformat(),
        }
    )


class TaskDependency(BaseModel):
    """Represents a dependency relationship between tasks."""

    id: UUID = Field(default_factory=uuid4)
    dependent_task_id: UUID  # Task that depends
    prerequisite_task_id: UUID  # Task that must complete first
    dependency_type: DependencyType
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    resolved_at: datetime | None = None  # When prerequisite completed

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
            datetime: lambda v: v.isoformat(),
        }
    )


class AgentState(str, Enum):
    """Agent lifecycle states."""

    SPAWNING = "spawning"
    IDLE = "idle"
    BUSY = "busy"
    TERMINATING = "terminating"
    TERMINATED = "terminated"


class Agent(BaseModel):
    """Represents a Claude agent instance."""

    id: UUID = Field(default_factory=uuid4)
    name: str
    specialization: str
    task_id: UUID
    state: AgentState = Field(default=AgentState.SPAWNING)
    model: str = "claude-sonnet-4-5-20250929"
    spawned_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    terminated_at: datetime | None = None
    resource_usage: dict[str, Any] = Field(default_factory=dict)
    session_id: str | None = None  # Link to session for memory context

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
            datetime: lambda v: v.isoformat(),
        }
    )


class ExecutionContext(BaseModel):
    """Runtime context for agent execution."""

    task: Task
    config: dict[str, Any]
    shared_state: dict[str, Any] = Field(default_factory=dict)


class Result(BaseModel):
    """Output from agent execution."""

    task_id: UUID
    agent_id: UUID
    success: bool
    data: dict[str, Any] | None = None
    error: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)
    token_usage: dict[str, int] | None = None
    execution_time_seconds: float | None = None

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
        }
    )


class LoopState(BaseModel):
    """State of iterative loop execution."""

    task_id: UUID
    iteration_count: int = 0
    max_iterations: int = 10
    converged: bool = False
    history: list[dict[str, Any]] = Field(default_factory=list)
    checkpoint_data: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(
        json_encoders={
            UUID: str,
        }
    )
