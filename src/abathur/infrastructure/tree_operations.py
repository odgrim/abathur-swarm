"""Tree operation methods extracted from Database class.

This module contains all tree-related operations for task hierarchies,
including tree traversal, validation, and structural analysis.
"""

from typing import TYPE_CHECKING, Any
from uuid import UUID

from abathur.domain.models import TaskStatus, TreeNode

if TYPE_CHECKING:
    from abathur.infrastructure.database import Database


class TreeOperations:
    """Handles tree-related operations for task hierarchies.

    This class encapsulates all tree traversal, validation, and structural
    operations that were previously part of the Database class. It uses
    delegation pattern to access database connection methods.
    """

    def __init__(self, database: "Database") -> None:
        """Initialize TreeOperations with database reference.

        Args:
            database: Database instance for accessing connection methods
        """
        self.db = database

    async def get_task_tree_with_status(
        self,
        root_task_ids: list[UUID],
        filter_statuses: list[TaskStatus] | None = None,
        max_depth: int = 100,
    ) -> dict[UUID, TreeNode]:
        """Use WITH RECURSIVE CTE to retrieve descendant tree with optional status filtering.

        Args:
            root_task_ids: Root task IDs to start traversal
            filter_statuses: Optional status filter (None = all descendants)
            max_depth: Maximum traversal depth to prevent infinite loops

        Returns:
            Mapping of task_id -> TreeNode for all descendants (including roots)

        Raises:
            ValueError: If root_task_ids empty or max_depth invalid
            RuntimeError: If tree depth exceeds max_depth (cycle detected)
        """
        # Validate parameters
        if not root_task_ids:
            raise ValueError("root_task_ids cannot be empty")
        if max_depth <= 0 or max_depth > 1000:
            raise ValueError("max_depth must be between 1 and 1000")

        async with self.db._get_connection() as conn:
            # Build root_id_placeholders
            root_id_placeholders = ",".join("?" * len(root_task_ids))

            # Build status filter clause
            status_filter_sql = ""
            status_params = []
            if filter_statuses is not None and len(filter_statuses) > 0:
                status_placeholders = ",".join("?" * len(filter_statuses))
                status_filter_sql = f"WHERE status IN ({status_placeholders})"
                status_params = [status.value for status in filter_statuses]

            # Build the WITH RECURSIVE CTE query
            query = f"""
                WITH RECURSIVE task_tree AS (
                    -- Base case: root tasks
                    SELECT
                        id,
                        parent_task_id,
                        status,
                        0 AS depth
                    FROM tasks
                    WHERE id IN ({root_id_placeholders})

                    UNION ALL

                    -- Recursive case: children of current level
                    SELECT
                        t.id,
                        t.parent_task_id,
                        t.status,
                        tt.depth + 1 AS depth
                    FROM tasks t
                    INNER JOIN task_tree tt ON t.parent_task_id = tt.id
                    WHERE tt.depth < ?
                )
                SELECT
                    id,
                    parent_task_id,
                    status,
                    depth
                FROM task_tree
                {status_filter_sql}
                ORDER BY depth ASC, id ASC
            """

            # Build parameters: root IDs + max_depth + optional status filters
            params = [str(task_id) for task_id in root_task_ids]
            params.append(max_depth)
            params.extend(status_params)

            # Execute query
            cursor = await conn.execute(query, tuple(params))
            rows = await cursor.fetchall()

            # Check if we hit the max_depth limit (potential cycle)
            if rows:
                max_observed_depth = max(row["depth"] for row in rows)
                if max_observed_depth >= max_depth:
                    raise RuntimeError(
                        f"Tree depth exceeded max_depth={max_depth}. "
                        "This may indicate a cycle in parent_task_id relationships."
                    )

            # Build TreeNode mapping
            tree_nodes: dict[UUID, TreeNode] = {}

            # First pass: Create all TreeNode objects
            for row in rows:
                task_id = UUID(row["id"])
                parent_id = UUID(row["parent_task_id"]) if row["parent_task_id"] else None
                status = TaskStatus(row["status"])
                depth = row["depth"]

                tree_nodes[task_id] = TreeNode(
                    id=task_id,
                    parent_id=parent_id,
                    status=status,
                    depth=depth,
                    children_ids=[],
                )

            # Second pass: Build children_ids lists by iterating results
            for row in rows:
                task_id = UUID(row["id"])
                parent_id = UUID(row["parent_task_id"]) if row["parent_task_id"] else None

                if parent_id and parent_id in tree_nodes:
                    # Add this task to parent's children_ids
                    tree_nodes[parent_id].children_ids.append(task_id)

            return tree_nodes

    async def check_tree_all_match_status(
        self,
        root_task_ids: list[UUID],
        allowed_statuses: list[TaskStatus]
    ) -> dict[UUID, bool]:
        """Check if entire tree matches deletion criteria.

        For each root task, recursively checks if all descendants (including the root)
        have statuses in the allowed_statuses list. Uses SQL WITH RECURSIVE for
        efficient tree traversal.

        Args:
            root_task_ids: Root task IDs to check (non-empty list required)
            allowed_statuses: Statuses matching deletion criteria (e.g., [COMPLETED, FAILED, CANCELLED])

        Returns:
            Mapping of root_task_id -> bool (True if all descendants match, False otherwise)

        Raises:
            ValueError: If root_task_ids or allowed_statuses is empty
            DatabaseError: If SQL query fails

        Performance:
            O(n) where n = total descendants across all trees
        """
        # Validate parameters
        if not root_task_ids:
            raise ValueError("root_task_ids cannot be empty")
        if not allowed_statuses:
            raise ValueError("allowed_statuses cannot be empty")

        # Convert statuses to values for SQL
        status_values = [status.value for status in allowed_statuses]

        result: dict[UUID, bool] = {}

        async with self.db._get_connection() as conn:
            # Process each root task individually
            # Future optimization: batch multiple roots into single query
            for root_task_id in root_task_ids:
                root_id_str = str(root_task_id)

                # Build status filter placeholders
                status_placeholders = ",".join("?" * len(status_values))

                # Count total descendants (including root) using WITH RECURSIVE
                total_count_query = f"""
                    WITH RECURSIVE task_tree(id, parent_task_id, status, depth) AS (
                        -- Base case: root task
                        SELECT id, parent_task_id, status, 0 as depth
                        FROM tasks
                        WHERE id = ?

                        UNION ALL

                        -- Recursive case: children of tasks in tree
                        SELECT t.id, t.parent_task_id, t.status, tree.depth + 1
                        FROM tasks t
                        INNER JOIN task_tree tree ON t.parent_task_id = tree.id
                        WHERE tree.depth < 100  -- Prevent infinite loops
                    )
                    SELECT COUNT(*) as total_count
                    FROM task_tree
                """

                cursor = await conn.execute(total_count_query, (root_id_str,))
                total_row = await cursor.fetchone()
                total_count = total_row["total_count"] if total_row else 0

                # Count descendants with allowed statuses using WITH RECURSIVE
                matching_count_query = f"""
                    WITH RECURSIVE task_tree(id, parent_task_id, status, depth) AS (
                        -- Base case: root task
                        SELECT id, parent_task_id, status, 0 as depth
                        FROM tasks
                        WHERE id = ?

                        UNION ALL

                        -- Recursive case: children of tasks in tree
                        SELECT t.id, t.parent_task_id, t.status, tree.depth + 1
                        FROM tasks t
                        INNER JOIN task_tree tree ON t.parent_task_id = tree.id
                        WHERE tree.depth < 100  -- Prevent infinite loops
                    )
                    SELECT COUNT(*) as matching_count
                    FROM task_tree
                    WHERE status IN ({status_placeholders})
                """

                cursor = await conn.execute(
                    matching_count_query,
                    (root_id_str, *status_values)
                )
                matching_row = await cursor.fetchone()
                matching_count = matching_row["matching_count"] if matching_row else 0

                # All descendants match if counts are equal and > 0
                all_match = total_count > 0 and total_count == matching_count
                result[root_task_id] = all_match

        return result

    def _build_tree_structure(self, tree_nodes: list[Any]) -> dict[UUID, list[UUID]]:
        """Build parent -> children adjacency list from flat tree nodes.

        This helper method constructs a hierarchical tree structure from a flat
        list of TreeNode objects. It builds an adjacency list mapping each parent
        task ID to its list of child task IDs, and populates the children field
        in each TreeNode object.

        Args:
            tree_nodes: Flat list of TreeNode objects with task_id, task, and
                       parent_task_id attributes. The children field will be
                       populated by this method.

        Returns:
            Dict mapping parent_id (UUID) to list of child IDs (list[UUID]).
            Parent IDs with no children are not included in the dict.

        Example:
            >>> nodes = [
            ...     TreeNode(task_id=uuid1, parent_id=None, children=[]),
            ...     TreeNode(task_id=uuid2, parent_id=uuid1, children=[]),
            ...     TreeNode(task_id=uuid3, parent_id=uuid1, children=[]),
            ... ]
            >>> adjacency = tree_ops._build_tree_structure(nodes)
            >>> adjacency[uuid1]  # Returns [uuid2, uuid3]
            >>> nodes[0].children  # Now contains [uuid2, uuid3]
        """
        # Build adjacency list mapping parent_id -> [child_ids]
        children_map: dict[UUID, list[UUID]] = {}

        for node in tree_nodes:
            # Get parent_task_id from the task object
            parent_id = node.task.parent_task_id
            if parent_id is not None:
                if parent_id not in children_map:
                    children_map[parent_id] = []
                children_map[parent_id].append(node.task_id)

        # Populate children field in TreeNode objects
        for node in tree_nodes:
            node.children = children_map.get(node.task_id, [])

        return children_map

    def _validate_tree_deletability(
        self,
        tree: dict[UUID, Any],
        root_id: UUID,
        allowed_statuses: list[TaskStatus],
    ) -> set[UUID]:
        """Validate which tasks in tree can be deleted based on status criteria.

        Implements FR003: Partial tree preservation. If any descendant doesn't match
        the allowed statuses, the entire subtree from that point up to root is preserved.

        Args:
            tree: Dict mapping task_id to TreeNode objects
            root_id: Root task ID to start validation from
            allowed_statuses: List of statuses that are allowed for deletion

        Returns:
            Set of task IDs that can be safely deleted. Empty set means preserve entire tree.

        Example:
            >>> # Root COMPLETED, child RUNNING -> preserve both
            >>> result = tree_ops._validate_tree_deletability(tree, root_id, [TaskStatus.COMPLETED])
            >>> assert result == set()  # Empty = preserve all
        """
        # Check if root exists in tree
        if root_id not in tree:
            return set()

        # Recursive validation: check all descendants
        def validate_subtree(task_id: UUID) -> tuple[bool, set[UUID]]:
            """Validate subtree and return (all_match, deletable_ids)."""
            if task_id not in tree:
                return (True, set())

            node = tree[task_id]
            task_status = node.task.status

            # Check if this node matches allowed statuses
            node_matches = task_status in allowed_statuses

            # Check all children recursively
            all_children_match = True
            deletable_children: set[UUID] = set()

            for child_id in node.children:
                child_matches, child_deletable = validate_subtree(child_id)
                if not child_matches:
                    all_children_match = False
                deletable_children.update(child_deletable)

            # FR003: If this node matches AND all children match -> include in deletable set
            if node_matches and all_children_match:
                return (True, {task_id} | deletable_children)
            # FR003: If this node doesn't match but some children do -> delete children only
            elif not node_matches and deletable_children:
                return (False, deletable_children)
            # FR003: If this node matches but some child doesn't -> preserve entire subtree
            elif node_matches and not all_children_match:
                return (False, set())
            # Neither this node nor children match
            else:
                return (False, set())

        all_match, deletable_ids = validate_subtree(root_id)
        return deletable_ids
