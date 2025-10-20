"""TUI-specific data models for TaskQueueTUI.

This module contains view models and enums specific to the TUI layer.
Domain models (Task, TaskStatus, etc.) are imported from abathur.domain.models.
"""

from enum import Enum


class ViewMode(str, Enum):
    """Available view modes for task visualization in the TUI.

    Attributes:
        TREE: Hierarchical view by parent_task_id and dependency_depth (default)
        DEPENDENCY: Focused view on prerequisite relationships
        TIMELINE: Chronological view sorted by submitted_at timestamp
        FEATURE_BRANCH: Grouped view by feature_branch field
        FLAT_LIST: Flat list sorted by calculated_priority
    """

    TREE = "tree"
    DEPENDENCY = "dependency"
    TIMELINE = "timeline"
    FEATURE_BRANCH = "feature_branch"
    FLAT_LIST = "flat_list"
