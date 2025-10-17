"""Loop execution with iterative refinement and convergence detection."""

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from enum import Enum
from typing import Any
from uuid import UUID

from abathur.application.agent_executor import AgentExecutor
from abathur.application.task_coordinator import TaskCoordinator
from abathur.domain.models import Task, TaskStatus
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


class ConvergenceType(str, Enum):
    """Types of convergence criteria."""

    THRESHOLD = "threshold"
    STABILITY = "stability"
    TEST_PASS = "test_pass"
    CUSTOM = "custom"
    LLM_JUDGE = "llm_judge"
    MAX_ITERATIONS = "max_iterations"


@dataclass
class ConvergenceCriteria:
    """Convergence criteria configuration."""

    type: ConvergenceType
    # For THRESHOLD
    metric_name: str | None = None
    threshold: float | None = None
    direction: str = "minimize"  # or "maximize"
    # For STABILITY
    stability_window: int = 3
    similarity_threshold: float = 0.95
    # For TEST_PASS
    test_suite: str | None = None
    # For CUSTOM
    function_path: str | None = None
    # For LLM_JUDGE
    requirements: str | None = None


@dataclass
class ConvergenceEvaluation:
    """Result of convergence evaluation."""

    converged: bool
    score: float
    reason: str
    metadata: dict[str, Any] | None = None

    def __post_init__(self) -> None:
        if self.metadata is None:
            self.metadata = {}


@dataclass
class LoopResult:
    """Result of loop execution."""

    success: bool
    reason: str
    iterations: int
    result: Any
    converged: bool
    convergence_score: float | None = None
    history: list[Any] | None = None

    def __post_init__(self) -> None:
        if self.history is None:
            self.history = []


@dataclass
class LoopState:
    """State of loop execution for checkpointing."""

    task_id: UUID
    current_iteration: int
    max_iterations: int
    accumulated_results: list[dict[str, Any]]
    convergence_history: list[dict[str, Any]]
    agent_context: dict[str, Any]
    checkpoint_restored: bool = False


