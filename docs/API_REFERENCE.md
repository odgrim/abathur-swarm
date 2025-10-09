# Abathur API Reference

## Core Components

### Task Coordinator

Manages task lifecycle and priority queue.

```python
from abathur.application import TaskCoordinator
from abathur.domain.models import Task, TaskStatus

# Initialize
task_coordinator = TaskCoordinator(database)

# Submit task
task = Task(template_name="agent", input_data={}, priority=8)
task_id = await task_coordinator.submit_task(task)

# Get task
task = await task_coordinator.get_task(task_id)

# List tasks
tasks = await task_coordinator.list_tasks(TaskStatus.PENDING, limit=100)

# Cancel task
success = await task_coordinator.cancel_task(task_id)

# Retry task
success = await task_coordinator.retry_task(task_id)
```

### Swarm Orchestrator

Coordinates concurrent agent execution.

```python
from abathur.application import SwarmOrchestrator

# Initialize
swarm = SwarmOrchestrator(
    task_coordinator=task_coordinator,
    agent_executor=agent_executor,
    max_concurrent_agents=10
)

# Start swarm
results = await swarm.start_swarm(task_limit=None)

# Execute batch
tasks = [Task(...), Task(...)]
results = await swarm.execute_batch(tasks)

# Get status
status = await swarm.get_swarm_status()

# Shutdown
await swarm.shutdown()
```

### Loop Executor

Executes iterative refinement loops.

```python
from abathur.application import LoopExecutor, ConvergenceCriteria, ConvergenceType

# Initialize
loop_executor = LoopExecutor(task_coordinator, agent_executor, database)

# Define convergence criteria
criteria = ConvergenceCriteria(
    type=ConvergenceType.THRESHOLD,
    metric_name="accuracy",
    threshold=0.95,
    direction="maximize"
)

# Execute loop
result = await loop_executor.execute_loop(
    task=task,
    convergence_criteria=criteria,
    max_iterations=10,
    timeout=timedelta(hours=1),
    checkpoint_interval=1
)

# Check result
if result.converged:
    print(f"Converged after {result.iterations} iterations")
```

### MCP Manager

Manages MCP server lifecycle.

```python
from abathur.application import MCPManager

# Initialize
mcp_manager = MCPManager()
await mcp_manager.initialize()

# Start server
success = await mcp_manager.start_server("filesystem")

# Stop server
success = await mcp_manager.stop_server("filesystem")

# Get status
status = mcp_manager.get_server_status("filesystem")

# Bind agent to servers
results = mcp_manager.bind_agent_to_servers(agent_id, ["filesystem", "github"])

# Shutdown
await mcp_manager.shutdown()
```

### Failure Recovery

Manages failure detection and recovery.

```python
from abathur.application import FailureRecovery, RetryPolicy

# Initialize with custom retry policy
retry_policy = RetryPolicy(
    max_retries=3,
    initial_backoff_seconds=10.0,
    max_backoff_seconds=300.0
)

failure_recovery = FailureRecovery(
    task_coordinator=task_coordinator,
    database=database,
    retry_policy=retry_policy
)

# Start monitoring
await failure_recovery.start_recovery_monitor(check_interval=60.0)

# Get statistics
stats = failure_recovery.get_stats()

# Get DLQ tasks
dlq_tasks = failure_recovery.get_dlq_tasks()

# Reprocess from DLQ
success = await failure_recovery.reprocess_dlq_task(task_id)

# Stop monitoring
await failure_recovery.stop_recovery_monitor()
```

### Resource Monitor

Monitors system and agent resource usage.

```python
from abathur.application import ResourceMonitor, ResourceLimits

# Initialize with custom limits
limits = ResourceLimits(
    max_memory_per_agent_mb=512,
    max_total_memory_mb=4096,
    max_cpu_percent=80.0
)

resource_monitor = ResourceMonitor(limits=limits)

# Start monitoring
await resource_monitor.start_monitoring()

# Get snapshot
snapshot = await resource_monitor.get_snapshot(agent_count=5)

# Check if can spawn agent
can_spawn = resource_monitor.can_spawn_agent(current_agent_count=5)

# Get statistics
stats = resource_monitor.get_stats()

# Stop monitoring
await resource_monitor.stop_monitoring()
```

