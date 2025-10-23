"""TUI-specific models for Abathur task visualization.

This module contains Pydantic models used exclusively by the TUI layer,
separate from domain models in abathur.domain.models.
"""

from enum import Enum
from uuid import UUID
from pydantic import BaseModel, Field

from abathur.domain.models import Task, TaskStatus, TaskSource


class ViewMode(str, Enum):
    """Task visualization modes for TUI."""

    TREE = "tree"  # Hierarchical tree view
    FLAT = "flat"  # Flat list view
    DAG = "dag"  # Full DAG visualization (future)


class TreeNode(BaseModel):
    """Node in rendered tree structure.

    Represents a single task positioned within the hierarchical layout.
    Each node tracks its position, level, and children for rendering.

    Attributes:
        task_id: UUID of the task this node represents
        task: Full Task object for rendering
        children: List of child task IDs (tasks with this task as parent)
        level: Depth in tree hierarchy (same as dependency_depth)
        is_expanded: Whether children should be visible
        position: Order within the same level (used for sorting)
    """

    task_id: UUID
    task: Task
    children: list[UUID] = Field(default_factory=list)
    level: int = Field(ge=0, description="Depth in tree (same as dependency_depth)")
    is_expanded: bool = True
    position: int = Field(ge=0, description="Order within level")


class TreeLayout(BaseModel):
    """Complete tree layout structure for rendering.

    Encapsulates the entire hierarchical structure of tasks
    ready for terminal rendering.

    Attributes:
        nodes: Dict mapping task_id to TreeNode objects
        root_nodes: List of task IDs with no parent (top-level tasks)
        max_depth: Maximum dependency depth in the tree
        total_nodes: Total number of nodes in the tree
    """

    nodes: dict[UUID, TreeNode] = Field(default_factory=dict)
    root_nodes: list[UUID] = Field(
        default_factory=list, description="Tasks with no parent_task_id"
    )
    max_depth: int = 0
    total_nodes: int = 0

    def get_visible_nodes(self, expanded_nodes: set[UUID] | None = None) -> list[TreeNode]:
        """Returns only visible nodes based on expand/collapse state.

        Traverses the tree from root nodes, including only nodes
        whose parents are expanded.

        Args:
            expanded_nodes: Set of node IDs that are expanded.
                          If None, uses each node's is_expanded property.
                          If provided (even if empty), it's authoritative -
                          nodes not in this set are collapsed.

        Returns:
            List of TreeNode objects that should be rendered
        """
        visible = []

        def traverse(node_id: UUID, is_visible: bool):
            """Recursively traverse tree and collect visible nodes."""
            if node_id not in self.nodes:
                return

            node = self.nodes[node_id]
            if is_visible:
                visible.append(node)

            # Children are visible if this node is expanded
            # If expanded_nodes is None, use is_expanded property
            # Otherwise, expanded_nodes is authoritative (even if empty)
            if expanded_nodes is None:
                child_visible = is_visible and node.is_expanded
            else:
                child_visible = is_visible and (node_id in expanded_nodes)

            for child_id in node.children:
                traverse(child_id, child_visible)

        # Start traversal from root nodes
        for root_id in self.root_nodes:
            traverse(root_id, True)

        return visible

    def find_node_path(self, task_id: UUID) -> list[UUID]:
        """Returns path from root to specified node.

        Useful for expanding all ancestors when navigating to a specific task.

        Args:
            task_id: Target task ID to find path to

        Returns:
            List of task IDs from root to target (inclusive)
            Returns empty list if task_id not found
        """
        # Build parent map for reverse traversal
        parent_map: dict[UUID, UUID] = {}
        for node_id, node in self.nodes.items():
            for child_id in node.children:
                parent_map[child_id] = node_id

        # Trace path from task to root
        path = []
        current = task_id

        # Guard against cycles and missing nodes
        visited = set()
        while current and current not in visited:
            if current not in self.nodes:
                # Task not found in tree
                return []

            path.insert(0, current)
            visited.add(current)
            current = parent_map.get(current)

        return path


class FilterState(BaseModel):
    """Filter state for task list filtering with multi-criteria AND logic.

    All filter criteria are ANDed together. A task must match ALL active
    filters to be included in filtered results.

    Attributes:
        status_filter: Set of task statuses to include (OR within set)
        agent_type_filter: Filter by agent type (case-insensitive substring match)
        feature_branch_filter: Filter by feature branch name (case-insensitive substring match)
        text_search: Search text across task description and summary (case-insensitive)
        source_filter: Filter by task source (exact match)
    """

    status_filter: set[TaskStatus] | None = Field(
        default=None, description="Set of task statuses to include (OR within set)"
    )
    agent_type_filter: str | None = Field(
        default=None,
        max_length=200,
        description="Filter by agent type (case-insensitive substring match)",
    )
    feature_branch_filter: str | None = Field(
        default=None,
        max_length=200,
        description="Filter by feature branch name (case-insensitive substring match)",
    )
    text_search: str | None = Field(
        default=None,
        max_length=500,
        description="Search text across task description and summary (case-insensitive)",
    )
    source_filter: TaskSource | None = Field(
        default=None, description="Filter by task source (exact match)"
    )

    def is_active(self) -> bool:
        """Returns True if any filter criteria is set.

        Used to determine if filtering UI should show active state.
        """
        return bool(
            self.status_filter
            or self.agent_type_filter
            or self.feature_branch_filter
            or self.text_search
            or self.source_filter
        )

    def matches(self, task: Task) -> bool:
        """Returns True if task passes ALL active filter criteria (AND logic).

        Filter semantics:
        - status_filter: Task status must be in the set (OR within set)
        - agent_type_filter: Case-insensitive substring match
        - feature_branch_filter: Case-insensitive substring match
        - text_search: Case-insensitive search in description and summary
        - source_filter: Exact match

        Args:
            task: Task to check against filter criteria

        Returns:
            True if task matches all active filters, False otherwise
        """
        # Status filter: Task must be in the allowed set
        if self.status_filter is not None:
            if task.status not in self.status_filter:
                return False

        # Agent type filter: Case-insensitive substring match
        if self.agent_type_filter:
            if (
                not task.agent_type
                or self.agent_type_filter.lower() not in task.agent_type.lower()
            ):
                return False

        # Feature branch filter: Case-insensitive substring match
        if self.feature_branch_filter:
            if (
                not task.feature_branch
                or self.feature_branch_filter.lower()
                not in task.feature_branch.lower()
            ):
                return False

        # Text search: Search in description and summary (case-insensitive)
        if self.text_search:
            search_lower = self.text_search.lower()
            description_match = search_lower in task.prompt.lower()
            summary_match = task.summary and search_lower in task.summary.lower()

            if not (description_match or summary_match):
                return False

        # Source filter: Exact match
        if self.source_filter is not None:
            if task.source != self.source_filter:
                return False

        # All active filters passed
        return True

    def clear(self) -> None:
        """Clear all filter criteria."""
        self.status_filter = None
        self.agent_type_filter = None
        self.feature_branch_filter = None
        self.text_search = None
        self.source_filter = None

    model_config = {
        "frozen": False,  # Allow mutation for interactive updates
        "validate_assignment": True,  # Validate on field updates
    }
