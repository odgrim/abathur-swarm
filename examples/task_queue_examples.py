"""Task Queue System Examples

This module demonstrates various usage patterns of the Abathur Task Queue System.
"""

import asyncio
from datetime import datetime, timedelta
from typing import Any

from abathur.domain.models import DependencyType, TaskSource
from abathur.services import TaskQueueService


class TaskQueueExamples:
    def __init__(self):  # type: ignore
        self.queue_service = TaskQueueService()  # type: ignore

    async def simple_linear_workflow(self) -> dict[str, Any]:
        """Demonstrate a simple linear task submission workflow.

        A high-level task with sequentially executed subtasks.

        Returns:
            Dict with workflow results and task details
        """
        # Human-submitted parent task
        parent_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Implement user authentication system",
            source=TaskSource.HUMAN,
            priority=8,
            deadline=datetime.now() + timedelta(days=14),
        )

        # Requirements gathering subtask
        requirements_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Define authentication requirements",
            source=TaskSource.AGENT_REQUIREMENTS,
            parent_task_id=parent_task.id,
            dependencies=[parent_task.id],
            priority=7,
        )

        # Database design subtask
        schema_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Design authentication database schema",
            source=TaskSource.AGENT_REQUIREMENTS,
            parent_task_id=parent_task.id,
            dependencies=[requirements_task.id],
            priority=6,
        )

        # Implementation subtasks
        jwt_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Implement JWT token generation",
            source=TaskSource.AGENT_PLANNER,
            parent_task_id=parent_task.id,
            dependencies=[schema_task.id],
            priority=5,
        )

        login_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Implement login endpoint",
            source=TaskSource.AGENT_IMPLEMENTATION,
            parent_task_id=parent_task.id,
            dependencies=[jwt_task.id],
            priority=4,
        )

        return {
            "workflow_name": "Authentication System",
            "parent_task_id": parent_task.id,
            "subtasks": [requirements_task.id, schema_task.id, jwt_task.id, login_task.id],
        }

    async def parallel_execution_example(self) -> dict[str, Any]:
        """Demonstrate parallel task execution with multiple prerequisites.

        Shows a task that requires multiple independent tasks to complete.

        Returns:
            Dict with workflow details and task results
        """
        # Independent data gathering tasks
        user_data_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Fetch user data from API", source=TaskSource.AGENT_REQUIREMENTS, priority=5
        )

        product_catalog_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Fetch product catalog from API",
            source=TaskSource.AGENT_REQUIREMENTS,
            priority=5,
        )

        # Task requiring both data sources (parallel dependency)
        recommendation_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Generate personalized product recommendations",
            source=TaskSource.AGENT_PLANNER,
            dependencies=[user_data_task.id, product_catalog_task.id],
            dependency_type=DependencyType.PARALLEL,
            priority=8,
        )

        return {
            "workflow_name": "Recommendation Generation",
            "prerequisite_tasks": [user_data_task.id, product_catalog_task.id],
            "dependent_task": recommendation_task.id,
        }

    async def error_handling_workflow(self) -> dict[str, Any]:
        """Demonstrate error handling and task failure propagation.

        Shows how task failures can trigger cascading effects.

        Returns:
            Dict with workflow error details
        """
        try:
            # Parent task
            deployment_task = await self.queue_service.submit_task(  # type: ignore
                prompt="Deploy application to production",
                source=TaskSource.HUMAN,
                priority=9,
                deadline=datetime.now() + timedelta(hours=4),
            )

            # Subtasks
            build_task = await self.queue_service.submit_task(  # type: ignore
                prompt="Build application package",
                source=TaskSource.AGENT_IMPLEMENTATION,
                parent_task_id=deployment_task.id,
                dependencies=[deployment_task.id],
                priority=7,
            )

            test_task = await self.queue_service.submit_task(  # type: ignore
                prompt="Run integration tests",
                source=TaskSource.AGENT_IMPLEMENTATION,
                parent_task_id=deployment_task.id,
                dependencies=[build_task.id],
                priority=6,
            )

            deploy_task = await self.queue_service.submit_task(  # type: ignore
                prompt="Deploy to production server",
                source=TaskSource.AGENT_IMPLEMENTATION,
                parent_task_id=deployment_task.id,
                dependencies=[test_task.id],
                priority=8,
            )

            # Simulate test failure
            await self.queue_service.fail_task(
                test_task.id,
                error_message="Integration tests failed: critical security vulnerability detected",
            )

            return {
                "workflow_name": "Production Deployment",
                "status": "FAILED",
                "failed_task_id": test_task.id,
                "impacted_tasks": [deploy_task.id],
            }

        except Exception as e:
            print(f"Workflow error: {e}")
            return {"error": str(e)}

    async def custom_priority_workflow(self) -> dict[str, Any]:
        """Demonstrate custom priority configuration.

        Shows how to use deadline and priority to influence task execution order.

        Returns:
            Dict with workflow priority configuration
        """
        # High-priority, urgent task with tight deadline
        security_patch_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Apply critical security patch",
            source=TaskSource.HUMAN,
            priority=10,  # Maximum priority
            deadline=datetime.now() + timedelta(hours=2),
            estimated_duration_seconds=3600,  # 1-hour estimated work
        )

        # Lower-priority maintenance task
        system_cleanup_task = await self.queue_service.submit_task(  # type: ignore
            prompt="Perform system log cleanup",
            source=TaskSource.AGENT_IMPLEMENTATION,
            priority=3,
            deadline=datetime.now() + timedelta(days=7),
        )

        return {
            "workflow_name": "Priority-Based Scheduling",
            "urgent_task": {
                "id": security_patch_task.id,
                "priority": security_patch_task.priority,
                "deadline": security_patch_task.deadline,
            },
            "background_task": {
                "id": system_cleanup_task.id,
                "priority": system_cleanup_task.priority,
                "deadline": system_cleanup_task.deadline,
            },
        }


async def run_examples():  # type: ignore
    """Execute all task queue system examples."""
    examples = TaskQueueExamples()  # type: ignore

    print("1. Linear Workflow Example:")
    linear_result = await examples.simple_linear_workflow()
    print(linear_result)

    print("\n2. Parallel Execution Example:")
    parallel_result = await examples.parallel_execution_example()
    print(parallel_result)

    print("\n3. Error Handling Workflow:")
    error_result = await examples.error_handling_workflow()
    print(error_result)

    print("\n4. Custom Priority Workflow:")
    priority_result = await examples.custom_priority_workflow()
    print(priority_result)


if __name__ == "__main__":
    asyncio.run(run_examples())  # type: ignore
