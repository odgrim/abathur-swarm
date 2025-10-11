"""Unit tests for TaskQueueService.

Tests cover:
- Task enqueue (basic, with dependencies, circular detection, priority calculation)
- Get next task (priority ordering, no ready tasks, FIFO tiebreaker)
- Complete task (unblock dependents, recalculate priorities, state transitions)
- Fail task (cascade cancellation, error message)
- Cancel task (dependent cancellation)
- Queue status (statistics calculation)
- Execution plan (topological sort)
- Edge cases and error handling
"""
