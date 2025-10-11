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
