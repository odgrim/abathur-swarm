"""Prune operation models and filters for task cleanup.

This module contains data models used for pruning (deleting) tasks from the database,
including filtering criteria, result statistics, and tree traversal structures.
"""

from dataclasses import dataclass, field
from datetime import datetime
from uuid import UUID

from pydantic import BaseModel, Field, field_validator, model_validator

from abathur.domain.models import TaskStatus


@dataclass
class TreeDiscoveryNode:
    """Runtime data structure representing a node in task tree during recursive operations.

    Named TreeDiscoveryNode to avoid conflicts with TUI's TreeNode Pydantic model.
    """
    id: UUID
    parent_id: UUID | None
    status: TaskStatus
    depth: int
    children_ids: list[UUID] = field(default_factory=list)

    @classmethod
    def from_row(cls, row: dict) -> "TreeDiscoveryNode":
        """Construct TreeDiscoveryNode from database query result row."""
        return cls(
            id=UUID(row["id"]),
            parent_id=UUID(row["parent_id"]) if row["parent_id"] else None,
            status=TaskStatus(row["status"]),
            depth=row["depth"],
            children_ids=[]  # Populated by _build_tree_structure()
        )

    def is_leaf(self) -> bool:
        """Check if this node is a leaf (no children)."""
        return len(self.children_ids) == 0

    def matches_status(self, allowed: list[TaskStatus]) -> bool:
        """Check if this node's status matches any of the allowed statuses."""
        return self.status in allowed


class PruneFilters(BaseModel):
    """Filtering criteria for pruning operation.

    Supports three selection strategies:
    1. By IDs: task_ids list (direct selection)
    2. By status: statuses list (all tasks with given statuses)
    3. By time: older_than_days or before_date (time-based filtering)

    Can combine strategies (e.g., task_ids + statuses, or time + statuses).
    """

    task_ids: list[UUID] | None = Field(
        default=None,
        description="Specific task IDs to delete (direct selection)",
    )

    older_than_days: int | None = Field(
        default=None,
        ge=1,
        description="Delete tasks older than N days (completed_at/submitted_at)",
    )

    before_date: datetime | None = Field(
        default=None, description="Delete tasks completed/submitted before this date"
    )

    statuses: list[TaskStatus] | None = Field(
        default=None,
        description="Task statuses to prune (None = all pruneable statuses)",
    )

    limit: int | None = Field(
        default=None, ge=1, description="Maximum tasks to delete in one operation"
    )

    dry_run: bool = Field(default=False, description="Preview mode without deletion")

    vacuum_mode: str = Field(
        default="conditional",
        description="VACUUM strategy: 'always', 'conditional', or 'never'",
    )

    recursive: bool = Field(
        default=False,
        description="Enable recursive tree deletion with status checking"
    )

    @model_validator(mode="after")
    def validate_filters(self) -> "PruneFilters":
        """Ensure at least one selection criterion is specified."""
        has_ids = self.task_ids is not None and len(self.task_ids) > 0
        has_time = self.older_than_days is not None or self.before_date is not None
        has_status = self.statuses is not None and len(self.statuses) > 0

        if not (has_ids or has_time or has_status):
            raise ValueError(
                "At least one selection criterion must be specified: "
                "'task_ids', 'older_than_days', 'before_date', or 'statuses'"
            )

        # Set default statuses if using time-based selection without explicit statuses
        if has_time and self.statuses is None:
            self.statuses = [
                TaskStatus.COMPLETED,
                TaskStatus.FAILED,
                TaskStatus.CANCELLED,
            ]

        return self

    @field_validator("statuses")
    @classmethod
    def validate_statuses(cls, v: list[TaskStatus]) -> list[TaskStatus]:
        """Ensure only pruneable statuses are specified."""
        forbidden = {
            TaskStatus.PENDING,
            TaskStatus.BLOCKED,
            TaskStatus.READY,
            TaskStatus.RUNNING,
        }
        invalid = set(v) & forbidden
        if invalid:
            raise ValueError(
                f"Cannot prune tasks with statuses: {invalid}. "
                f"Only COMPLETED, FAILED, or CANCELLED tasks can be pruned."
            )
        return v

    @field_validator("vacuum_mode")
    @classmethod
    def validate_vacuum_mode(cls, v: str) -> str:
        """Validate vacuum_mode is one of allowed values."""
        allowed = {"always", "conditional", "never"}
        if v not in allowed:
            raise ValueError(f"vacuum_mode must be one of {allowed}, got '{v}'")
        return v

    def build_where_clause(self) -> tuple[str, list[str]]:
        """Build SQL WHERE clause and parameters for task filtering.

        Handles multiple selection strategies:
        - ID-based: WHERE id IN (...)
        - Time-based: WHERE completed_at/submitted_at < ...
        - Status-based: WHERE status IN (...)

        Returns:
            Tuple of (where_clause_sql, parameters) where:
            - where_clause_sql: SQL WHERE condition without 'WHERE' keyword
            - parameters: List of parameter values for SQL placeholders

        Used by both CLI preview queries and database prune_tasks() execution
        to ensure consistent filtering logic.
        """
        where_clauses = []
        params = []

        # ID filter (if specified, most specific)
        if self.task_ids is not None and len(self.task_ids) > 0:
            id_placeholders = ",".join("?" * len(self.task_ids))
            where_clauses.append(f"id IN ({id_placeholders})")
            params.extend([str(task_id) for task_id in self.task_ids])

        # Time filter (optional)
        if self.older_than_days is not None:
            where_clauses.append(
                "(completed_at < date('now', ?) OR "
                "(completed_at IS NULL AND submitted_at < date('now', ?)))"
            )
            days_param = f"-{self.older_than_days} days"
            params.extend([days_param, days_param])
        elif self.before_date is not None:
            where_clauses.append(
                "(completed_at < ? OR (completed_at IS NULL AND submitted_at < ?))"
            )
            before_iso = self.before_date.isoformat()
            params.extend([before_iso, before_iso])

        # Status filter (optional)
        if self.statuses is not None and len(self.statuses) > 0:
            status_placeholders = ",".join("?" * len(self.statuses))
            where_clauses.append(f"status IN ({status_placeholders})")
            params.extend([status.value for status in self.statuses])

        # Default to always match if no filters (shouldn't happen due to validation)
        if not where_clauses:
            where_clauses.append("1=1")

        where_sql = " AND ".join(where_clauses)
        return (where_sql, params)


