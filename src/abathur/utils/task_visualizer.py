"""Task Dependency Visualization Utility

This module provides tools to visualize task dependencies using GraphViz and Mermaid formats.
"""

import asyncio
from uuid import UUID

import graphviz
from abathur.domain.models import TaskStatus
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator
from abathur.services.task_queue_service import TaskQueueService


class TaskDependencyVisualizer:
    def __init__(self, queue_service: TaskQueueService, dependency_resolver: DependencyResolver):
        """Initialize visualizer with task queue and dependency services.

        Args:
            queue_service (TaskQueueService): Service for task operations
            dependency_resolver (DependencyResolver): Service for dependency graph operations
        """
        self.queue_service = queue_service
        self.dependency_resolver = dependency_resolver

    async def get_dependency_graph(self, task_ids: list[UUID]) -> dict[UUID, list[UUID]]:
        """Build dependency graph for given tasks.

        Args:
            task_ids (List[UUID]): Tasks to include in graph

        Returns:
            Dict[UUID, List[UUID]]: Adjacency list representing dependencies
        """
        graph = {}
        for task_id in task_ids:
            task_dependencies = await self.queue_service._db.get_task_dependencies(task_id)
            graph[task_id] = [dep.prerequisite_task_id for dep in task_dependencies]
        return graph

    async def export_graphviz(
        self, graph: dict[UUID, list[UUID]], output_path: str = "task_dependencies.dot"
    ) -> str:
        """Export dependency graph to GraphViz DOT format.

        Args:
            graph (Dict[UUID, List[UUID]]): Dependency graph
            output_path (str): File path to save GraphViz DOT file

        Returns:
            str: GraphViz DOT representation
        """
        dot = graphviz.Digraph(comment="Task Dependencies")
        dot.attr(rankdir="LR")  # Left to right layout

        # Add nodes and edges
        for task_id, dependencies in graph.items():
            task = await self.queue_service._db.get_task(task_id)
            if not task:
                continue

            node_color = {
                TaskStatus.PENDING: "lightblue",
                TaskStatus.BLOCKED: "yellow",
                TaskStatus.READY: "lightgreen",
                TaskStatus.RUNNING: "orange",
                TaskStatus.COMPLETED: "green",
                TaskStatus.FAILED: "red",
                TaskStatus.CANCELLED: "gray",
            }.get(task.status, "white")

            dot.node(
                str(task_id),
                f"{str(task_id)[:8]}\n{task.prompt[:30]}",
                style="filled",
                fillcolor=node_color,
            )

            for dep_id in dependencies:
                dot.edge(str(dep_id), str(task_id))

        # Save and return DOT representation
        dot.render(output_path, format="dot", cleanup=True)
        return str(dot.source)

    async def export_mermaid(
        self, graph: dict[UUID, list[UUID]], output_path: str = "task_dependencies.mmd"
    ) -> str:
        """Export dependency graph to Mermaid markdown format.

        Args:
            graph (Dict[UUID, List[UUID]]): Dependency graph
            output_path (str): File path to save Mermaid markdown

        Returns:
            str: Mermaid graph representation
        """
        mermaid_lines = ["```mermaid", "graph LR"]

        # Add nodes and edges
        for task_id, dependencies in graph.items():
            task = await self.queue_service._db.get_task(task_id)
            if not task:
                continue

            status_class = {
                TaskStatus.PENDING: "pending",
                TaskStatus.BLOCKED: "blocked",
                TaskStatus.READY: "ready",
                TaskStatus.RUNNING: "running",
                TaskStatus.COMPLETED: "completed",
                TaskStatus.FAILED: "failed",
                TaskStatus.CANCELLED: "cancelled",
            }.get(task.status, "default")

            mermaid_lines.append(f"    {str(task_id)[:8]}[{task.prompt[:30]}] --> {status_class}")

            for dep_id in dependencies:
                mermaid_lines.append(f"    {str(dep_id)[:8]} --> {str(task_id)[:8]}")

        mermaid_lines.extend(["```", ""])

        # Save Mermaid markdown
        with open(output_path, "w") as f:
            f.write("\n".join(mermaid_lines))

        return "\n".join(mermaid_lines)

    async def cli_visualize(self, task_ids: list[UUID], format: str = "graphviz") -> None:
        """CLI interface for task dependency visualization.

        Args:
            task_ids (List[UUID]): Tasks to visualize
            format (str): Visualization format ('graphviz' or 'mermaid')
        """
        graph = await self.get_dependency_graph(task_ids)

        if format == "graphviz":
            dot_output = await self.export_graphviz(graph)
            print("GraphViz Dependency Graph:\n", dot_output)
        elif format == "mermaid":
            mermaid_output = await self.export_mermaid(graph)
            print("Mermaid Dependency Graph:\n", mermaid_output)
        else:
            raise ValueError("Unsupported visualization format. Use 'graphviz' or 'mermaid'.")


async def main() -> None:
    """Example CLI usage."""
    import sys
    from pathlib import Path

    if len(sys.argv) < 3:
        print("Usage: python task_visualizer.py <format> <task_id1> [task_id2 ...]")
        sys.exit(1)

    format_arg = sys.argv[1]
    task_ids = [UUID(task_id) for task_id in sys.argv[2:]]

    # Initialize services
    db = Database(Path("abathur.db"))
    await db.initialize()
    dependency_resolver = DependencyResolver(db)
    priority_calculator = PriorityCalculator(dependency_resolver)
    queue_service = TaskQueueService(db, dependency_resolver, priority_calculator)

    visualizer = TaskDependencyVisualizer(queue_service, dependency_resolver)
    await visualizer.cli_visualize(task_ids, format_arg)


if __name__ == "__main__":
    asyncio.run(main())