class LoopExecutor:
    """Executes iterative loops with convergence detection and checkpointing."""

    def __init__(
        self,
        task_coordinator: TaskCoordinator,
        agent_executor: AgentExecutor,
        database: Database,
    ):
        """Initialize loop executor.

        Args:
            task_coordinator: Task coordinator for task management
            agent_executor: Agent executor for task execution
            database: Database for state persistence
        """
        self.task_coordinator = task_coordinator
        self.agent_executor = agent_executor
        self.database = database

    async def execute_loop(
        self,
        task: Task,
        convergence_criteria: ConvergenceCriteria,
        max_iterations: int = 10,
        timeout: timedelta = timedelta(hours=1),
        checkpoint_interval: int = 1,
    ) -> LoopResult:
        """Execute iterative loop until convergence or termination condition.

        Termination conditions (OR):
            - Convergence criteria met
            - Max iterations reached
            - Timeout exceeded
            - User cancellation

        Args:
            task: Task to execute iteratively
            convergence_criteria: Criteria for convergence
            max_iterations: Maximum iterations (default: 10)
            timeout: Maximum execution time (default: 1 hour)
            checkpoint_interval: Save checkpoint every N iterations (default: 1)

        Returns:
            LoopResult with final output, iteration count, convergence status
        """
        iteration = 0
        start_time = datetime.now(timezone.utc)
        history: list[dict[str, Any]] = []

        # Attempt checkpoint restoration
        checkpoint_state = await self._try_restore_checkpoint(task.id)
        if checkpoint_state:
            iteration = checkpoint_state.current_iteration
            history = checkpoint_state.accumulated_results
            logger.info(
                "checkpoint_restored",
                task_id=str(task.id),
                iteration=iteration,
            )

        while iteration < max_iterations:
            # Check timeout
            elapsed_time = datetime.now(timezone.utc) - start_time
            if elapsed_time > timeout:
                logger.warning(
                    "loop_timeout",
                    task_id=str(task.id),
                    elapsed_seconds=elapsed_time.total_seconds(),
                )
                return LoopResult(
                    success=False,
                    reason="timeout",
                    iterations=iteration,
                    result=self._get_best_result(history),
                    converged=False,
                    history=history,
                )

            # Check cancellation
            if await self._is_task_cancelled(task.id):
                logger.info("loop_cancelled", task_id=str(task.id))
                return LoopResult(
                    success=False,
                    reason="cancelled",
                    iterations=iteration,
                    result=self._get_best_result(history),
                    converged=False,
                    history=history,
                )

            # Execute iteration
            iteration += 1
            logger.info(
                "loop_iteration_start",
                task_id=str(task.id),
                iteration=iteration,
                max_iterations=max_iterations,
            )

            # Build context from history
            context = self._build_iteration_context(history, convergence_criteria)

            # Execute task with context
            result = await self._execute_task_iteration(task, context, iteration)

            # Store result in history
            history.append(result)

            # Checkpoint state
            if iteration % checkpoint_interval == 0:
                await self._save_checkpoint(
                    task_id=task.id,
                    iteration=iteration,
                    state=LoopState(
                        task_id=task.id,
                        current_iteration=iteration,
                        max_iterations=max_iterations,
                        accumulated_results=history,
                        convergence_history=[],
                        agent_context={},
                    ),
                )

            # Evaluate convergence
            convergence_evaluation = await self._evaluate_convergence(
                result=result,
                criteria=convergence_criteria,
                history=history,
            )

            if convergence_evaluation.converged:
                logger.info(
                    "convergence_achieved",
                    task_id=str(task.id),
                    iteration=iteration,
                    score=convergence_evaluation.score,
                    reason=convergence_evaluation.reason,
                )

                return LoopResult(
                    success=True,
                    reason="converged",
                    iterations=iteration,
                    result=result,
                    converged=True,
                    convergence_score=convergence_evaluation.score,
                    history=history,
                )

            # Refine task for next iteration
            task = self._refine_task(task, result, convergence_evaluation)

        # Max iterations reached without convergence
        logger.warning(
            "max_iterations_reached",
            task_id=str(task.id),
            iterations=iteration,
        )
        return LoopResult(
            success=False,
            reason="max_iterations",
            iterations=iteration,
            result=self._get_best_result(history),
            converged=False,
            history=history,
        )

    async def _try_restore_checkpoint(self, task_id: UUID) -> LoopState | None:
        """Try to restore loop state from checkpoint.

        Args:
            task_id: Task ID to restore

        Returns:
            LoopState if checkpoint exists, None otherwise
        """
        try:
            # Query latest checkpoint
            async with self.database._get_connection() as conn:
                cursor = await conn.execute(
                    """
                    SELECT iteration, state, created_at
                    FROM checkpoints
                    WHERE task_id = ?
                    ORDER BY iteration DESC
                    LIMIT 1
                    """,
                    (str(task_id),),
                )
                row = await cursor.fetchone()

            if not row:
                return None

            import json

            state_data = json.loads(row[1])

            return LoopState(
                task_id=task_id,
                current_iteration=row[0],
                max_iterations=state_data.get("max_iterations", 10),
                accumulated_results=state_data.get("accumulated_results", []),
                convergence_history=state_data.get("convergence_history", []),
                agent_context=state_data.get("agent_context", {}),
                checkpoint_restored=True,
            )

        except Exception as e:
            logger.error("checkpoint_restore_error", error=str(e), task_id=str(task_id))
            return None

    async def _save_checkpoint(self, task_id: UUID, iteration: int, state: LoopState) -> None:
        """Save checkpoint to database.

        Args:
            task_id: Task ID
            iteration: Current iteration
            state: Loop state to save
        """
        try:
            import json

            state_json = json.dumps(
                {
                    "current_iteration": state.current_iteration,
                    "max_iterations": state.max_iterations,
                    "accumulated_results": state.accumulated_results,
                    "convergence_history": state.convergence_history,
                    "agent_context": state.agent_context,
                }
            )

            async with self.database._get_connection() as conn:
                await conn.execute(
                    """
                    INSERT OR REPLACE INTO checkpoints (task_id, iteration, state, created_at)
                    VALUES (?, ?, ?, ?)
                    """,
                    (str(task_id), iteration, state_json, datetime.now(timezone.utc)),
                )
                await conn.commit()

            logger.info(
                "checkpoint_saved",
                task_id=str(task_id),
                iteration=iteration,
            )

        except Exception as e:
            logger.error("checkpoint_save_error", error=str(e), task_id=str(task_id))

    async def _is_task_cancelled(self, task_id: UUID) -> bool:
        """Check if task has been cancelled.

        Args:
            task_id: Task ID to check

        Returns:
            True if task is cancelled
        """
        task = await self.task_coordinator.get_task(task_id)
        return task is not None and task.status == TaskStatus.CANCELLED

    def _build_iteration_context(
        self,
        history: list[dict[str, Any]],
        convergence_criteria: ConvergenceCriteria,
    ) -> dict[str, Any]:
        """Build context for iteration from history.

        Args:
            history: List of previous results
            convergence_criteria: Convergence criteria

        Returns:
            Context dictionary for agent
        """
        return {
            "iteration": len(history) + 1,
            "previous_results": history[-3:] if history else [],  # Last 3 results
            "convergence_criteria": {
                "type": convergence_criteria.type.value,
                "target": convergence_criteria.threshold
                if convergence_criteria.threshold
                else None,
            },
        }

    async def _execute_task_iteration(
        self, task: Task, context: dict[str, Any], iteration: int
    ) -> dict[str, Any]:
        """Execute single iteration of task.

        Args:
            task: Task to execute
            context: Iteration context
            iteration: Current iteration number

        Returns:
            Result dictionary
        """
        # Execute task via agent executor
        result = await self.agent_executor.execute_task(task)

        return {
            "iteration": iteration,
            "output": result.data.get("output") if result.success and result.data else None,
            "success": result.success,
            "error": result.error if not result.success else None,
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "metrics": result.metadata or {},
        }

    async def _evaluate_convergence(
        self,
        result: dict[str, Any],
        criteria: ConvergenceCriteria,
        history: list[dict[str, Any]],
    ) -> ConvergenceEvaluation:
        """Evaluate convergence based on criteria.

        Args:
            result: Current iteration result
            criteria: Convergence criteria
            history: Full iteration history

        Returns:
            ConvergenceEvaluation with convergence status
        """
        if criteria.type == ConvergenceType.THRESHOLD:
            return self._evaluate_threshold(result, criteria)

        elif criteria.type == ConvergenceType.STABILITY:
            return self._evaluate_stability(history, criteria)

        elif criteria.type == ConvergenceType.MAX_ITERATIONS:
            # This is handled by main loop
            return ConvergenceEvaluation(
                converged=False, score=0.0, reason="Max iterations not reached"
            )

        else:
            # Not yet implemented convergence types
            logger.warning("unsupported_convergence_type", type=criteria.type.value)
            return ConvergenceEvaluation(
                converged=False, score=0.0, reason=f"Unsupported type: {criteria.type}"
            )

    def _evaluate_threshold(
        self, result: dict[str, Any], criteria: ConvergenceCriteria
    ) -> ConvergenceEvaluation:
        """Evaluate threshold-based convergence.

        Args:
            result: Current result
            criteria: Convergence criteria

        Returns:
            ConvergenceEvaluation
        """
        if not criteria.metric_name or criteria.threshold is None:
            return ConvergenceEvaluation(
                converged=False, score=0.0, reason="Missing metric_name or threshold"
            )

        # Extract metric from result
        metric_value = result.get("metrics", {}).get(criteria.metric_name)

        if metric_value is None:
            return ConvergenceEvaluation(
                converged=False,
                score=0.0,
                reason=f"Metric {criteria.metric_name} not found",
            )

        # Check against threshold
        if criteria.direction == "minimize":
            converged = metric_value <= criteria.threshold
        else:  # maximize
            converged = metric_value >= criteria.threshold

        return ConvergenceEvaluation(
            converged=converged,
            score=float(metric_value),
            reason=f"Metric {criteria.metric_name} = {metric_value}",
        )

    def _evaluate_stability(
        self, history: list[dict[str, Any]], criteria: ConvergenceCriteria
    ) -> ConvergenceEvaluation:
        """Evaluate stability-based convergence.

        Args:
            history: Full iteration history
            criteria: Convergence criteria

        Returns:
            ConvergenceEvaluation
        """
        if len(history) < criteria.stability_window:
            return ConvergenceEvaluation(
                converged=False,
                score=0.0,
                reason="Insufficient history for stability check",
            )

        recent_results = history[-criteria.stability_window :]

        # Compute similarity (simple implementation: check if outputs are identical)
        outputs = [r.get("output") for r in recent_results]
        if all(o == outputs[0] for o in outputs):
            similarity = 1.0
        else:
            # Simple string similarity
            similarity = self._compute_similarity(outputs)

        converged = similarity >= criteria.similarity_threshold

        return ConvergenceEvaluation(
            converged=converged,
            score=similarity,
            reason=f"Stability score: {similarity:.3f}",
        )

    def _compute_similarity(self, outputs: list[Any]) -> float:
        """Compute similarity between outputs.

        Args:
            outputs: List of outputs to compare

        Returns:
            Similarity score (0.0-1.0)
        """
        if not outputs:
            return 0.0

        # Simple implementation: ratio of identical outputs
        first = outputs[0]
        identical_count = sum(1 for o in outputs if o == first)
        return identical_count / len(outputs)

    def _refine_task(
        self,
        task: Task,
        result: dict[str, Any],
        convergence_evaluation: ConvergenceEvaluation,
    ) -> Task:
        """Refine task for next iteration based on result.

        Args:
            task: Current task
            result: Current iteration result
            convergence_evaluation: Convergence evaluation

        Returns:
            Refined task for next iteration
        """
        # Simple refinement: add feedback to input data
        refined_input = task.input_data.copy()
        refined_input["previous_result"] = result.get("output")
        refined_input["convergence_feedback"] = convergence_evaluation.reason

        # Create new task with refined input
        refined_task = Task(
            prompt=task.prompt,
            agent_type=task.agent_type,
            input_data=refined_input,
            priority=task.priority,
            max_retries=task.max_retries,
            summary=task.summary,
        )

        return refined_task

    def _get_best_result(self, history: list[dict[str, Any]]) -> dict | None:
        """Get best result from history.

        Args:
            history: List of iteration results

        Returns:
            Best result or None if history is empty
        """
        if not history:
            return None

        # Simple heuristic: return last successful result
        for result in reversed(history):
            if result.get("success"):
                return result

        # If no successful results, return last result
        return history[-1] if history else None