class PruneResult(BaseModel):
    """Statistics from prune operation.

    Contains comprehensive metrics about the pruning operation including
    the number of tasks and dependencies deleted, space reclaimed, and
    a breakdown of deleted tasks by status.
    """

    deleted_tasks: int = Field(ge=0, description="Number of tasks deleted")

    deleted_dependencies: int = Field(
        ge=0, description="Number of task_dependencies records deleted"
    )

    reclaimed_bytes: int | None = Field(
        default=None, ge=0, description="Bytes reclaimed by VACUUM (optional)"
    )

    dry_run: bool = Field(description="Whether this was a dry run")

    breakdown_by_status: dict[TaskStatus, int] = Field(
        default_factory=dict, description="Count of deleted tasks by status"
    )

    vacuum_auto_skipped: bool = Field(
        default=False,
        description="Whether VACUUM was automatically skipped due to large task count (>10,000 tasks)"
    )

    tree_depth: int | None = Field(
        default=None,
        description="Maximum depth of deleted tree (None if not recursive)"
    )

    deleted_by_depth: dict[int, int] | None = Field(
        default=None,
        description="Count of tasks deleted at each depth level {0: 5, 1: 12, 2: 8}"
    )

    trees_affected: int | None = Field(
        default=None,
        description="Number of tree roots processed (None if not recursive)"
    )

    partial_trees_preserved: int | None = Field(
        default=None,
        description="Number of trees skipped due to non-matching children"
    )

    @field_validator("breakdown_by_status")
    @classmethod
    def validate_breakdown_values(cls, v: dict[TaskStatus, int]) -> dict[TaskStatus, int]:
        """Ensure all breakdown values are non-negative."""
        for status, count in v.items():
            if count < 0:
                raise ValueError(
                    f"Breakdown count for status {status} must be non-negative, got {count}"
                )
        return v


class RecursivePruneResult(PruneResult):
    """Enhanced result with tree-specific statistics.

    Extends PruneResult with additional metrics specific to recursive
    tree deletion operations, including depth tracking and tree-level
    deletion statistics.
    """

    tree_depth: int = Field(
        ge=0,
        description="Maximum depth of deleted tree"
    )

    deleted_by_depth: dict[int, int] = Field(
        default_factory=dict,
        description="Count of tasks deleted at each depth level"
    )

    trees_deleted: int = Field(
        default=0,
        ge=0,
        description="Number of complete task trees deleted"
    )

    partial_trees: int = Field(
        default=0,
        ge=0,
        description="Number of trees partially deleted"
    )


class TreeNode(BaseModel):
    """Runtime data structure for task tree traversal (not persisted)."""

    id: UUID = Field(description="Task identifier")
    parent_id: UUID | None = Field(default=None, description="Parent task reference")
    status: TaskStatus = Field(description="Current task status")
    depth: int = Field(ge=0, le=100, description="Depth in tree (0 for root)")
    children_ids: list[UUID] = Field(default_factory=list, description="Direct children")

    def is_leaf(self) -> bool:
        """Check if node is a leaf (no children)."""
        return len(self.children_ids) == 0

    def add_child(self, child_id: UUID) -> None:
        """Add child ID to children list."""
        self.children_ids.append(child_id)
