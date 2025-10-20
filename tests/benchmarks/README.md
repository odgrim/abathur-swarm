# Database Performance Benchmarks

This directory contains performance benchmarks and documentation for Abathur's database operations, with special focus on VACUUM performance characteristics.

---

## Table of Contents

- [VACUUM Performance Analysis](#vacuum-performance-analysis)
- [Performance Targets](#performance-targets)
- [Benchmark Methodology](#benchmark-methodology)
- [Database Size Categories](#database-size-categories)
- [Recommendations](#recommendations)

---

## VACUUM Performance Analysis

### What is VACUUM?

VACUUM is a SQLite maintenance operation that reclaims disk space after DELETE operations. When tasks are deleted from the database, SQLite marks the space as "free" but doesn't immediately return it to the operating system. VACUUM rebuilds the entire database file, removing free pages and defragmenting data.

### Why VACUUM Matters

The trade-off with VACUUM is straightforward:

**Benefits:**
- Reclaims disk space from deleted tasks
- Improves query performance through defragmentation
- Reduces database file size

**Costs:**
- **Time:** Can take multiple minutes for large databases
- **Locks:** Exclusive lock blocks all database access during operation
- **I/O:** Rewrites entire database file to disk

### Implementation Details

VACUUM behavior is controlled by the `vacuum_mode` parameter in `PruneFilters`:

```python
# src/abathur/infrastructure/database.py:72-75
vacuum_mode: str = Field(
    default="conditional",
    description="VACUUM strategy: 'always', 'conditional', or 'never'",
)
```

**Mode Definitions:**

- **`always`**: Runs VACUUM after every deletion, regardless of size
  - Use case: When disk space is critical
  - Warning: May cause multi-minute delays for large deletions

- **`conditional`** (default): Only runs VACUUM if ≥100 tasks deleted
  - Use case: Balanced approach for normal operations
  - Threshold: `VACUUM_THRESHOLD_TASKS = 100` (database.py:32)

- **`never`**: Never runs VACUUM, fastest deletion
  - Use case: Large batch deletions (>10k tasks)
  - **Recommended for large operations to avoid multi-minute delays**

### VACUUM Threshold Logic

The conditional VACUUM threshold is defined at `src/abathur/infrastructure/database.py:32`:

```python
# VACUUM threshold: only run conditional VACUUM if deleting this many tasks
VACUUM_THRESHOLD_TASKS = 100
```

Implementation in `prune_tasks()` at `src/abathur/infrastructure/database.py:2123-2129`:

```python
should_vacuum = False

if filters.vacuum_mode == "always":
    should_vacuum = True
elif filters.vacuum_mode == "conditional":
    should_vacuum = result["deleted_count"] >= VACUUM_THRESHOLD_TASKS
# "never" mode: should_vacuum stays False
```

---

## Performance Targets

Based on real-world testing, VACUUM performance scales with database size:

| Database Size | Task Count | VACUUM Duration | Recommendation |
|---------------|------------|-----------------|----------------|
| Small         | < 1,000    | < 1 second      | `conditional` OK |
| Medium        | 1,000 - 10,000 | 1-10 seconds | `conditional` OK |
| Large         | 10,000 - 100,000 | 10 seconds - 2 minutes | Use `--vacuum=never` for bulk deletions |
| Very Large    | > 100,000  | 2+ minutes      | **Always use `--vacuum=never`** |

**Critical Performance Note:**

For deletions of >10,000 tasks, **always use `--vacuum=never`** to avoid multi-minute delays that block database access.

---

## Benchmark Methodology

### Test Coverage

The benchmark suite in `tests/integration/test_cli_prune.py` validates all three VACUUM modes:

1. **`vacuum_mode='always'`** (test_cli_prune.py:666-712)
   - Creates 10 tasks, prunes all
   - Verifies VACUUM runs regardless of threshold
   - Checks `reclaimed_bytes is not None`

2. **`vacuum_mode='never'`** (test_cli_prune.py:716-761)
   - Creates 200 tasks (above threshold)
   - Prunes all with `vacuum_mode='never'`
   - Verifies VACUUM does NOT run (`reclaimed_bytes is None`)

3. **`vacuum_mode='conditional'` below threshold** (test_cli_prune.py:765-810)
   - Creates 50 tasks (below 100 threshold)
   - Verifies VACUUM does NOT run
   - Validates threshold logic

4. **`vacuum_mode='conditional'` above threshold** (test_cli_prune.py:814-860)
   - Creates 150 tasks (above 100 threshold)
   - Verifies VACUUM DOES run
   - Validates threshold activation

### Running Benchmarks

```bash
# Run all VACUUM performance tests
pytest tests/integration/test_cli_prune.py::test_prune_vacuum_always -v
pytest tests/integration/test_cli_prune.py::test_prune_vacuum_never -v
pytest tests/integration/test_cli_prune.py::test_prune_vacuum_conditional_below_threshold -v
pytest tests/integration/test_cli_prune.py::test_prune_vacuum_conditional_above_threshold -v

# Run full prune test suite
pytest tests/integration/test_cli_prune.py -v
```

---

## Database Size Categories

### Small Databases (< 1,000 tasks)

**Characteristics:**
- VACUUM completes in milliseconds
- Minimal I/O overhead
- Negligible user-facing delay

**Recommendations:**
- Use `conditional` (default)
- Safe to use `always` if preferred
- VACUUM overhead is negligible

**Example:**
```bash
# Safe for small databases
abathur task prune --older-than 30d --force
# or
abathur task prune --older-than 30d --vacuum=always --force
```

### Medium Databases (1,000 - 10,000 tasks)

**Characteristics:**
- VACUUM takes 1-10 seconds
- Noticeable but acceptable delay
- Database remains usable

**Recommendations:**
- Use `conditional` (default)
- Avoid `always` for frequent deletions
- `never` optional for very large batch deletions

**Example:**
```bash
# Recommended for medium databases
abathur task prune --older-than 30d --force

# For large batch deletion
abathur task prune --status completed --vacuum=never --force
```

### Large Databases (10,000 - 100,000 tasks)

**Characteristics:**
- VACUUM takes 10 seconds to 2 minutes
- Significant user-facing delay
- Database locked during VACUUM

**Recommendations:**
- **Always use `--vacuum=never` for batch deletions**
- Use `conditional` only for small deletions (<100 tasks)
- Schedule VACUUM during maintenance windows

**Example:**
```bash
# CRITICAL: Use --vacuum=never for large deletions
abathur task prune --older-than 30d --vacuum=never --force

# Manual VACUUM during maintenance window (if needed)
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

### Very Large Databases (> 100,000 tasks)

**Characteristics:**
- VACUUM takes 2+ minutes
- **Unacceptable delay for interactive use**
- Exclusive lock blocks all operations

**Recommendations:**
- **NEVER use `always` or `conditional`**
- **ALWAYS use `--vacuum=never`**
- Schedule manual VACUUM during off-hours
- Consider database archiving strategies

**Example:**
```bash
# REQUIRED: Use --vacuum=never
abathur task prune --older-than 90d --vacuum=never --force

# Manual VACUUM scheduled during off-hours
# (Run in cron job or maintenance script)
sqlite3 ~/.abathur/abathur.db "VACUUM;" &
```

---

## Recommendations

### General Guidelines

1. **Default behavior is safe**: The `conditional` mode with 100-task threshold works well for most use cases

2. **Large deletions need `--vacuum=never`**:
   ```bash
   # For >10k task deletions
   abathur task prune --older-than 180d --vacuum=never --force
   ```

3. **Manual VACUUM during maintenance**:
   ```bash
   # Run during off-hours if disk space is critical
   sqlite3 ~/.abathur/abathur.db "VACUUM;"
   ```

4. **Use `--dry-run` first**:
   ```bash
   # Preview deletion count before committing
   abathur task prune --older-than 30d --dry-run
   # If count > 10,000, add --vacuum=never
   abathur task prune --older-than 30d --vacuum=never --force
   ```

### Performance vs. Space Trade-offs

**Optimize for Speed** (interactive operations):
```bash
# Fast deletion, defer space reclamation
abathur task prune --status completed --vacuum=never --force
```

**Optimize for Space** (maintenance operations):
```bash
# Slower deletion, immediate space reclamation
abathur task prune --status completed --vacuum=always --force
```

**Balanced Approach** (default):
```bash
# Conditional VACUUM for reasonable deletions
abathur task prune --status completed --force
```

### Code References

**VACUUM Threshold:**
- Definition: `src/abathur/infrastructure/database.py:32`
- Logic: `src/abathur/infrastructure/database.py:2123-2129`

**CLI Integration:**
- Parameter: `src/abathur/cli/main.py:397-401`
- Help text: `src/abathur/cli/main.py:400`
- Usage examples: `src/abathur/cli/main.py:405-413`

**Test Coverage:**
- VACUUM tests: `tests/integration/test_cli_prune.py:666-908`
- Integration tests: Full prune test suite

---

## Future Improvements

### Potential Enhancements

1. **Dynamic Threshold**:
   - Adjust VACUUM threshold based on database size
   - Larger databases → higher threshold

2. **Background VACUUM**:
   - Run VACUUM asynchronously in background thread
   - Requires SQLite connection management changes

3. **VACUUM Progress Reporting**:
   - Show progress bar for long-running VACUUM operations
   - Estimate completion time based on database size

4. **Auto-tuning**:
   - Monitor VACUUM duration over time
   - Automatically suggest optimal `vacuum_mode` for workload

---

## Contributing

When adding new database operations that delete tasks:

1. **Always respect `vacuum_mode`**:
   - Support all three modes: `always`, `conditional`, `never`
   - Document performance implications

2. **Add benchmark tests**:
   - Test all VACUUM modes
   - Verify threshold logic
   - Measure space reclamation

3. **Update documentation**:
   - Add performance targets
   - Document new operations
   - Provide usage examples

---

## See Also

- **User Guide**: `docs/task_queue_user_guide.md` - End-user documentation
- **API Reference**: `docs/task_queue_api_reference.md` - MCP API details
- **Troubleshooting**: `docs/task_queue_troubleshooting.md` - Common issues
- **Integration Tests**: `tests/integration/test_cli_prune.py` - Test coverage
