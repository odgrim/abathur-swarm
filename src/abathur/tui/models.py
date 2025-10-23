"""TUI-specific data models for task visualization and interaction.

This module defines Pydantic V2 models for the Textual-based TUI, including:
- ViewMode: Available visualization modes
- FilterState: Task filtering criteria and matching logic
- NavigationState: User navigation and UI state
- CachedTaskData: Task data cache with TTL expiration
- TreeNode: Individual node in tree visualization
- TreeLayout: Complete tree structure for rendering
- FeatureBranchSummary: Feature branch statistics
"""

from datetime import datetime, timezone
from enum import Enum
from typing import Any
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field

from abathur.domain.models import Task, TaskStatus, TaskSource


class ViewMode(str, Enum):
    """Available view modes for task visualization."""

    TREE = "tree"  # Hierarchical tree view (default)
    DEPENDENCY = "dependency"  # Dependency graph view
    TIMELINE = "timeline"  # Chronological timeline view
    FEATURE_BRANCH = "feature_branch"  # Grouped by feature branch
    FLAT_LIST = "flat_list"  # Simple flat list view


class FilterState(BaseModel):
    """Encapsulates filter criteria for task list with multi-criteria AND logic.

    All filter criteria are ANDed together. A task must match ALL active
    filters to be included in filtered results.

    Attributes:
        status_filter: Set of TaskStatus values to include (OR within set)
        agent_type_filter: Agent type string for substring match (None = all)
        feature_branch_filter: Feature branch for substring match (None = all)
        text_search: Text to search in summary/prompt (None = no search)
        source_filter: TaskSource to filter by (None = all sources)
    """

    status_filter: set[TaskStatus] | None = Field(
        default=None, description="Filter by task status (None = all statuses)"
    )
    agent_type_filter: str | None = Field(
        default=None, description="Filter by agent type (case-insensitive substring, None = all agents)"
    )
    feature_branch_filter: str | None = Field(
        default=None, description="Filter by feature branch (case-insensitive substring, None = all branches)"
    )
    text_search: str | None = Field(
        default=None, description="Search text in summary/prompt (case-insensitive, None = no search)"
    )
    source_filter: TaskSource | None = Field(
        default=None, description="Filter by task source (exact match, None = all sources)"
    )

    model_config = ConfigDict()

    def is_active(self) -> bool:
        """Returns True if any filter is currently set.

        Returns:
            bool: True if at least one filter is active
        """
        return (
            self.status_filter is not None
            or self.agent_type_filter is not None
            or self.feature_branch_filter is not None
            or self.text_search is not None
            or self.source_filter is not None
        )

    def matches(self, task: Task) -> bool:
        """Returns True if task passes all active filters (AND logic).

        All active filters must match for the task to pass.

        Filter semantics:
        - status_filter: Task status must be in the set (OR within set)
        - agent_type_filter: Case-insensitive substring match
        - feature_branch_filter: Case-insensitive substring match
        - text_search: Case-insensitive search in description and summary

        Args:
            task: Task to check against filters

        Returns:
            bool: True if task passes all active filters
        """
        # Status filter: Task must be in the allowed set
        if self.status_filter is not None:
            if task.status not in self.status_filter:
                return False

        # Agent type filter: Case-insensitive substring match
        if self.agent_type_filter is not None:
            if (
                not task.agent_type
                or self.agent_type_filter.lower() not in task.agent_type.lower()
            ):
                return False

        # Feature branch filter: Case-insensitive substring match
        if self.feature_branch_filter is not None:
            if (
                not task.feature_branch
                or self.feature_branch_filter.lower()
                not in task.feature_branch.lower()
            ):
                return False

        # Text search filter (case-insensitive, searches summary and prompt)
        if self.text_search is not None:
            search_lower = self.text_search.lower()
            summary_match = (
                task.summary and search_lower in task.summary.lower()
            )
            prompt_match = search_lower in task.prompt.lower()
            if not (summary_match or prompt_match):
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


class NavigationState(BaseModel):
    """Tracks user navigation state within TUI.

    Attributes:
        selected_task_id: Currently selected task (None = no selection)
        expanded_nodes: Set of task IDs with expanded children
        scroll_position: Current scroll position in view
        focus_widget: Name of currently focused widget
    """

    selected_task_id: UUID | None = Field(
        default=None, description="Currently selected task ID"
    )
    expanded_nodes: set[UUID] = Field(
        default_factory=set, description="Set of expanded tree node IDs"
    )
    scroll_position: int = Field(
        default=0, ge=0, description="Current scroll position (non-negative)"
    )
    focus_widget: str = Field(
        default="tree", description="Name of currently focused widget"
    )

    model_config = ConfigDict()


