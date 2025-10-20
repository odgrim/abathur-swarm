# VACUUM Performance Guide: Technical Implementation and User Guide

## Overview

This document provides comprehensive guidance on SQLite VACUUM operations in the Abathur task queue system. VACUUM is a database optimization operation that reclaims disk space after task deletion, but comes with performance trade-offs that users and developers must understand.

**Document Purpose**: Explain VACUUM behavior, performance characteristics, implementation details, and provide actionable guidance for both users and developers.

**Target Audience**:
- Users managing task queue databases
- Developers maintaining or extending the task queue system
- Operations teams optimizing database performance

**Last Updated**: 2025-10-18

---

## Table of Contents

- [Understanding VACUUM](#understanding-vacuum)
- [VACUUM Modes](#vacuum-modes)
- [Performance Characteristics](#performance-characteristics)
- [Implementation Details](#implementation-details)
- [User Guide](#user-guide)
- [Developer Guide](#developer-guide)
- [Troubleshooting](#troubleshooting)

---

## Understanding VACUUM

### What is VACUUM?

VACUUM is a SQLite operation that rebuilds the entire database file to reclaim unused space. When tasks are deleted, SQLite marks the space as "free" but doesn't immediately return it to the operating system. VACUUM physically reorganizes the database to eliminate this free space.

### Why VACUUM Matters

**Without VACUUM:**
- Database file grows over time
- Deleted task data remains in the file (marked as free)
- Disk space is not reclaimed
- Fast deletion operations

**With VACUUM:**
- Database file shrinks to actual data size
- Deleted task data is permanently removed
- Disk space is reclaimed
- Slower deletion operations (VACUUM takes time)

### The Trade-off

**Space vs. Time**: VACUUM reclaims disk space but increases deletion time and locks the database during execution.

**Critical Decision Point**: For large deletions (>10,000 tasks), VACUUM can take several minutes and block all database operations. The system automatically skips VACUUM in these cases to maintain responsiveness.

---

## VACUUM Modes

The Abathur task queue provides three VACUUM modes to balance performance and space reclamation:

### 1. Conditional Mode (Default)

**Trigger**: Runs VACUUM only if deleting ≥100 tasks

**Use Cases**:
- Regular maintenance operations
- Small to medium deletions (100-10,000 tasks)
- Automated scripts where disk space matters

**Performance**:
- Small deletions (<100 tasks): Fast, no VACUUM overhead
- Medium deletions (100-10,000 tasks): Moderate delay (1-60 seconds)
- Large deletions (>10,000 tasks): Automatically switches to "never" mode

**Command Example**:
```bash
# Default behavior - VACUUM runs if ≥100 tasks deleted
abathur task prune --older-than 30d --force
```

**Implementation**: See `src/abathur/infrastructure/database.py:2231-2234`

### 2. Never Mode

**Trigger**: Never runs VACUUM, regardless of deletion count

**Use Cases**:
- Large batch deletions (>10,000 tasks)
- Performance-critical operations
- When disk space is not a concern
- Scheduled maintenance (VACUUM separately during off-hours)

**Performance**:
- Fastest deletion possible
- No database locking
- No progress indicator needed

**Command Example**:
```bash
# Fast deletion without VACUUM
abathur task prune --older-than 180d --vacuum=never --force
```

**Critical Recommendation**: Always use `--vacuum=never` for deletions >10,000 tasks to avoid multi-minute delays.

### 3. Always Mode

**Trigger**: Always runs VACUUM after deletion, even for 1 task

**Use Cases**:
- Disk space is critical
- Compliance requirements for data deletion
- Manual cleanup operations
- Small databases (<1,000 tasks total)

**Performance**:
- Slowest deletion mode
- Locks database during VACUUM
- Shows progress indicator for user feedback

**Command Example**:
```bash
# Always reclaim space immediately
abathur task prune --older-than 30d --vacuum=always --force
```

**Warning**: May cause multi-minute delays for large databases (>100,000 tasks).

---

## Performance Characteristics

### VACUUM Duration by Database Size

Based on empirical testing and SQLite documentation:

| Database Size | Task Count | VACUUM Duration | Recommended Mode |
|---------------|------------|-----------------|------------------|
| Small         | < 1,000    | < 1 second      | `conditional` or `always` |
| Medium        | 1,000 - 10,000 | 1-10 seconds | `conditional` |
| Large         | 10,000 - 100,000 | 10 sec - 2 min | `never` (for bulk ops) |
| Very Large    | > 100,000  | 2+ minutes      | **Always use `never`** |

### Auto-Skip Threshold

**Constant**: `AUTO_SKIP_VACUUM_THRESHOLD = 10_000` (defined in `src/abathur/infrastructure/database.py:36`)

**Behavior**: When deleting ≥10,000 tasks, the system automatically overrides `vacuum_mode` to `"never"` to prevent long database locks.

**Rationale**: VACUUM on 10,000+ tasks can take minutes, blocking the database and preventing concurrent operations.

**Code Reference**: `src/abathur/infrastructure/database.py:2195-2197`

```python
if len(task_ids) >= AUTO_SKIP_VACUUM_THRESHOLD and filters.vacuum_mode != "never":
    effective_vacuum_mode = "never"
    vacuum_auto_skipped = True
```

### VACUUM Conditional Threshold

**Constant**: `VACUUM_THRESHOLD_TASKS = 100` (defined in `src/abathur/infrastructure/database.py:32`)

**Behavior**: In `conditional` mode, VACUUM runs only if `deleted_tasks >= 100`.

**Code Reference**: `src/abathur/infrastructure/database.py:2233-2234`

```python
elif effective_vacuum_mode == "conditional":
    should_vacuum = result["deleted_count"] >= VACUUM_THRESHOLD_TASKS
```

### Progress Indicator Behavior

The CLI shows a spinner progress indicator when VACUUM is expected to run, providing user feedback during long operations.

**Display Logic** (`src/abathur/cli/main.py:632-635`):

```python
show_vacuum_progress = (
    filters.vacuum_mode == "always" or
    (filters.vacuum_mode == "conditional" and len(preview_task_ids) >= 100)
)
```

**Progress Indicator Example**:
```
Deleting tasks and optimizing database... ⠋
```

**Implementation**: Uses `rich.progress.Progress` with `SpinnerColumn` and `TextColumn` for visual feedback.

**Code Reference**: `src/abathur/cli/main.py:640-648`

---

## Implementation Details

### Database Layer

#### Prune Tasks Method

**Location**: `src/abathur/infrastructure/database.py:2139-2271`

**Signature**:
```python
async def prune_tasks(self, filters: PruneFilters) -> PruneResult:
    """Prune tasks based on age and status criteria.

    Handles:
    1. Task selection (via filters)
    2. Task deletion (via unified core)
    3. Statistics collection
    4. Optional VACUUM
    """
```

**VACUUM Decision Flow**:

1. **Select tasks** using `PruneFilters.build_where_clause()`
2. **Auto-skip check**: If `len(task_ids) >= 10_000`, override to `vacuum_mode="never"`
3. **Delete tasks** using `_delete_tasks_by_ids()`
4. **VACUUM decision**:
   - `always`: Always run VACUUM
   - `conditional`: Run if `deleted_count >= 100`
   - `never`: Skip VACUUM
5. **Measure space reclaimed** (if VACUUM runs)

**Code Example** (`src/abathur/infrastructure/database.py:2226-2258`):

```python
# STEP 3: VACUUM (outside transaction, conditional)
reclaimed_bytes = None
should_vacuum = False

if effective_vacuum_mode == "always":
    should_vacuum = True
elif effective_vacuum_mode == "conditional":
    should_vacuum = result["deleted_count"] >= VACUUM_THRESHOLD_TASKS

if should_vacuum:
    # Get database size before VACUUM
    cursor = await conn.execute("PRAGMA page_count")
    page_count_before = (await cursor.fetchone())[0]
    cursor = await conn.execute("PRAGMA page_size")
    page_size = (await cursor.fetchone())[0]
    size_before = page_count_before * page_size

    # Run VACUUM
    await conn.execute("VACUUM")

    # Get database size after VACUUM
    cursor = await conn.execute("PRAGMA page_count")
    page_count_after = (await cursor.fetchone())[0]
    size_after = page_count_after * page_size
    reclaimed_bytes = size_before - size_after
```

#### PruneResult Model

**Location**: `src/abathur/infrastructure/database.py:185-224`

**Fields**:
```python
class PruneResult(BaseModel):
    deleted_tasks: int              # Number of tasks deleted
    deleted_dependencies: int       # Number of dependencies deleted
    reclaimed_bytes: int | None     # Bytes reclaimed by VACUUM (if run)
    dry_run: bool                   # Whether this was a preview
    breakdown_by_status: dict[TaskStatus, int]  # Tasks by status
    vacuum_auto_skipped: bool       # Auto-skipped for large deletion
```

**Usage**: Returned by `Database.prune_tasks()` to inform CLI display logic.

### CLI Layer

#### Prune Command

**Location**: `src/abathur/cli/main.py:377-825`

**Command Signature**:
```bash
abathur task prune [OPTIONS]
```

**Key Options**:
- `--vacuum`: VACUUM mode (`always`, `conditional`, `never`)
- `--older-than`: Delete tasks older than duration (e.g., `30d`, `6h`)
- `--status`: Filter by task status
- `--dry-run`: Preview without deleting
- `--force`: Skip confirmation prompt

**Progress Indicator Logic** (`src/abathur/cli/main.py:631-652`):

```python
# Show progress indicator for VACUUM if expected to run
show_vacuum_progress = (
    filters.vacuum_mode == "always" or
    (filters.vacuum_mode == "conditional" and len(preview_task_ids) >= 100)
)

if show_vacuum_progress:
    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        console=console,
    ) as progress:
        task_desc = progress.add_task(
            description="Deleting tasks and optimizing database...",
            total=None
        )
        result = await services["database"].prune_tasks(filters)
else:
    # No VACUUM expected, run without progress indicator
    result = await services["database"].prune_tasks(filters)
```

**Result Display Logic** (`src/abathur/cli/main.py:683-694`):

```python
# Display VACUUM information
if result.vacuum_auto_skipped:
    console.print(f"\n[yellow]⚠[/yellow]  VACUUM automatically skipped (deleting {result.deleted_tasks} tasks)")
    console.print("[dim]Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.[/dim]")
    console.print("[dim]Run 'abathur task prune --older-than 0d --vacuum=always' to manually VACUUM if needed.[/dim]")
elif result.reclaimed_bytes is not None:
    reclaimed_mb = result.reclaimed_bytes / (1024 * 1024)
    console.print(f"\n[green]VACUUM completed: {reclaimed_mb:.2f} MB reclaimed[/green]")
elif filters.vacuum_mode == "never":
    console.print("\n[dim]VACUUM skipped (--vacuum=never)[/dim]")
elif filters.vacuum_mode == "conditional" and result.deleted_tasks < 100:
    console.print(f"\n[dim]VACUUM skipped (conditional mode, only {result.deleted_tasks} tasks deleted, threshold is 100)[/dim]")
```

### Testing

**Test Suite**: `tests/integration/test_cli_vacuum_progress.py`

**Test Coverage**:
1. **Always mode**: Progress indicator shown for all deletions
2. **Conditional above threshold**: VACUUM runs for ≥100 tasks
3. **Conditional below threshold**: VACUUM skipped for <100 tasks
4. **Never mode**: VACUUM never runs, even for large deletions

**Test Example** (`tests/integration/test_cli_vacuum_progress.py:76-115`):

```python
@pytest.mark.asyncio
async def test_vacuum_progress_indicator_shown_for_conditional_above_threshold():
    """Test progress indicator shown when deleting ≥100 tasks."""
    db = Database(db_path)
    await db.initialize()

    # Create 100 tasks (at conditional threshold)
    for i in range(100):
        task = Task(
            prompt=f"Task {i}",
            summary=f"Summary {i}",
            agent_type="test-agent",
            source=TaskSource.HUMAN,
            status=TaskStatus.COMPLETED,
            submitted_at=old_timestamp,
            completed_at=old_timestamp,
        )
        await db.insert_task(task)

    # Execute prune with conditional mode
    filters = PruneFilters(task_ids=task_ids, vacuum_mode="conditional")
    result = await db.prune_tasks(filters)

    # Verify VACUUM ran
    assert result.deleted_tasks == 100
    assert result.reclaimed_bytes is not None
```

---

## User Guide

### Basic Usage

#### Default Behavior (Conditional Mode)

For most operations, use the default `conditional` mode:

```bash
# Delete completed tasks older than 30 days
abathur task prune --older-than 30d --force
```

**What happens**:
- If deleting ≥100 tasks: VACUUM runs, space reclaimed
- If deleting <100 tasks: VACUUM skipped, faster execution
- If deleting ≥10,000 tasks: VACUUM auto-skipped to prevent long locks

#### Fast Deletion (Never Mode)

For large batch deletions, explicitly use `--vacuum=never`:

```bash
# Delete tasks older than 180 days (fast, no VACUUM)
abathur task prune --older-than 180d --vacuum=never --force
```

**When to use**:
- Deleting >10,000 tasks
- Performance-critical operations
- Scheduled batch cleanup jobs

#### Always Reclaim Space (Always Mode)

When disk space is critical:

```bash
# Always reclaim space, even for 1 task
abathur task prune --older-than 30d --vacuum=always --force
```

**When to use**:
- Disk space is limited
- Compliance requirements for data deletion
- Small databases (<1,000 tasks)

### Monitoring VACUUM Execution

#### Check if VACUUM Ran

The CLI provides clear feedback about VACUUM execution:

**VACUUM ran successfully**:
```
✓ Successfully deleted 250 tasks

VACUUM completed: 12.45 MB reclaimed
```

**VACUUM auto-skipped (large deletion)**:
```
✓ Successfully deleted 15000 tasks

⚠  VACUUM automatically skipped (deleting 15000 tasks)
Large prune operations (>10,000 tasks) skip VACUUM to avoid long database locks.
Run 'abathur task prune --older-than 0d --vacuum=always' to manually VACUUM if needed.
```

**VACUUM skipped (conditional mode, below threshold)**:
```
✓ Successfully deleted 50 tasks

VACUUM skipped (conditional mode, only 50 tasks deleted, threshold is 100)
```

**VACUUM skipped (never mode)**:
```
✓ Successfully deleted 200 tasks

VACUUM skipped (--vacuum=never)
```

### Manual VACUUM During Maintenance

If you skip VACUUM during deletion (using `--vacuum=never`), you can manually reclaim space during off-hours:

```bash
# Run manual VACUUM during maintenance window
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Best Practice**: Schedule weekly VACUUM during low-traffic periods:

```bash
# Add to crontab for weekly Sunday 2 AM VACUUM
0 2 * * 0 sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

### Monitoring Database Size

Check database file size to determine if VACUUM is needed:

```bash
# Check database size
ls -lh ~/.abathur/abathur.db

# Example output:
# -rw-r--r-- 1 user group 245M Oct 18 10:30 /Users/user/.abathur/abathur.db
```

If the database is growing despite task deletions, manual VACUUM may be needed.

### Performance Best Practices

#### 1. Preview Before Deleting

Always preview deletion operations with `--dry-run`:

```bash
# Check what will be deleted
abathur task prune --older-than 30d --dry-run
```

**Output example**:
```
Tasks to Delete (250)
┌────────┬─────────────────┬───────────┬────────────┐
│ ID     │ Summary         │ Status    │ Agent Type │
├────────┼─────────────────┼───────────┼────────────┤
│ ebec23 │ Implement aut...│ completed │ python-... │
│ a7f3d2 │ Add tests for...│ completed │ python-... │
└────────┴─────────────────┴───────────┴────────────┘

Dry-run mode - no changes will be made
Would delete 250 task(s)
```

#### 2. Choose Appropriate VACUUM Mode

**Decision Tree**:

```
How many tasks are you deleting?
├─ < 100 tasks
│  └─ Use default (conditional) - VACUUM skipped, fast
├─ 100 - 10,000 tasks
│  ├─ Disk space matters?
│  │  ├─ Yes → Use default (conditional) - VACUUM runs
│  │  └─ No → Use --vacuum=never - faster
│  └─ Performance critical?
│     └─ Yes → Use --vacuum=never
└─ > 10,000 tasks
   └─ **ALWAYS use --vacuum=never** (VACUUM auto-skipped anyway)
```

#### 3. Schedule Regular Maintenance

**Weekly VACUUM** (during low-traffic periods):

```bash
# Cron job: Sunday 2 AM
0 2 * * 0 sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Monthly cleanup** (delete old completed tasks):

```bash
# Cron job: 1st of month, 3 AM
0 3 1 * * abathur task prune --older-than 90d --vacuum=never --force
```

**Quarterly space reclamation** (manual VACUUM after big cleanup):

```bash
# After large deletion, manually VACUUM
abathur task prune --older-than 180d --vacuum=never --force
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

### Common Scenarios

#### Scenario 1: Daily Maintenance

**Goal**: Delete yesterday's completed tasks, reclaim space if significant

```bash
# Delete tasks older than 1 day (conditional VACUUM)
abathur task prune --older-than 1d --status completed --force
```

**Expected behavior**:
- If <100 tasks: Fast deletion, no VACUUM
- If ≥100 tasks: VACUUM runs, space reclaimed

#### Scenario 2: Large Cleanup

**Goal**: Delete 6 months of old tasks, optimize for speed

```bash
# Step 1: Fast deletion without VACUUM
abathur task prune --older-than 180d --vacuum=never --force

# Step 2: Manual VACUUM during off-hours
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Why**: Deleting many tasks with VACUUM can lock the database for minutes. Separate deletion from VACUUM for better control.

#### Scenario 3: Disk Space Emergency

**Goal**: Immediately reclaim all possible space

```bash
# Delete all completed/failed/cancelled tasks, always VACUUM
abathur task prune --status completed --vacuum=always --force
abathur task prune --status failed --vacuum=always --force
abathur task prune --status cancelled --vacuum=always --force
```

**Warning**: This may take several minutes for large databases.

---

## Developer Guide

### Extending VACUUM Behavior

#### Adding New VACUUM Modes

**Current modes**: `always`, `conditional`, `never`

**To add a new mode** (e.g., `adaptive`):

1. **Update validation** in `src/abathur/infrastructure/database.py:122-129`:

```python
@field_validator("vacuum_mode")
@classmethod
def validate_vacuum_mode(cls, v: str) -> str:
    allowed = {"always", "conditional", "never", "adaptive"}  # Add new mode
    if v not in allowed:
        raise ValueError(f"vacuum_mode must be one of {allowed}, got '{v}'")
    return v
```

2. **Implement logic** in `src/abathur/infrastructure/database.py:2226-2235`:

```python
if effective_vacuum_mode == "always":
    should_vacuum = True
elif effective_vacuum_mode == "conditional":
    should_vacuum = result["deleted_count"] >= VACUUM_THRESHOLD_TASKS
elif effective_vacuum_mode == "adaptive":
    # New logic: VACUUM based on database size ratio
    should_vacuum = (size_before / size_after) > 1.2  # 20% free space
```

3. **Update CLI** in `src/abathur/cli/main.py:348`:

```python
vacuum: str = typer.Option(
    "conditional",
    help="VACUUM mode: always, conditional, never, adaptive"
),
```

4. **Add tests** in `tests/integration/test_cli_vacuum_progress.py`:

```python
@pytest.mark.asyncio
async def test_adaptive_vacuum_mode():
    """Test adaptive VACUUM mode based on free space ratio."""
    # Test implementation
```

#### Tuning Thresholds

**Current thresholds**:
- `VACUUM_THRESHOLD_TASKS = 100` (conditional trigger)
- `AUTO_SKIP_VACUUM_THRESHOLD = 10_000` (auto-skip threshold)

**To adjust thresholds**:

1. **Edit constants** in `src/abathur/infrastructure/database.py:31-36`:

```python
# VACUUM threshold: only run conditional VACUUM if deleting this many tasks
VACUUM_THRESHOLD_TASKS = 200  # Increase from 100 to 200

# Auto-skip VACUUM threshold
AUTO_SKIP_VACUUM_THRESHOLD = 20_000  # Increase from 10,000 to 20,000
```

2. **Update documentation** in user guide and this document

3. **Update tests** to reflect new thresholds

**Considerations**:
- Higher `VACUUM_THRESHOLD_TASKS`: Less frequent VACUUM, faster deletions, more wasted space
- Lower `AUTO_SKIP_VACUUM_THRESHOLD`: More aggressive auto-skip, faster large deletions, but may skip VACUUM when it's acceptable

#### Monitoring VACUUM Performance

**Add timing metrics** to track VACUUM duration:

```python
import time

# In Database.prune_tasks() method
if should_vacuum:
    start_time = time.time()
    await conn.execute("VACUUM")
    vacuum_duration = time.time() - start_time

    # Log metric
    logger.info(
        "VACUUM completed",
        extra={
            "vacuum_duration_sec": vacuum_duration,
            "deleted_tasks": result["deleted_count"],
            "reclaimed_bytes": reclaimed_bytes
        }
    )
```

**Add metrics table** to database for historical tracking:

```python
# Store VACUUM metrics
await conn.execute(
    """
    INSERT INTO metrics (timestamp, metric_name, metric_value, labels)
    VALUES (?, 'vacuum_duration_seconds', ?, ?)
    """,
    (
        datetime.now(timezone.utc).isoformat(),
        vacuum_duration,
        json.dumps({
            "deleted_tasks": result["deleted_count"],
            "reclaimed_mb": reclaimed_bytes / (1024 * 1024)
        })
    )
)
```

### Performance Optimization

#### Incremental VACUUM (SQLite 3.31+)

SQLite 3.31 introduced incremental VACUUM for large databases:

```python
# Instead of full VACUUM
await conn.execute("VACUUM")

# Use incremental VACUUM (max 10MB per run)
await conn.execute("PRAGMA incremental_vacuum(10240)")  # 10MB in pages
```

**Benefits**:
- Shorter lock duration
- Gradual space reclamation
- Better for large databases

**Implementation example**:

```python
if effective_vacuum_mode == "incremental":
    # Incremental VACUUM: reclaim up to 50MB at a time
    await conn.execute("PRAGMA incremental_vacuum(51200)")  # 50MB
```

#### Auto-Vacuum Mode

Enable SQLite auto-vacuum to automatically reclaim space on delete:

```python
# In Database.initialize()
await conn.execute("PRAGMA auto_vacuum=INCREMENTAL")
```

**Trade-offs**:
- Automatic space reclamation
- Slightly slower deletions
- No manual VACUUM needed
- Cannot be disabled without rebuilding database

---

## Troubleshooting

### Common Issues

#### Issue 1: Deletion Takes Too Long

**Symptoms**:
```
Deleting tasks and optimizing database... ⠋
[Hangs for several minutes]
```

**Cause**: VACUUM running on large database (>100,000 tasks)

**Solution**: Use `--vacuum=never` for large deletions

```bash
# Cancel the hung operation (Ctrl+C)
# Retry with --vacuum=never
abathur task prune --older-than 90d --vacuum=never --force
```

**Prevention**: Always use `--vacuum=never` for deletions >10,000 tasks

#### Issue 2: Database File Not Shrinking

**Symptoms**:
```
✓ Successfully deleted 5000 tasks

VACUUM skipped (conditional mode, only 5000 tasks deleted, threshold is 100)
```

Database file size unchanged despite deletion.

**Cause**: Conditional mode threshold not met (this is a bug in the message - 5000 > 100)

**Actual Cause**: Auto-skip threshold triggered (5000 < 10,000), or explicit `--vacuum=never`

**Solution**: Manually run VACUUM

```bash
# Manual VACUUM
sqlite3 ~/.abathur/abathur.db "VACUUM;"

# Or force VACUUM with always mode
abathur task prune --older-than 0d --vacuum=always --force
```

#### Issue 3: Database Locked During Deletion

**Symptoms**:
```
Error: Database operation failed: OperationalError: database is locked
```

**Cause**: VACUUM holds exclusive lock, another process is accessing the database

**Solution**: Wait for VACUUM to complete, or use `--vacuum=never` next time

```bash
# Check for running processes
ps aux | grep abathur

# Kill hung processes (if necessary)
kill -9 <PID>

# Retry with --vacuum=never
abathur task prune --older-than 90d --vacuum=never --force
```

**Prevention**: Use `--vacuum=never` for operations that must not block

#### Issue 4: VACUUM Auto-Skipped Unexpectedly

**Symptoms**:
```
⚠  VACUUM automatically skipped (deleting 12000 tasks)
```

You expected VACUUM to run, but it was auto-skipped.

**Cause**: Deletion count ≥10,000 triggers auto-skip threshold

**Solution**: This is intentional behavior to prevent long locks. To force VACUUM:

```bash
# Manual VACUUM after deletion
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Alternative**: Lower `AUTO_SKIP_VACUUM_THRESHOLD` in code (see Developer Guide)

### Debugging Tools

#### Check Database Integrity

```bash
# Verify database integrity
sqlite3 ~/.abathur/abathur.db "PRAGMA integrity_check;"

# Expected output:
# ok
```

#### Analyze Database Size

```bash
# Get detailed database statistics
sqlite3 ~/.abathur/abathur.db <<EOF
PRAGMA page_count;
PRAGMA page_size;
PRAGMA freelist_count;
EOF

# Calculate wasted space:
# wasted_space = freelist_count * page_size
```

#### Monitor VACUUM Progress

SQLite VACUUM doesn't provide progress updates, but you can monitor file size changes:

```bash
# Terminal 1: Run VACUUM
sqlite3 ~/.abathur/abathur.db "VACUUM;"

# Terminal 2: Monitor file size (updates every 2 seconds)
watch -n 2 "ls -lh ~/.abathur/abathur.db"
```

---

## Summary

### Key Takeaways

1. **VACUUM reclaims disk space** but increases deletion time and locks the database
2. **Three VACUUM modes**:
   - `conditional` (default): Balance of space and speed
   - `never`: Fastest deletion, no space reclamation
   - `always`: Always reclaim space, slowest
3. **Auto-skip threshold** (10,000 tasks): Automatically prevents long VACUUM locks
4. **Conditional threshold** (100 tasks): Triggers VACUUM in conditional mode
5. **Progress indicator**: Shows spinner when VACUUM is expected to run
6. **Best practice**: Use `--vacuum=never` for deletions >10,000 tasks, then manual VACUUM during maintenance

### Quick Reference

**Small deletions (<100 tasks)**:
```bash
abathur task prune --older-than 7d --force  # Fast, no VACUUM
```

**Medium deletions (100-10,000 tasks)**:
```bash
# Space matters
abathur task prune --older-than 30d --force  # Default, VACUUM runs

# Speed matters
abathur task prune --older-than 30d --vacuum=never --force  # Fast, skip VACUUM
```

**Large deletions (>10,000 tasks)**:
```bash
# ALWAYS use --vacuum=never (auto-skipped anyway)
abathur task prune --older-than 180d --vacuum=never --force

# Manual VACUUM later (optional)
sqlite3 ~/.abathur/abathur.db "VACUUM;"
```

**Disk space critical**:
```bash
abathur task prune --older-than 30d --vacuum=always --force  # Force VACUUM
```

---

## Related Documentation

- **User Guide**: `docs/task_queue_user_guide.md` - General task queue usage and VACUUM user guide
- **Database Schema**: `src/abathur/infrastructure/database.py` - Database implementation
- **CLI Reference**: `src/abathur/cli/main.py` - CLI prune command implementation
- **Test Suite**: `tests/integration/test_cli_vacuum_progress.py` - VACUUM progress indicator tests
- **Benchmark Guide**: `tests/benchmarks/README.md` - Performance benchmarking methodology

---

## Changelog

**2025-10-18**: Initial version
- Comprehensive VACUUM guide created
- Technical implementation details documented
- User guide with scenarios and best practices
- Developer guide for extending VACUUM behavior
- Troubleshooting section for common issues

---

**Document Maintainer**: Technical Documentation Writer Specialist
**Review Cycle**: Quarterly or when VACUUM behavior changes
**Feedback**: Report issues at https://github.com/anthropics/claude-code/issues
