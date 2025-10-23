# CLI Integration Tests

This directory contains end-to-end integration tests for CLI commands.

## Test Files

### test_task_show_children_integration.py

**Purpose**: Integration tests for child task display enhancement in `abathur task show` command.

**Status**: ✅ Tests implemented, waiting for CLI implementation (task ID: aa8837f8-047a-4bdb-b5e8-6b32fd2fb194)

**Test Coverage**:
1. **test_task_show_with_real_children** - Full workflow with 3 child tasks
   - Creates parent task with 3 children in real database
   - Validates "Child Tasks:" section appears
   - Verifies all children displayed with ID, summary, status
   - Performance target: <200ms total execution time

2. **test_task_show_performance_50_children** - Performance test with 50 children
   - Creates parent task with 50 children
   - Validates performance meets NFR001 requirement (<100ms for child retrieval)
   - Tests scalability and no performance degradation
   - Performance target: <200ms total execution time

3. **test_task_show_output_unchanged_for_no_children** - Backward compatibility
   - Creates task without children
   - Validates NO "Child Tasks:" section appears
   - Ensures output identical to pre-feature behavior
   - No extra whitespace or formatting changes

4. **test_task_show_child_summary_truncation** - Summary truncation validation
   - Creates child with 80-character summary
   - Validates truncation to 40 chars + '...'
   - Verifies displayed summary is 43 characters total

5. **test_task_show_child_missing_summary** - Null handling
   - Creates child with summary=None
   - Validates '-' is displayed for missing summary
   - No errors or crashes

## Running Tests

### Run all CLI integration tests:
```bash
pytest tests/integration/cli/ -v
```

### Run specific test file:
```bash
pytest tests/integration/cli/test_task_show_children_integration.py -v
```

### Run with coverage:
```bash
pytest tests/integration/cli/test_task_show_children_integration.py -v --cov=src/abathur/cli
```

### Run specific test:
```bash
pytest tests/integration/cli/test_task_show_children_integration.py::test_task_show_with_real_children -v
```

## Test Design

### Database Setup
- Uses real in-memory SQLite database (not mocked)
- Creates parent-child task relationships using `TaskQueueService`
- Tests use temporary database files that are cleaned up automatically

### Fixtures
- `temp_db_path_sync`: Creates temporary database file with cleanup
- `cli_runner`: Typer CLI test runner
- Helper functions:
  - `_setup_parent_with_children_sync()`: Creates parent with N children
  - `_setup_task_no_children_sync()`: Creates standalone task

### Mocking Strategy
- Mock `_get_services()` to use test database instead of `.abathur/abathur.db`
- Use real Database, TaskQueueService, TaskCoordinator (no mocking)
- Mock only external dependencies (template_manager, mcp_manager)

## Performance Targets

As specified in technical requirements:
- Child retrieval from database: <50ms
- Table rendering: <50ms
- Total execution time: <200ms
- NFR001 requirement: Child retrieval <100ms

## Acceptance Criteria

✅ All 5 integration tests implemented
✅ Tests use real in-memory SQLite database
✅ Tests create parent/child task relationships
✅ Full workflow test validates end-to-end behavior
✅ Performance test validates <100ms target
✅ Backward compatibility test validates no regressions
✅ Summary truncation test validates 40-char limit
✅ Null handling test validates missing summary displays '-'
⏳ Tests will pass once CLI implementation is complete

## Current Status

**Tests Status**: All tests correctly fail with expected behavior
- Tests execute CLI command successfully
- Database setup works correctly
- Parent task info displays correctly
- Child task display feature not yet implemented (expected)

**Waiting on**: CLI implementation task (aa8837f8-047a-4bdb-b5e8-6b32fd2fb194)
- Implementation needs to add child task retrieval after line 339 in main.py
- Create Rich Table for child task display
- Handle edge cases (no children, missing summaries)

**Next Steps**:
1. Complete CLI implementation (separate task)
2. Re-run integration tests to verify they all pass
3. Verify performance targets are met
4. Run full test suite to ensure no regressions

## Dependencies

Tests depend on:
- pytest
- pytest-asyncio
- typer[testing]
- abathur.infrastructure.database.Database
- abathur.services.task_queue_service.TaskQueueService
- abathur.domain.models (Task, TaskSource, TaskStatus)
- abathur.cli.main (app)

## Technical Notes

### Why These Tests Fail (Expected)
The tests correctly fail because the CLI implementation has not been completed yet. The failures show:
- "Child Tasks:" section not found in output
- This is expected - the feature hasn't been implemented yet
- Test infrastructure is working correctly

### Test Execution Time
Current execution time (with mocked CLI): ~2 seconds for all 5 tests
Expected time after implementation: <1 second (tests are fast with in-memory DB)

### Coverage
Tests provide comprehensive coverage for:
- Full workflow (parent with children)
- Performance (50 children)
- Backward compatibility (no children)
- Summary truncation (long summary)
- Null handling (missing summary)
