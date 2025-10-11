"""Unit tests for DependencyResolver service.

Tests cover:
- Circular dependency detection (simple, complex, transitive cycles)
- Topological sorting (linear, branching, diamond patterns)
- Dependency depth calculation (single level, multi-level, max depth)
- Edge cases (empty graph, single node, disconnected components)
- Graph caching and invalidation
- Error handling and validation
"""