class CachedTaskData(BaseModel):
    """Cache wrapper for task data with TTL-based expiration.

    Attributes:
        tasks: List of cached tasks
        dependency_graph: Map of task_id to list of prerequisite task_ids
        cached_at: Timestamp when cache was created
        ttl_seconds: Time-to-live in seconds (default 2.0)
    """

    tasks: list[Task] = Field(description="Cached task list")
    dependency_graph: dict[UUID, list[UUID]] = Field(
        description="Map of task_id to prerequisite task_ids"
    )
    cached_at: datetime = Field(description="Cache creation timestamp")
    ttl_seconds: float = Field(
        default=2.0, gt=0, description="Cache TTL in seconds (must be positive)"
    )

    model_config = ConfigDict()

    def is_expired(self) -> bool:
        """Returns True if cache has exceeded TTL.

        Returns:
            bool: True if cache is expired
        """
        now = datetime.now(timezone.utc)
        elapsed_seconds = (now - self.cached_at).total_seconds()
        return elapsed_seconds >= self.ttl_seconds


class TreeNode(BaseModel):
    """Node in rendered tree structure.

    Attributes:
        task_id: UUID of the task this node represents
        task: The actual Task object
        children: List of child task IDs
        level: Depth in tree (same as task.dependency_depth)
        is_expanded: Whether node's children are visible
        position: Order within level (for stable sorting)
    """

    task_id: UUID = Field(description="Task UUID")
    task: Task = Field(description="Task object")
    children: list[UUID] = Field(
        default_factory=list, description="Child task IDs"
    )
    level: int = Field(
        ge=0, description="Tree depth (same as dependency_depth)"
    )
    is_expanded: bool = Field(
        default=True, description="Whether children are visible"
    )
    position: int = Field(ge=0, description="Order within level")

    model_config = ConfigDict()


class TreeLayout(BaseModel):
    """Complete tree layout structure for rendering.

    Attributes:
        nodes: Map of task_id to TreeNode
        root_nodes: List of root task IDs (no parent_task_id)
        max_depth: Maximum tree depth
        total_nodes: Total number of nodes in tree
    """

    nodes: dict[UUID, TreeNode] = Field(description="Map of task_id to TreeNode")
    root_nodes: list[UUID] = Field(
        description="Root task IDs (tasks with no parent)"
    )
    max_depth: int = Field(ge=0, description="Maximum tree depth")
    total_nodes: int = Field(ge=0, description="Total number of nodes")

    model_config = ConfigDict()

    def get_visible_nodes(self, expanded_nodes: set[UUID]) -> list[TreeNode]:
        """Returns only visible nodes based on expand/collapse state.

        A node is visible if:
        1. It's a root node, OR
        2. All its ancestors are expanded

        Args:
            expanded_nodes: Set of expanded node IDs

        Returns:
            list[TreeNode]: List of visible nodes in tree order
        """
        visible: list[TreeNode] = []

        def is_visible(node_id: UUID) -> bool:
            """Check if node should be visible."""
            node = self.nodes.get(node_id)
            if node is None:
                return False

            # Root nodes are always visible
            parent_id = node.task.parent_task_id
            if parent_id is None:
                return True

            # Non-root: visible if parent is expanded AND parent is visible
            parent_expanded = parent_id in expanded_nodes
            parent_visible = is_visible(parent_id)
            return parent_expanded and parent_visible

        # Traverse in level order to maintain tree structure
        def traverse(node_id: UUID) -> None:
            """Recursively collect visible nodes."""
            if is_visible(node_id):
                node = self.nodes[node_id]
                visible.append(node)
                # Only traverse children if this node is expanded
                if node_id in expanded_nodes:
                    for child_id in node.children:
                        traverse(child_id)

        # Start traversal from root nodes
        for root_id in self.root_nodes:
            traverse(root_id)

        return visible

    def find_node_path(self, task_id: UUID) -> list[UUID]:
        """Returns path from root to specified node.

        Args:
            task_id: Target task ID

        Returns:
            list[UUID]: Path from root to node (empty if not found)
        """
        node = self.nodes.get(task_id)
        if node is None:
            return []

        path: list[UUID] = [task_id]
        current_id = task_id

        # Walk up the tree to root
        while True:
            current_node = self.nodes.get(current_id)
            if current_node is None:
                break

            parent_id = current_node.task.parent_task_id
            if parent_id is None:
                # Reached root
                break

            path.insert(0, parent_id)
            current_id = parent_id

        return path


class FeatureBranchSummary(BaseModel):
    """Summary statistics for a feature branch.

    Attributes:
        feature_branch: Feature branch name
        total_tasks: Total number of tasks in branch
        status_counts: Count of tasks by status
        blockers: List of failed or blocked tasks
        completion_rate: Completion rate (0.0-1.0)
        avg_priority: Average priority of tasks
    """

    feature_branch: str = Field(description="Feature branch name")
    total_tasks: int = Field(ge=0, description="Total task count")
    status_counts: dict[TaskStatus, int] = Field(
        description="Task counts by status"
    )
    blockers: list[Task] = Field(
        default_factory=list,
        description="Failed or blocked tasks preventing completion",
    )
    completion_rate: float = Field(
        ge=0.0, le=1.0, description="Completion rate (0.0-1.0)"
    )
    avg_priority: float = Field(
        ge=0.0, le=10.0, description="Average task priority"
    )

    model_config = ConfigDict()