## Domain Models

### Task

```python
from abathur.domain.models import Task, TaskStatus

task = Task(
    id=uuid4(),  # Auto-generated if not provided
    template_name="analyzer",
    priority=8,  # 0-10 scale
    status=TaskStatus.PENDING,
    input_data={"prompt": "Analyze this code"},
    result_data=None,
    error_message=None,
    retry_count=0,
    max_retries=3,
    submitted_at=datetime.now(UTC),
    started_at=None,
    completed_at=None,
    created_by="user",
    parent_task_id=None,
    dependencies=[]
)
```

### Agent

```python
from abathur.domain.models import Agent, AgentState

agent = Agent(
    id=uuid4(),
    name="analyzer-1",
    specialization="code_analysis",
    task_id=task_id,
    state=AgentState.IDLE,
    model="claude-sonnet-4",
    spawned_at=datetime.now(UTC),
    terminated_at=None,
    resource_usage={}
)
```

### Result

```python
from abathur.domain.models import Result

result = Result(
    id=uuid4(),
    task_id=task_id,
    agent_id=agent_id,
    success=True,
    output="Analysis complete",
    error_message=None,
    started_at=datetime.now(UTC),
    completed_at=datetime.now(UTC),
    metadata={"score": 0.95}
)
```

## Complete Example

```python
import asyncio
from datetime import timedelta
from abathur.infrastructure import ConfigManager, Database
from abathur.application import (
    TaskCoordinator,
    ClaudeClient,
    AgentExecutor,
    SwarmOrchestrator,
    FailureRecovery,
    ResourceMonitor,
    LoopExecutor,
    ConvergenceCriteria,
    ConvergenceType
)
from abathur.domain.models import Task

async def main():
    # Setup infrastructure
    config_manager = ConfigManager()
    database = Database(config_manager.get_database_path())
    await database.initialize()

    # Setup services
    task_coordinator = TaskCoordinator(database)
    claude_client = ClaudeClient(api_key=config_manager.get_api_key())
    agent_executor = AgentExecutor(database, claude_client)

    # Setup swarm orchestrator
    swarm = SwarmOrchestrator(
        task_coordinator=task_coordinator,
        agent_executor=agent_executor,
        max_concurrent_agents=10
    )

    # Setup monitoring
    resource_monitor = ResourceMonitor()
    await resource_monitor.start_monitoring()

    failure_recovery = FailureRecovery(task_coordinator, database)
    await failure_recovery.start_recovery_monitor()

    # Submit tasks
    tasks = [
        Task(template_name="analyzer", input_data={"code": "..."}, priority=8),
        Task(template_name="tester", input_data={"test": "..."}, priority=9),
    ]

    for task in tasks:
        await task_coordinator.submit_task(task)

    # Execute with swarm
    results = await swarm.start_swarm(task_limit=10)

    # Or use loop execution for iterative refinement
    loop_executor = LoopExecutor(task_coordinator, agent_executor, database)
    criteria = ConvergenceCriteria(
        type=ConvergenceType.THRESHOLD,
        metric_name="accuracy",
        threshold=0.95
    )

    loop_result = await loop_executor.execute_loop(
        tasks[0], criteria, max_iterations=10
    )

    # Get statistics
    print(f"Resource stats: {resource_monitor.get_stats()}")
    print(f"Failure stats: {failure_recovery.get_stats()}")

    # Cleanup
    await swarm.shutdown()
    await resource_monitor.stop_monitoring()
    await failure_recovery.stop_recovery_monitor()

if __name__ == "__main__":
    asyncio.run(main())
```

---

**Version:** 0.1.0
**Last Updated:** 2025-10-09
