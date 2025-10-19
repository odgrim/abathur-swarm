# VACUUM Performance Benchmarks

This directory contains performance benchmarks for SQLite VACUUM operations in the Abathur task queue database.

## Overview

VACUUM is a SQLite operation that reclaims disk space by rebuilding the database file and removing deleted data. These benchmarks measure VACUUM performance across different database sizes and vacuum modes.

## Benchmark Tests

### 1. Small Database Benchmark (`test_vacuum_small_db`)
- **Database Size**: 100 tasks
- **Operations**: Delete 50 tasks, run VACUUM
- **Performance Target**: < 1 second
- **Purpose**: Baseline performance for small databases

### 2. Medium Database Benchmark (`test_vacuum_medium_db`)
- **Database Size**: 10,000 tasks
- **Operations**: Delete 5,000 tasks, run VACUUM
- **Performance Target**: < 60 seconds
- **Purpose**: Real-world performance for typical databases

### 3. Large Database Benchmark (`test_vacuum_large_db`)
- **Database Size**: 100,000 tasks
- **Operations**: Delete 50,000 tasks, run VACUUM
- **Performance Target**: < 300 seconds (5 minutes)
- **Purpose**: Stress test for large-scale databases
- **Note**: Marked as `@pytest.mark.slow` - may take 5-10 minutes total

### 4. Incremental Impact Analysis (`test_vacuum_incremental_impact`)
- **Database Sizes**: 100, 500, 1,000, 5,000, 10,000 tasks
- **Operations**: For each size, delete half and measure VACUUM time
- **Purpose**: Analyze VACUUM scaling behavior across sizes
- **Output**: Performance curve showing size vs duration relationship

### 5. Vacuum Mode Comparison (`test_vacuum_conditional_vs_always_performance`)
- **Test Modes**: `conditional`, `always`, `never`
- **Database Size**: 1,000 tasks
- **Purpose**: Compare performance characteristics of different vacuum modes
- **Metrics**: Duration, reclaimed bytes, VACUUM overhead

## Running Benchmarks

### Run All Benchmarks
```bash
pytest tests/benchmarks/ -v
```

### Run Only Fast Benchmarks (exclude slow tests)
```bash
pytest tests/benchmarks/ -v -m "benchmark and not slow"
```

### Run Specific Benchmark
```bash
# Small database benchmark
pytest tests/benchmarks/test_vacuum_performance.py::test_vacuum_small_db -v

# Medium database benchmark
pytest tests/benchmarks/test_vacuum_performance.py::test_vacuum_medium_db -v

# Large database benchmark (slow)
pytest tests/benchmarks/test_vacuum_performance.py::test_vacuum_large_db -v

# Incremental impact analysis
pytest tests/benchmarks/test_vacuum_performance.py::test_vacuum_incremental_impact -v

# Vacuum mode comparison
pytest tests/benchmarks/test_vacuum_performance.py::test_vacuum_conditional_vs_always_performance -v
```

### Run Only Slow Benchmarks
```bash
pytest tests/benchmarks/ -v -m "slow"
```

### Run with Detailed Output
```bash
pytest tests/benchmarks/ -v -s
# -s flag shows print statements with detailed metrics
```

## Performance Metrics

Each benchmark logs comprehensive metrics including:

- **Duration**: Time taken for VACUUM operation
- **Database Size**: Size before and after VACUUM
- **Reclaimed Space**: Bytes reclaimed by VACUUM
- **Reclaim Percentage**: Percentage of space reclaimed
- **Performance Target**: Whether target was met
- **Scaling Analysis**: Size vs time relationship (incremental test)

### Example Output

```json
{
  "test": "vacuum_medium_db",
  "task_count": 10000,
  "deleted_count": 5000,
  "create_duration_seconds": 12.456,
  "vacuum_duration_seconds": 23.789,
  "size_before_bytes": 15728640,
  "size_after_bytes": 8388608,
  "reclaimed_bytes": 7340032,
  "actual_reclaimed_bytes": 7340032,
  "reclaim_percentage": 46.67,
  "performance_target": "< 60 seconds",
  "target_met": true
}
```

## Pytest Markers

Benchmarks use the following pytest markers:

- `@pytest.mark.benchmark`: All performance benchmarks
- `@pytest.mark.slow`: Long-running tests (>30 seconds)
- `@pytest.mark.asyncio`: Async tests

### Marker Usage Examples

```bash
# Run all benchmarks
pytest -m benchmark

# Run only fast benchmarks
pytest -m "benchmark and not slow"

# Run only slow benchmarks
pytest -m "benchmark and slow"

# Exclude benchmarks from regular test runs
pytest -m "not benchmark"
```

## Performance Targets

| Database Size | Deleted Tasks | Target Duration | Test Name |
|--------------|---------------|-----------------|-----------|
| 100 tasks | 50 | < 1 second | `test_vacuum_small_db` |
| 10,000 tasks | 5,000 | < 60 seconds | `test_vacuum_medium_db` |
| 100,000 tasks | 50,000 | < 300 seconds | `test_vacuum_large_db` |

## Interpreting Results

### Performance Target Met
When a benchmark passes, it means the VACUUM operation completed within the target duration. This indicates acceptable performance for the database size.

### Performance Target Missed
If a benchmark fails (duration exceeds target), consider:
1. **Hardware factors**: Disk I/O speed, CPU performance
2. **Database factors**: WAL mode, page size, fragmentation
3. **System factors**: Background processes, available memory
4. **Scale factors**: Database may be larger than expected use case

### Scaling Analysis
The incremental impact test shows how VACUUM performance scales with database size. Ideally:
- **Linear scaling**: Time increases proportionally with size (time_ratio ≈ size_ratio)
- **Sublinear scaling**: Time increases slower than size (good)
- **Superlinear scaling**: Time increases faster than size (concerning)

## VACUUM Modes

The benchmarks test three VACUUM modes:

### `conditional` (default)
- VACUUM runs only if ≥ 100 tasks deleted
- Balances performance and space reclamation
- Recommended for most use cases

### `always`
- VACUUM runs after every prune operation
- Maximum space reclamation
- Higher performance overhead

### `never`
- VACUUM never runs automatically
- Fastest prune operations
- Database file grows over time

## CI/CD Integration

For continuous integration:

```bash
# Fast benchmarks only (CI pipeline)
pytest tests/benchmarks/ -m "benchmark and not slow" --maxfail=1

# Full benchmark suite (nightly builds)
pytest tests/benchmarks/ -m benchmark --maxfail=1
```

## Troubleshooting

### Tests are slow
- Large database tests can take 5-10 minutes
- Use `-m "benchmark and not slow"` to exclude large tests
- Run slow tests separately: `pytest -m slow`

### Tests fail with timeout
- Increase pytest timeout: `pytest --timeout=600 tests/benchmarks/`
- Check system resources (CPU, disk I/O)
- Verify no background processes affecting performance

### Inconsistent results
- Run multiple times to establish baseline
- Close other applications to reduce system load
- Use file-based database on SSD for best performance

## Contributing

When adding new benchmarks:

1. Use `@pytest.mark.benchmark` marker
2. Add `@pytest.mark.slow` for tests >30 seconds
3. Include performance targets in docstring
4. Log comprehensive metrics as JSON
5. Use descriptive test names
6. Document expected behavior
7. Update this README with new benchmarks

## References

- SQLite VACUUM documentation: https://www.sqlite.org/lang_vacuum.html
- Abathur database implementation: `src/abathur/infrastructure/database.py`
- VACUUM integration tests: `tests/integration/test_vacuum_behavior.py`
