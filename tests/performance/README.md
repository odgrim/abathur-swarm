# Performance Tests for Task Queue MCP Server

This directory contains performance and load tests for the Task Queue MCP server.

## Performance Targets

Based on requirements specification:

| Operation | Target Latency | Notes |
|-----------|----------------|-------|
| Task enqueue (simple) | <10ms | 95th percentile |
| Task enqueue (with deps) | <20ms | With 5 prerequisites |
| Task get by ID | <5ms | 99th percentile, indexed query |
| Get next task | <5ms | Single indexed query |
| Task list (50 results) | <20ms | With filtering |
| Queue statistics | <20ms | Even with 10,000 tasks |
| Task cancel (10 deps) | <50ms | Cascade cancellation |
| Execution plan (100 tasks) | <30ms | Topological sort |

## Throughput Targets

- Enqueue throughput: >50 tasks/second
- Query throughput: >100 queries/second
- Concurrent access: 100 agents without degradation

## Running Performance Tests

### Run all performance tests:
```bash
pytest tests/performance/ -v -s --durations=10
```

### Run specific test category:
```bash
# Latency tests only
pytest tests/performance/test_task_queue_mcp_performance.py::test_enqueue_simple_task_latency -v -s

# Throughput tests
pytest tests/performance/test_task_queue_mcp_performance.py -k "throughput" -v -s

# Scalability tests
pytest tests/performance/test_task_queue_mcp_performance.py -k "scales" -v -s

# Concurrent access tests
pytest tests/performance/test_task_queue_mcp_performance.py -k "concurrent" -v -s
```

### With performance markers:
```bash
pytest -m performance -v -s
```

## Test Categories

### 1. Single Operation Latency Tests
- `test_enqueue_simple_task_latency` - Simple task enqueue <10ms
- `test_enqueue_task_with_dependencies_latency` - With dependencies <20ms
- `test_get_task_by_id_latency` - Get by ID <5ms
- `test_get_next_task_latency` - Dequeue next task <5ms
- `test_queue_status_latency` - Queue statistics <20ms
- `test_cancel_task_with_dependents_latency` - Cascade cancel <50ms
- `test_execution_plan_latency` - Topological sort <30ms

### 2. Throughput Tests
- `test_enqueue_throughput` - >50 tasks/second
- `test_query_throughput` - >100 queries/second

### 3. Scalability Tests
- `test_queue_status_scales_with_task_count` - Linear scaling with task count
- `test_dependency_depth_scales_linearly` - Linear scaling with dependency depth

### 4. Concurrent Access Tests
- `test_concurrent_enqueue_50_agents` - 50 agents enqueuing simultaneously
- `test_concurrent_dequeue_100_agents` - 100 agents dequeuing simultaneously
- `test_concurrent_mixed_operations` - Mixed operations (enqueue/dequeue/status)

### 5. Database Query Performance Tests
- `test_explain_get_next_task_query` - Verify index usage
- `test_explain_queue_status_query` - Verify aggregation optimization

### 6. Memory Usage Tests
- `test_memory_usage_with_large_queue` - Memory leak detection with 10,000 tasks

## Interpreting Results

### Latency Statistics
- **Mean**: Average latency across all runs
- **Median**: 50th percentile (typical case)
- **P95**: 95th percentile (most operations should be this fast)
- **P99**: 99th percentile (worst case for most operations)
- **Max**: Worst case observed

### What to Look For
- **Regressions**: P95/P99 latencies increasing over time
- **Outliers**: Max latency significantly higher than P99
- **Scaling**: Performance degrading non-linearly with data size
- **Concurrency**: Throughput dropping with more concurrent agents

## Performance Profiling

For detailed profiling, use:

```bash
# Profile specific test
python -m cProfile -o profile.stats -m pytest tests/performance/test_task_queue_mcp_performance.py::test_enqueue_simple_task_latency

# Analyze profile
python -c "import pstats; p = pstats.Stats('profile.stats'); p.sort_stats('cumulative'); p.print_stats(20)"
```

## Memory Profiling

For memory profiling:

```bash
# Install memory_profiler
pip install memory-profiler

# Profile memory usage
python -m memory_profiler tests/performance/test_task_queue_mcp_performance.py
```

## Benchmarking Best Practices

1. **Run on dedicated hardware**: Avoid running on shared/busy machines
2. **Warm up**: Run operations once before measuring to warm up caches
3. **Multiple iterations**: Run each test multiple times for statistical significance
4. **Consistent environment**: Use same Python version, dependencies, hardware
5. **Monitor system**: Check CPU/memory usage during tests

## Continuous Performance Testing

Integrate into CI/CD:

```yaml
# .github/workflows/performance.yml
name: Performance Tests
on:
  push:
    branches: [main]
  pull_request:

jobs:
  performance:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - name: Run performance tests
        run: |
          pytest tests/performance/ -v --durations=10
          # Fail if P95 latencies exceed targets
```

## Troubleshooting Performance Issues

### Slow Enqueue
- Check circular dependency validation performance
- Check priority calculation overhead
- Profile database INSERT operations

### Slow Queries
- Run EXPLAIN QUERY PLAN
- Check if indexes are being used
- Verify WAL mode is enabled

### High Concurrency Issues
- Check for database locking contention
- Verify connection pooling is working
- Monitor transaction rollbacks

### Memory Leaks
- Check dependency resolver cache growth
- Verify task objects are not held in memory
- Profile with memory_profiler

## Performance Optimization Ideas

If targets are not met:

1. **Database Indexes**
   - Add composite indexes for common queries
   - Use covering indexes to avoid table lookups

2. **Caching**
   - Cache queue statistics for 1 second
   - Cache dependency graph for hot tasks
   - Use Redis for distributed caching

3. **Query Optimization**
   - Use prepared statements
   - Batch operations where possible
   - Use EXPLAIN QUERY PLAN to optimize queries

4. **Concurrency**
   - Implement connection pooling
   - Use read replicas for queries
   - Implement queue-based task distribution

5. **Algorithm Optimization**
   - Cache dependency depth calculations
   - Use incremental priority updates
   - Optimize circular dependency detection

## Related Documentation

- `/docs/performance-tuning.md` - Performance tuning guide
- `/docs/database-optimization.md` - Database optimization guide
- Requirements: `/design_docs/12-task-queue-mcp/requirements.md`
- Technical Specs: `/design_docs/12-task-queue-mcp/technical-specifications.md`
