"""Performance tests for PriorityCalculator service.

Tests performance targets:
- Single priority calculation: <5ms
- Batch calculation (100 tasks): <50ms
- 10-level cascade recalculation: <100ms
"""
