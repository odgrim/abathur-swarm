"""Unit tests for PriorityCalculator service.

Tests cover:
- Base priority calculation
- Depth score calculation (linear scaling)
- Urgency score calculation (exponential decay)
- Blocking score calculation (logarithmic scaling)
- Source score calculation (fixed mapping)
- Integration tests with DependencyResolver
- Batch recalculation
- Edge cases and error handling
"""

import math
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import UUID, uuid4

import pytest
from abathur.domain.models import (
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator


@pytest.fixture
async def db():
    """In-memory database for testing."""
    database = Database(db_path=Path(":memory:"))
    await database.initialize()
    yield database
    await database.close()


@pytest.fixture
async def resolver(db):
    """DependencyResolver instance."""
    return DependencyResolver(db, cache_ttl_seconds=60.0)


@pytest.fixture
async def calculator(resolver):
    """PriorityCalculator instance with default weights."""
    return PriorityCalculator(resolver)


@pytest.fixture
async def calculator_custom_weights(resolver):
    """PriorityCalculator with custom weights for testing."""
    return PriorityCalculator(
        resolver,
        base_weight=0.4,
        depth_weight=0.2,
        urgency_weight=0.2,
        blocking_weight=0.1,
        source_weight=0.1,
    )


def create_task(
    priority: int = 5,
    status: TaskStatus = TaskStatus.READY,
    source: TaskSource = TaskSource.HUMAN,
    deadline: datetime | None = None,
    estimated_duration: int | None = None,
    task_id: UUID | None = None,
) -> Task:
    """Create test task with specified attributes."""
    return Task(
        id=task_id or uuid4(),
        prompt="Test task",
        priority=priority,
        status=status,
        source=source,
        deadline=deadline,
        estimated_duration_seconds=estimated_duration,
    )


async def create_dependency_chain(db: Database, depth: int) -> list[Task]:
    """Create linear dependency chain of specified depth."""
    tasks = []
    for i in range(depth + 1):  # depth+1 tasks create depth levels
        task = create_task()
        await db.insert_task(task)
        tasks.append(task)

        if i > 0:
            dep = TaskDependency(
                dependent_task_id=task.id,
                prerequisite_task_id=tasks[i - 1].id,
                dependency_type=DependencyType.SEQUENTIAL,
            )
            await db.insert_task_dependency(dep)

    return tasks


# =============================================================================
# Base Priority Tests
# =============================================================================


@pytest.mark.asyncio
async def test_calculate_priority_base_only(calculator):
    """Test priority calculation with only base priority set."""
    task = create_task(priority=5)  # Middle priority
    priority = await calculator.calculate_priority(task)

    # With default weights: base=0.30, others contribute to neutral 50
    # base_score = 5 * 10 = 50 -> 50 * 0.30 = 15
    # depth = 0 -> 0 * 0.25 = 0
    # urgency (no deadline) = 50 -> 50 * 0.25 = 12.5
    # blocking = 0 -> 0 * 0.15 = 0
    # source (HUMAN) = 100 -> 100 * 0.05 = 5
    # Total = 15 + 0 + 12.5 + 0 + 5 = 32.5
    assert 30 < priority < 35, f"Expected ~32.5, got {priority}"


@pytest.mark.asyncio
async def test_calculate_priority_base_scaling(calculator):
    """Test base priority scales from 0-10 to 0-100."""
    # Priority 0 (minimum)
    task_min = create_task(priority=0)
    priority_min = await calculator.calculate_priority(task_min)

    # Priority 10 (maximum)
    task_max = create_task(priority=10)
    priority_max = await calculator.calculate_priority(task_max)

    # Higher base priority should result in higher overall priority
    assert priority_max > priority_min
    # Base priority contributes 30% of total
    # Min: 0*10*0.30 = 0, Max: 10*10*0.30 = 30
    # Difference should be ~30 points (from base alone)
    assert 25 < (priority_max - priority_min) < 35


@pytest.mark.asyncio
async def test_calculate_priority_clamping(calculator):
    """Test priority is clamped to [0, 100] range."""
    # Maximum possible priority: all factors at 100
    task = create_task(
        priority=10,  # 100 score
        source=TaskSource.HUMAN,  # 100 score
        deadline=datetime.now(timezone.utc) - timedelta(hours=1),  # Past deadline: 100
    )
    priority = await calculator.calculate_priority(task)

    assert 0 <= priority <= 100, f"Priority {priority} not in [0, 100]"
    assert priority <= 100.0, "Priority should be clamped at 100"


# =============================================================================
# Depth Score Tests
# =============================================================================


@pytest.mark.asyncio
async def test_depth_score_root_task(db, calculator):
    """Test depth score for root task (no dependencies)."""
    task = create_task()
    await db.insert_task(task)

    priority = await calculator.calculate_priority(task)

    # Root task depth = 0 -> depth_score = 0
    # Depth contributes 25% weight, so 0 points from depth
    # Total should be < 50 (no depth boost)
    assert priority < 50, f"Root task should have priority < 50, got {priority}"


@pytest.mark.asyncio
async def test_depth_score_linear_scaling(db, calculator):
    """Test depth score scales linearly: depth=5 -> score=50."""
    # Create 5-level dependency chain: [0] -> [1] -> [2] -> [3] -> [4] -> [5]
    tasks = await create_dependency_chain(db, depth=5)

    # Task at depth 5 (deepest task)
    deep_task = tasks[-1]
    priority = await calculator.calculate_priority(deep_task)

    # Depth 5 -> score = 50 -> 50 * 0.25 = 12.5 points from depth
    # Should be noticeably higher than root task
    root_task = tasks[0]
    root_priority = await calculator.calculate_priority(root_task)

    assert priority > root_priority, "Deeper task should have higher priority"
    # Depth difference: 5 levels * 10 points * 0.25 weight = 12.5 points
    assert 10 < (priority - root_priority) < 15


@pytest.mark.asyncio
async def test_depth_score_max_capping(db, calculator):
    """Test depth score is capped at 100 (depth 10+)."""
    # Create 15-level dependency chain (exceeds cap)
    tasks = await create_dependency_chain(db, depth=15)

    # Task at depth 15 should have capped depth score
    deep_task = tasks[-1]
    priority = await calculator.calculate_priority(deep_task)

    # Depth 15 -> score = min(150, 100) = 100 -> 100 * 0.25 = 25 points
    # Maximum contribution from depth is 25 points
    # Even with other factors, should see depth contribution capped
    assert priority > 40, "Deep task with capped depth should have high priority"


# =============================================================================
# Urgency Score Tests
# =============================================================================


@pytest.mark.asyncio
async def test_urgency_score_no_deadline(calculator):
    """Test urgency score for task with no deadline."""
    task = create_task(deadline=None)
    priority = await calculator.calculate_priority(task)

    # No deadline -> urgency = 50 (neutral) -> 50 * 0.25 = 12.5
    # Should be in neutral range
    assert 25 < priority < 45, f"Expected neutral priority ~32.5, got {priority}"


@pytest.mark.asyncio
async def test_urgency_score_past_deadline(calculator):
    """Test urgency score for task past deadline."""
    past_deadline = datetime.now(timezone.utc) - timedelta(hours=1)
    task = create_task(deadline=past_deadline)
    priority = await calculator.calculate_priority(task)

    # Past deadline -> urgency = 100 -> 100 * 0.25 = 25 points
    # Should have high priority due to urgency
    assert priority > 40, f"Past deadline task should have priority > 40, got {priority}"


@pytest.mark.asyncio
async def test_urgency_score_one_minute(calculator):
    """Test urgency score for deadline < 1 minute away."""
    near_deadline = datetime.now(timezone.utc) + timedelta(seconds=30)
    task = create_task(deadline=near_deadline)
    priority = await calculator.calculate_priority(task)

    # < 1 minute -> urgency = 100 -> 100 * 0.25 = 25 points
    assert priority > 40, f"Imminent deadline should have priority > 40, got {priority}"


@pytest.mark.asyncio
async def test_urgency_score_one_hour(calculator):
    """Test urgency score for deadline < 1 hour away."""
    one_hour = datetime.now(timezone.utc) + timedelta(minutes=30)
    task = create_task(deadline=one_hour)
    priority = await calculator.calculate_priority(task)

    # < 1 hour -> urgency = 80 -> 80 * 0.25 = 20 points
    assert priority > 35, f"1-hour deadline should have priority > 35, got {priority}"


@pytest.mark.asyncio
async def test_urgency_score_one_day(calculator):
    """Test urgency score for deadline < 1 day away."""
    one_day = datetime.now(timezone.utc) + timedelta(hours=12)
    task = create_task(deadline=one_day)
    priority = await calculator.calculate_priority(task)

    # < 1 day -> urgency = 50 -> 50 * 0.25 = 12.5 points
    # Should be moderate priority
    assert 25 < priority < 45


@pytest.mark.asyncio
async def test_urgency_score_one_week(calculator):
    """Test urgency score for deadline < 1 week away."""
    one_week = datetime.now(timezone.utc) + timedelta(days=3)
    task = create_task(deadline=one_week)
    priority = await calculator.calculate_priority(task)

    # < 1 week -> urgency = 30 -> 30 * 0.25 = 7.5 points
    # Should be lower priority
    assert priority < 40


@pytest.mark.asyncio
async def test_urgency_score_insufficient_time(calculator):
    """Test urgency when time_remaining < estimated_duration."""
    # Deadline in 1 hour but task takes 2 hours
    deadline = datetime.now(timezone.utc) + timedelta(hours=1)
    task = create_task(deadline=deadline, estimated_duration=7200)  # 2 hours

    priority = await calculator.calculate_priority(task)

    # Insufficient time -> urgency = 100 -> 100 * 0.25 = 25 points
    assert priority > 40, "Insufficient time should result in high priority"


@pytest.mark.asyncio
async def test_urgency_score_exponential_decay(calculator):
    """Test exponential urgency decay with estimated duration."""
    estimated_duration = 3600  # 1 hour

    # Task with plenty of time (10x duration)
    far_deadline = datetime.now(timezone.utc) + timedelta(hours=10)
    task_far = create_task(deadline=far_deadline, estimated_duration=estimated_duration)
    priority_far = await calculator.calculate_priority(task_far)

    # Task with approaching deadline (2x duration)
    near_deadline = datetime.now(timezone.utc) + timedelta(hours=2)
    task_near = create_task(deadline=near_deadline, estimated_duration=estimated_duration)
    priority_near = await calculator.calculate_priority(task_near)

    # Nearer deadline should have higher priority
    assert priority_near > priority_far, "Nearer deadline should increase urgency"


# =============================================================================
# Blocking Score Tests
# =============================================================================


@pytest.mark.asyncio
async def test_blocking_score_no_blocked_tasks(db, calculator):
    """Test blocking score when no tasks are blocked."""
    task = create_task()
    await db.insert_task(task)

    priority = await calculator.calculate_priority(task)

    # No blocked tasks -> blocking = 0 -> 0 * 0.15 = 0 points
    # Should have baseline priority
    assert priority < 50


@pytest.mark.asyncio
async def test_blocking_score_1_blocked(db, calculator):
    """Test blocking score with 1 blocked task."""
    prerequisite = create_task()
    await db.insert_task(prerequisite)

    # Create blocked task
    blocked = create_task()
    await db.insert_task(blocked)
    dep = TaskDependency(
        dependent_task_id=blocked.id,
        prerequisite_task_id=prerequisite.id,
        dependency_type=DependencyType.SEQUENTIAL,
    )
    await db.insert_task_dependency(dep)

    priority = await calculator.calculate_priority(prerequisite)

    # 1 blocked -> log10(2) * 33.33 = 10 -> 10 * 0.15 = 1.5 points
    # Should be slightly higher than unblocking task
    unblocking_task = create_task()
    await db.insert_task(unblocking_task)
    priority_unblocking = await calculator.calculate_priority(unblocking_task)

    assert priority > priority_unblocking, "Blocking task should have higher priority"


@pytest.mark.asyncio
async def test_blocking_score_logarithmic_scaling(db, calculator):
    """Test blocking score scales logarithmically."""
    prerequisite = create_task()
    await db.insert_task(prerequisite)

    # Create 10 blocked tasks
    for _ in range(10):
        blocked = create_task()
        await db.insert_task(blocked)
        dep = TaskDependency(
            dependent_task_id=blocked.id,
            prerequisite_task_id=prerequisite.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

    priority = await calculator.calculate_priority(prerequisite)

    # 10 blocked -> log10(11) * 33.33 = 34.7 -> 34.7 * 0.15 = 5.2 points
    # Should have noticeably higher priority
    assert priority > 35, f"Task blocking 10 tasks should have priority > 35, got {priority}"


@pytest.mark.asyncio
async def test_blocking_score_many_blocked(db, calculator):
    """Test blocking score with many blocked tasks (100+)."""
    prerequisite = create_task()
    await db.insert_task(prerequisite)

    # Create 100 blocked tasks
    for _ in range(100):
        blocked = create_task()
        await db.insert_task(blocked)
        dep = TaskDependency(
            dependent_task_id=blocked.id,
            prerequisite_task_id=prerequisite.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

    priority = await calculator.calculate_priority(prerequisite)

    # 100 blocked -> log10(101) * 33.33 = 66.9 -> 66.9 * 0.15 = 10 points
    # Should have high priority
    assert priority > 40, f"Task blocking 100 tasks should have priority > 40, got {priority}"


# =============================================================================
# Source Score Tests
# =============================================================================


@pytest.mark.asyncio
async def test_source_score_human(calculator):
    """Test source score for HUMAN tasks."""
    task = create_task(source=TaskSource.HUMAN)
    priority = await calculator.calculate_priority(task)

    # HUMAN -> 100 * 0.05 = 5 points
    # Should be baseline + human boost
    assert priority > 30


@pytest.mark.asyncio
async def test_source_score_agent_requirements(calculator):
    """Test source score for AGENT_REQUIREMENTS tasks."""
    task_human = create_task(source=TaskSource.HUMAN)
    task_agent = create_task(source=TaskSource.AGENT_REQUIREMENTS)

    priority_human = await calculator.calculate_priority(task_human)
    priority_agent = await calculator.calculate_priority(task_agent)

    # AGENT_REQUIREMENTS -> 75 * 0.05 = 3.75 points
    # Should be slightly lower than HUMAN
    assert priority_human > priority_agent


@pytest.mark.asyncio
async def test_source_score_agent_planner(calculator):
    """Test source score for AGENT_PLANNER tasks."""
    task = create_task(source=TaskSource.AGENT_PLANNER)
    priority = await calculator.calculate_priority(task)

    # AGENT_PLANNER -> 50 * 0.05 = 2.5 points
    # Should be moderate
    assert 25 < priority < 40


@pytest.mark.asyncio
async def test_source_score_agent_implementation(calculator):
    """Test source score for AGENT_IMPLEMENTATION tasks."""
    task = create_task(source=TaskSource.AGENT_IMPLEMENTATION)
    priority = await calculator.calculate_priority(task)

    # AGENT_IMPLEMENTATION -> 25 * 0.05 = 1.25 points
    # Should be lowest source priority
    assert priority < 35


# =============================================================================
# Integration Tests
# =============================================================================


@pytest.mark.asyncio
async def test_calculate_priority_all_factors(db, calculator):
    """Test priority calculation with all factors combined."""
    # Create dependency chain
    tasks = await create_dependency_chain(db, depth=3)
    prerequisite = tasks[-1]  # Deep task

    # Create blocked tasks
    for _ in range(5):
        blocked = create_task()
        await db.insert_task(blocked)
        dep = TaskDependency(
            dependent_task_id=blocked.id,
            prerequisite_task_id=prerequisite.id,
            dependency_type=DependencyType.SEQUENTIAL,
        )
        await db.insert_task_dependency(dep)

    # Set deadline and high priority
    deadline = datetime.now(timezone.utc) + timedelta(hours=1)
    prerequisite.deadline = deadline
    prerequisite.priority = 8
    prerequisite.source = TaskSource.HUMAN

    priority = await calculator.calculate_priority(prerequisite)

    # Should have high priority from all factors:
    # - High base priority (8 -> 80 * 0.30 = 24)
    # - Depth 3 (30 * 0.25 = 7.5)
    # - Urgent deadline (80+ * 0.25 = 20+)
    # - Blocking 5 tasks (~23 * 0.15 = 3.5)
    # - Human source (100 * 0.05 = 5)
    # Total ~60+
    assert priority > 55, f"All factors combined should give priority > 55, got {priority}"


@pytest.mark.asyncio
async def test_calculate_priority_weighted_sum(calculator_custom_weights):
    """Test weighted sum with custom weights."""
    task = create_task(priority=10)  # Max base priority
    priority = await calculator_custom_weights.calculate_priority(task)

    # Custom weights: base=0.4, others sum to 0.6
    # Base: 100 * 0.4 = 40
    # Should be higher than default (100 * 0.3 = 30)
    assert priority > 50, "Custom base weight should increase priority"


@pytest.mark.asyncio
async def test_recalculate_priorities_batch(db, calculator):
    """Test batch priority recalculation."""
    # Create 5 tasks
    task_ids = []
    for i in range(5):
        task = create_task(priority=i)
        await db.insert_task(task)
        task_ids.append(task.id)

    # Recalculate priorities
    results = await calculator.recalculate_priorities(task_ids, db)

    # Should return priorities for all tasks
    assert len(results) == 5
    for task_id in task_ids:
        assert task_id in results
        assert 0 <= results[task_id] <= 100


@pytest.mark.asyncio
async def test_recalculate_priorities_filters_status(db, calculator):
    """Test recalculation only affects PENDING/BLOCKED/READY tasks."""
    # Create tasks in different statuses
    task_ready = create_task(status=TaskStatus.READY)
    task_running = create_task(status=TaskStatus.RUNNING)
    task_completed = create_task(status=TaskStatus.COMPLETED)

    await db.insert_task(task_ready)
    await db.insert_task(task_running)
    await db.insert_task(task_completed)

    task_ids = [task_ready.id, task_running.id, task_completed.id]
    results = await calculator.recalculate_priorities(task_ids, db)

    # Only READY task should be recalculated
    assert task_ready.id in results
    assert task_running.id not in results
    assert task_completed.id not in results


# =============================================================================
# Edge Cases and Error Handling
# =============================================================================


@pytest.mark.asyncio
async def test_handle_missing_task(db, calculator):
    """Test graceful handling of non-existent task ID."""
    fake_id = uuid4()
    results = await calculator.recalculate_priorities([fake_id], db)

    # Should skip missing task, return empty results
    assert len(results) == 0


@pytest.mark.asyncio
async def test_handle_none_values(calculator):
    """Test handling of tasks with None optional fields."""
    task = Task(
        prompt="Test",
        priority=5,
        deadline=None,
        estimated_duration_seconds=None,
    )
    priority = await calculator.calculate_priority(task)

    # Should calculate successfully with defaults
    assert 0 <= priority <= 100


@pytest.mark.asyncio
async def test_weight_validation():
    """Test weight validation in constructor."""
    from pathlib import Path

    from abathur.services.dependency_resolver import DependencyResolver

    db = Database(Path(":memory:"))
    await db.initialize()
    resolver = DependencyResolver(db)

    # Weights that don't sum to 1.0
    with pytest.raises(ValueError, match="must sum to 1.0"):
        PriorityCalculator(
            resolver,
            base_weight=0.3,
            depth_weight=0.3,
            urgency_weight=0.3,
            blocking_weight=0.3,  # Sum = 1.2
            source_weight=0.0,
        )

    await db.close()


@pytest.mark.asyncio
async def test_priority_weights_sum_to_one(calculator):
    """Test that default weights sum to 1.0."""
    total = (
        calculator._base_weight
        + calculator._depth_weight
        + calculator._urgency_weight
        + calculator._blocking_weight
        + calculator._source_weight
    )
    assert math.isclose(total, 1.0, rel_tol=1e-6), f"Weights sum to {total}, expected 1.0"


@pytest.mark.asyncio
async def test_error_recovery_in_calculation(db, resolver):
    """Test error recovery when depth calculation fails."""
    # Create calculator with resolver
    calculator = PriorityCalculator(resolver)

    # Create task but don't insert it (will cause depth calculation to potentially fail)
    task = create_task()

    # Should return neutral priority (50.0) on error
    priority = await calculator.calculate_priority(task)
    assert priority == 50.0, "Should return neutral priority on error"
