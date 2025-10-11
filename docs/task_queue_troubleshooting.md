# Task Queue System Troubleshooting Guide

## Overview

This guide provides solutions for common issues encountered when using the Abathur Task Queue System. It covers debugging techniques, performance tuning, and error resolution.

## Task Lifecycle Issues

### Stuck Tasks (BLOCKED Status)

**Problem:** Tasks remain in BLOCKED status indefinitely.

**Potential Causes:**
- Unresolved dependencies
- Circular dependencies
- Incomplete prerequisite tasks

**Diagnosis:**
```python
async def diagnose_blocked_tasks():
    queue_service = TaskQueueService()
    blocked_tasks = await queue_service.list_tasks(status=TaskStatus.BLOCKED)

    for task in blocked_tasks:
        dependencies = await queue_service.get_task_dependencies(task.id)
        unmet_deps = [dep for dep in dependencies if not dep.is_resolved]

        print(f"Task {task.id} blocked by:")
        for dep in unmet_deps:
            print(f"- Dependency {dep.prerequisite_task_id}: {dep.status}")
```

**Solutions:**
1. Check prerequisite tasks' status
2. Manually resolve dependencies
3. Use `cancel_task()` to break dependency chain
4. Investigate potential circular dependencies

### Circular Dependency Detection

**Problem:** Task submission fails due to circular dependencies.

**Prevention:**
```python
async def submit_task_safely(queue_service, task_details):
    try:
        return await queue_service.submit_task(**task_details)
    except CircularDependencyError as e:
        logging.error(f"Circular dependency detected: {e}")
        # Log dependency graph for investigation
        dependency_graph = await queue_service.get_dependency_chain(task_details['id'])
        visualization_service.export_graph(dependency_graph)
```

### Failed Task Cascade

**Problem:** Task failure causes unexpected task cancellations.

**Diagnosis & Mitigation:**
```python
async def handle_task_failure(queue_service, task_id, error):
    # 1. Log detailed error
    logging.error(f"Task {task_id} failed: {error}")

    # 2. Get impacted tasks
    impacted_tasks = await queue_service.fail_task(task_id, str(error))

    # 3. Notify or retry impacted tasks
    for impacted_task_id in impacted_tasks:
        # Custom retry or escalation logic
        await retry_or_escalate(impacted_task_id)
```

## Performance Issues

### Slow Task Enqueue/Dequeue

**Symptoms:**
- High latency for task submission
- Delays in task retrieval
- Degraded system responsiveness

**Diagnostic Tools:**
```python
async def measure_task_queue_performance():
    # Performance metrics collection
    start_time = time.time()

    # Enqueue performance
    enqueue_start = time.time()
    await queue_service.submit_task(prompt="Performance test task")
    enqueue_time = time.time() - enqueue_start

    # Dequeue performance
    dequeue_start = time.time()
    await queue_service.get_next_task()
    dequeue_time = time.time() - dequeue_start

    print(f"Enqueue time: {enqueue_time * 1000:.2f}ms")
    print(f"Dequeue time: {dequeue_time * 1000:.2f}ms")
```

**Optimization Strategies:**
1. Review database indexes
2. Batch task operations
3. Optimize priority calculation
4. Use connection pooling
5. Monitor system resources

### High CPU/Memory Usage

**Monitoring Script:**
```python
async def monitor_task_queue_resources():
    while True:
        queue_status = await queue_service.get_queue_status()

        # Resource utilization
        print("Active Tasks:", queue_status['active_tasks'])
        print("CPU Usage: {psutil.cpu_percent()}%")
        print("Memory Usage: {psutil.virtual_memory().percent}%")

        # Detect potential bottlenecks
        if queue_status['active_tasks'] > 1000:
            logging.warning("High task volume detected")

        await asyncio.sleep(60)  # Check every minute
```

## Logging and Debugging

### Comprehensive Logging Configuration
```python
import logging

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
    handlers=[
        logging.FileHandler('task_queue.log'),
        logging.StreamHandler()
    ]
)

# Task Queue specific logger
task_queue_logger = logging.getLogger('task_queue_service')
```

### Debug Logging for Task Operations
```python
class TaskQueueService:
    async def submit_task(self, **kwargs):
        task_queue_logger.info(f"Submitting task: {kwargs}")
        try:
            task = await self._submit_task(**kwargs)
            task_queue_logger.info(f"Task submitted successfully: {task.id}")
            return task
        except Exception as e:
            task_queue_logger.error(f"Task submission failed: {e}")
            raise
```

## Common Error Messages

### "CircularDependencyError"
- **Meaning:** Detected a dependency cycle
- **Action:** Break dependency chain, review task graph

### "TaskBlockedError"
- **Meaning:** Task cannot proceed due to unmet dependencies
- **Action:** Resolve prerequisite tasks

### "PriorityCalculationError"
- **Meaning:** Cannot compute task priority
- **Action:** Check task metadata, validate priority weights

## Configuration Troubleshooting

### Priority Weight Tuning
```python
PRIORITY_WEIGHTS = {
    "base_weight": 1.0,  # Adjust if base priority seems ineffective
    "urgency_weight": 2.0,  # Increase for more deadline sensitivity
    "dependency_weight": 1.5,  # Tune dependency impact
    "starvation_weight": 0.5,  # Adjust long-waiting task prioritization
    "source_weight": 1.0,  # Balance task source importance
}
```

## Advanced Diagnostics

### Dependency Graph Visualization
```python
async def visualize_task_dependencies(task_ids):
    dependency_resolver = DependencyResolver()
    graph = await dependency_resolver.get_dependency_graph(task_ids)

    # Export to GraphViz or Mermaid
    visualization_service.export_dependency_graph(graph)
```

## Emergency Procedures

### Force Task State Reset
```python
async def reset_task_state(task_id):
    # Use with caution - bypasses normal state transitions
    await queue_service.db.execute("""
        UPDATE tasks
        SET status = 'ready',
            calculated_priority = priority
        WHERE id = ?
    """, (task_id,))
```

## Best Practices

1. Implement comprehensive error handling
2. Use structured logging
3. Monitor system resources
4. Periodically review task dependencies
5. Set realistic timeouts
6. Implement graceful degradation

## Conclusion

Effective troubleshooting requires a systematic approach, combining logging, performance monitoring, and understanding the task queue's internal mechanics.
