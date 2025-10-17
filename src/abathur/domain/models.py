"""Core domain models for Abathur."""

from datetime import datetime, timezone
from enum import Enum
from typing import Any
from uuid import UUID, uuid4

from pydantic import BaseModel, ConfigDict, Field, field_validator


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
    """Represents a unit of work in the task queue.

    Attributes:
        id: Unique task identifier
        summary: Short, human-readable task summary for display (optional, max 140 chars, auto-generated if not provided)
        prompt: The actual instruction/task to execute
        feature_branch: Feature branch that task changes get merged into (optional)
        task_branch: Individual task branch for isolated work (optional)
        worktree_path: Git worktree directory path for isolated execution (optional)
    """

    id: UUID = Field(default_factory=uuid4)
    summary: str | None = Field(
        default=None,
        description="Short, human-readable task summary (max 140 chars after stripping, auto-generated from description if not provided)",
    )
    prompt: str  # The actual instruction/task to execute (description in MCP API)
    agent_type: str = (
        "requirements-gatherer"  # Agent definition to use (defaults to requirements-gatherer)
    )
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

    # NEW: Feature branch tracking
    feature_branch: str | None = None  # Feature branch that task changes get merged into

    # NEW: Task branch tracking
    task_branch: str | None = (
        None  # Individual task branch for isolated work (merges into feature_branch)
    )

    # NEW: Worktree path tracking
    worktree_path: str | None = None  # Git worktree directory path for isolated execution

    @field_validator("summary")
    @classmethod
    def validate_summary(cls, v: str | None) -> str | None:
        """Validate summary field: strip whitespace, truncate if too long.

        Auto-corrects instead of raising errors for better UX:
        - Strips whitespace
        - Returns None if empty (triggers auto-generation)
        - Truncates to 140 chars if too long
        """
        if v is None:
            return None
        # Strip whitespace first
        v = v.strip()
        # Reject empty string after stripping (will trigger auto-generation)
        if not v:
            return None
        # Truncate to max length (auto-correction vs rejection)
        if len(v) > 140:
            return v[:140]
        return v

    model_config = ConfigDict(
        # Note: Use model_dump(mode='json') for proper JSON serialization
        # This automatically converts UUID→str, datetime→ISO string
        # Enums serialize to their values by default in mode='json'
    )


class TaskDependency(BaseModel):
    """Represents a dependency relationship between tasks."""

    id: UUID = Field(default_factory=uuid4)
    dependent_task_id: UUID  # Task that depends
    prerequisite_task_id: UUID  # Task that must complete first
    dependency_type: DependencyType
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    resolved_at: datetime | None = None  # When prerequisite completed

    model_config = ConfigDict()


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

    model_config = ConfigDict()


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

    model_config = ConfigDict()


class LoopState(BaseModel):
    """State of iterative loop execution."""

    task_id: UUID
    iteration_count: int = 0
    max_iterations: int = 10
    converged: bool = False
    history: list[dict[str, Any]] = Field(default_factory=list)
    checkpoint_data: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict()
