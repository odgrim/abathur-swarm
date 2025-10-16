"""MCP server exposing Abathur task queue management tools."""

import asyncio
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any
from uuid import UUID

# Add MCP SDK
try:
    from mcp.server import Server
    from mcp.server.stdio import stdio_server
    from mcp.types import TextContent, Tool
except ImportError:
    print("ERROR: mcp package not installed. Run: pip install mcp", file=sys.stderr)
    sys.exit(1)

# Add abathur to path
sys.path.insert(0, str(Path(__file__).parent.parent.parent))

# Import abathur modules
from abathur.domain.models import TaskSource, TaskStatus  # noqa: E402
from abathur.infrastructure.database import Database  # noqa: E402
from abathur.infrastructure.logger import get_logger  # noqa: E402
from abathur.services.dependency_resolver import (  # noqa: E402
    CircularDependencyError,
    DependencyResolver,
)
from abathur.services.priority_calculator import PriorityCalculator  # noqa: E402
from abathur.services.task_queue_service import (  # noqa: E402
    TaskNotFoundError,
    TaskQueueError,
    TaskQueueService,
)

logger = get_logger(__name__)


class AbathurTaskQueueServer:
    """MCP server for Abathur task queue management.

    Exposes tools for:
    - Task enqueuing with dependencies and priorities
    - Task retrieval and status monitoring
    - Queue status and statistics
    - Task cancellation with cascade
    - Execution plan calculation
    """

    def __init__(self, db_path: Path) -> None:
        """Initialize task queue server.

        Args:
            db_path: Path to SQLite database
        """
        self.db_path = db_path
        self._db: Database | None = None
        self._task_queue_service: TaskQueueService | None = None
        self._dependency_resolver: DependencyResolver | None = None
        self._priority_calculator: PriorityCalculator | None = None
        self.server = Server("abathur-task-queue")

        # Register tools
        self._register_tools()

    def _register_tools(self) -> None:
        """Register all MCP tools."""

        @self.server.list_tools()
        async def list_tools() -> list[Tool]:
            """List available tools."""
            return [
                Tool(
                    name="task_enqueue",
                    description="Enqueue a new task into Abathur's task queue with dependencies and priorities",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "Task description/instruction",
                            },
                            "source": {
                                "type": "string",
                                "enum": [
                                    "human",
                                    "agent_requirements",
                                    "agent_planner",
                                    "agent_implementation",
                                ],
                                "description": "Task source",
                            },
                            "agent_type": {
                                "type": "string",
                                "default": "requirements-gatherer",
                                "description": "Agent type to execute task",
                            },
                            "base_priority": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 10,
                                "default": 5,
                                "description": "User-specified priority (0-10)",
                            },
                            "prerequisites": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "List of prerequisite task IDs (UUIDs)",
                            },
                            "parent_task_id": {
                                "type": "string",
                                "description": "Parent task ID for hierarchical tasks",
                            },
                            "deadline": {
                                "type": "string",
                                "description": "Task deadline (ISO 8601 timestamp)",
                            },
                            "estimated_duration_seconds": {
                                "type": "integer",
                                "description": "Estimated execution time in seconds",
                            },
                            "session_id": {
                                "type": "string",
                                "description": "Session ID for memory context",
                            },
                            "input_data": {
                                "type": "object",
                                "description": "Additional input data",
                            },
                            "feature_branch": {
                                "type": "string",
                                "description": "Feature branch that task changes get merged into",
                            },
                        },
                        "required": ["description", "source"],
                    },
                ),
                Tool(
                    name="task_get",
                    description="Retrieve full task details by task ID",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "task_id": {
                                "type": "string",
                                "description": "Task ID (UUID)",
                            },
                        },
                        "required": ["task_id"],
                    },
                ),
                Tool(
                    name="task_list",
                    description="List tasks with filtering and pagination",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "status": {
                                "type": "string",
                                "enum": [
                                    "pending",
                                    "blocked",
                                    "ready",
                                    "running",
                                    "completed",
                                    "failed",
                                    "cancelled",
                                ],
                                "description": "Filter by task status",
                            },
                            "limit": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 500,
                                "default": 50,
                                "description": "Maximum results",
                            },
                            "source": {
                                "type": "string",
                                "enum": [
                                    "human",
                                    "agent_requirements",
                                    "agent_planner",
                                    "agent_implementation",
                                ],
                                "description": "Filter by task source",
                            },
                            "agent_type": {
                                "type": "string",
                                "description": "Filter by agent type",
                            },
                            "feature_branch": {
                                "type": "string",
                                "description": "Filter by feature branch name",
                            },
                        },
                    },
                ),
                Tool(
                    name="task_queue_status",
                    description="Get queue statistics for monitoring (task counts, priorities, depths)",
                    inputSchema={
                        "type": "object",
                        "properties": {},
                    },
                ),
                Tool(
                    name="task_cancel",
                    description="Cancel a task and cascade cancellation to all dependent tasks",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "task_id": {
                                "type": "string",
                                "description": "Task ID to cancel (UUID)",
                            },
                        },
                        "required": ["task_id"],
                    },
                ),
                Tool(
                    name="task_execution_plan",
                    description="Calculate execution plan for tasks (topological sort with parallelization)",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "task_ids": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "List of task IDs to plan (UUIDs)",
                            },
                        },
                        "required": ["task_ids"],
                    },
                ),
                Tool(
                    name="feature_branch_summary",
                    description="Get comprehensive summary of all tasks for a feature branch including status breakdown, progress metrics, and agent distribution",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "feature_branch": {
                                "type": "string",
                                "description": "Feature branch name",
                            },
                        },
                        "required": ["feature_branch"],
                    },
                ),
                Tool(
                    name="feature_branch_list",
                    description="List all feature branches with task statistics and completion rates",
                    inputSchema={
                        "type": "object",
                        "properties": {},
                    },
                ),
                Tool(
                    name="feature_branch_blockers",
                    description="Identify blocking issues (failed or blocked tasks) preventing feature branch completion",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "feature_branch": {
                                "type": "string",
                                "description": "Feature branch name",
                            },
                        },
                        "required": ["feature_branch"],
                    },
                ),
            ]

        @self.server.call_tool()
        async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
            """Handle tool calls."""
            if self._db is None or self._task_queue_service is None:
                return [
                    TextContent(
                        type="text",
                        text=json.dumps(
                            {"error": "InternalError", "message": "Database not initialized"}
                        ),
                    )
                ]

            try:
                # Task queue operations
                if name == "task_enqueue":
                    result = await self._handle_task_enqueue(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "task_get":
                    result = await self._handle_task_get(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "task_list":
                    result = await self._handle_task_list(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "task_queue_status":
                    result = await self._handle_task_queue_status(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "task_cancel":
                    result = await self._handle_task_cancel(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "task_execution_plan":
                    result = await self._handle_task_execution_plan(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "feature_branch_summary":
                    result = await self._handle_feature_branch_summary(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "feature_branch_list":
                    result = await self._handle_feature_branch_list(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                elif name == "feature_branch_blockers":
                    result = await self._handle_feature_branch_blockers(arguments)
                    return [TextContent(type="text", text=json.dumps(result, default=str))]

                else:
                    return [
                        TextContent(
                            type="text",
                            text=json.dumps(
                                {"error": "UnknownTool", "message": f"Unknown tool: {name}"}
                            ),
                        )
                    ]

            except Exception as e:
                logger.error("mcp_tool_error", tool=name, error=str(e))
                return [
                    TextContent(
                        type="text",
                        text=json.dumps(
                            {"error": "InternalError", "message": str(e), "tool": name}
                        ),
                    )
                ]

    async def _handle_task_enqueue(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle task_enqueue tool invocation."""
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
        feature_branch = arguments.get("feature_branch")

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

        # Validate priority range
        if not isinstance(base_priority, int) or not 0 <= base_priority <= 10:
            return {
                "error": "ValidationError",
                "message": f"base_priority must be an integer in range [0, 10], got {base_priority}",
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
            assert self._task_queue_service is not None
            task = await self._task_queue_service.enqueue_task(
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
                feature_branch=feature_branch,
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

    async def _handle_task_get(self, arguments: dict[str, Any]) -> dict[str, Any]:
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
            assert self._db is not None
            task = await self._db.get_task(task_id)

            if not task:
                return {
                    "error": "NotFoundError",
                    "message": f"Task {task_id} not found",
                }

            return self._serialize_task(task)

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_list(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle task_list tool invocation."""
        # Optional filters
        status_filter = arguments.get("status")
        limit = arguments.get("limit", 50)
        source_filter = arguments.get("source")
        agent_type_filter = arguments.get("agent_type")
        feature_branch_filter = arguments.get("feature_branch")

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
            assert self._db is not None
            # Build query filters
            filters: dict[str, Any] = {}
            if status_filter:
                filters["status"] = TaskStatus(status_filter)
            if source_filter:
                filters["source"] = TaskSource(source_filter)
            if agent_type_filter:
                filters["agent_type"] = agent_type_filter
            if feature_branch_filter:
                filters["feature_branch"] = feature_branch_filter

            tasks = await self._db.list_tasks(limit=limit, **filters)

            return {"tasks": [self._serialize_task(task) for task in tasks]}

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_queue_status(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle task_queue_status tool invocation."""
        try:
            assert self._task_queue_service is not None
            status = await self._task_queue_service.get_queue_status()

            # Serialize datetime fields
            result = {**status}
            if status.get("oldest_pending"):
                result["oldest_pending"] = status["oldest_pending"].isoformat()
            if status.get("newest_task"):
                result["newest_task"] = status["newest_task"].isoformat()

            return result

        except Exception as e:
            return {"error": "InternalError", "message": str(e)}

    async def _handle_task_cancel(self, arguments: dict[str, Any]) -> dict[str, Any]:
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
            assert self._task_queue_service is not None
            cancelled_ids = await self._task_queue_service.cancel_task(task_id)

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

    async def _handle_task_execution_plan(self, arguments: dict[str, Any]) -> dict[str, Any]:
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
            assert self._task_queue_service is not None
            batches = await self._task_queue_service.get_task_execution_plan(task_ids)

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

    async def _handle_feature_branch_summary(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle feature_branch_summary tool invocation."""
        feature_branch = arguments.get("feature_branch")
        if not feature_branch:
            return {"error": "ValidationError", "message": "feature_branch is required"}

        try:
            assert self._db is not None
            summary = await self._db.get_feature_branch_summary(feature_branch)
            return summary
        except Exception as e:
            logger.error("feature_branch_summary_error", feature_branch=feature_branch, error=str(e))
            return {"error": "InternalError", "message": str(e)}

    async def _handle_feature_branch_list(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle feature_branch_list tool invocation."""
        try:
            assert self._db is not None
            branches = await self._db.list_feature_branches()
            return {"feature_branches": branches, "count": len(branches)}
        except Exception as e:
            logger.error("feature_branch_list_error", error=str(e))
            return {"error": "InternalError", "message": str(e)}

    async def _handle_feature_branch_blockers(self, arguments: dict[str, Any]) -> dict[str, Any]:
        """Handle feature_branch_blockers tool invocation."""
        feature_branch = arguments.get("feature_branch")
        if not feature_branch:
            return {"error": "ValidationError", "message": "feature_branch is required"}

        try:
            assert self._db is not None
            blockers = await self._db.get_feature_branch_blockers(feature_branch)
            return {
                "feature_branch": feature_branch,
                "blockers": blockers,
                "blocker_count": len(blockers),
                "has_blockers": len(blockers) > 0,
            }
        except Exception as e:
            logger.error("feature_branch_blockers_error", feature_branch=feature_branch, error=str(e))
            return {"error": "InternalError", "message": str(e)}

    def _serialize_task(self, task: Any) -> dict[str, Any]:
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
            "feature_branch": task.feature_branch,
        }

    async def run(self) -> None:
        """Run the MCP server."""
        # Initialize database
        self._db = Database(self.db_path)
        await self._db.initialize()

        # Initialize services
        self._dependency_resolver = DependencyResolver(self._db)
        self._priority_calculator = PriorityCalculator(self._dependency_resolver)
        self._task_queue_service = TaskQueueService(
            self._db,
            self._dependency_resolver,
            self._priority_calculator,
        )

        logger.info("abathur_task_queue_mcp_server_started", db_path=str(self.db_path))

        # Run stdio server
        async with stdio_server() as (read_stream, write_stream):
            await self.server.run(
                read_stream, write_stream, self.server.create_initialization_options()
            )


async def main() -> None:
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(description="Abathur Task Queue MCP Server")
    parser.add_argument(
        "--db-path",
        type=Path,
        default=Path.cwd() / "abathur.db",
        help="Path to SQLite database (default: ./abathur.db)",
    )

    args = parser.parse_args()

    server = AbathurTaskQueueServer(args.db_path)
    await server.run()


if __name__ == "__main__":
    asyncio.run(main())
