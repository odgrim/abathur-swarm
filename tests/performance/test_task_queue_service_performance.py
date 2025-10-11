"""Performance tests for TaskQueueService.

Validates performance targets:
- Task enqueue: <10ms (including validation + priority calculation)
- Get next task: <5ms (single indexed query)
- Complete task: <50ms (including cascade for 10 dependents)
- Queue status: <20ms (aggregate queries)
- Enqueue throughput: >100 tasks/sec

Benchmarks use statistical sampling (100+ iterations) for reliable measurements.
"""
