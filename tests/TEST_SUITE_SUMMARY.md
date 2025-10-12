# Task Queue MCP Server - Test Suite Summary

## Overview

Comprehensive test suite for the Task Queue MCP Server following test-first development (TFD) principles. Tests are written BEFORE implementation to clearly define expected behavior.

**Status**: Tests will FAIL initially (implementation does not exist yet)
**Purpose**: Guide implementation and ensure all requirements are met
**Coverage Target**: >90% for MCP server, all error paths tested

---

## Test Structure

```
tests/
├── unit/mcp/
│   └── test_task_queue_server.py       # Unit tests for MCP tool handlers
├── integration/mcp/
│   └── test_task_queue_mcp_integration.py  # End-to-end workflow tests
└── performance/
    ├── test_task_queue_mcp_performance.py  # Performance and load tests
    └── README.md                            # Performance testing guide
```

---

## Test Files

### 1. Unit Tests: `tests/unit/mcp/test_task_queue_server.py`

**Purpose**: Test individual MCP tool handlers with mocked dependencies
**Approach**: Mock TaskQueueService to isolate MCP server logic
**Test Count**: 50+ tests covering all tool handlers and error cases

#### Test Categories:

**Server Initialization (1 test)**
- Server initializes with correct name and configuration

**task_enqueue Handler (12 tests)**
- ✓ Success with minimal parameters
- ✓ Success with all parameters
- ✗ Missing required fields (description, source)
- ✗ Invalid priority range (<0, >10)
- ✗ Invalid source enum value
- ✗ Invalid UUID formats (prerequisites, parent_task_id)
- ✗ Invalid deadline format
- ✗ Circular dependency detected
- ✗ Prerequisite not found
- ✓ Returns correct response format

**task_get Handler (4 tests)**
- ✓ Success - retrieve task by ID
- ✗ Task not found (404)
- ✗ Missing task_id parameter
- ✗ Invalid UUID format

**task_list Handler (6 tests)**
- ✓ Success with no filters
- ✓ Success with status filter
- ✓ Success with custom limit
- ✗ Invalid status value
- ✗ Invalid limit (negative, exceeds max)
- ✓ Empty result (no matches)

**task_queue_status Handler (2 tests)**
- ✓ Success - returns all statistics
- ✓ Empty queue (all zeros)

**task_cancel Handler (5 tests)**
- ✓ Success with no cascade
- ✓ Success with cascade (multiple dependents)
- ✗ Task not found
- ✗ Missing task_id parameter
- ✗ Invalid UUID format

**task_execution_plan Handler (5 tests)**
- ✓ Success - linear chain
- ✓ Success - parallel branches
- ✓ Empty task_ids
- ✗ Circular dependency detected
- ✗ Invalid input formats

**Serialization & Error Handling (3 tests)**
- Task serialization with all fields
- Task serialization with minimal fields
- Unexpected exceptions caught and formatted

#### How to Run:

```bash
# Run all unit tests
pytest tests/unit/mcp/test_task_queue_server.py -v

# Run specific test
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -v

# With coverage
pytest tests/unit/mcp/test_task_queue_server.py --cov=abathur.mcp --cov-report=html
```

---

### 2. Integration Tests: `tests/integration/mcp/test_task_queue_mcp_integration.py`

**Purpose**: Test end-to-end workflows with real database and service layer
**Approach**: Use in-memory database, real TaskQueueService
**Test Count**: 25+ tests covering complete user workflows

#### Test Categories:

**End-to-End Task Workflow (3 tests)**
- Complete flow: enqueue → get → dequeue → complete
- Task with dependency blocks until prerequisite completes
- Dependency chain executes in correct order (A → B → C)
- Parallel tasks (no dependencies) can be dequeued simultaneously

**Cascade Cancellation (2 tests)**
- Cancelling task cascades to all dependents
- Failing task cascades cancellation to dependents

**Queue Status (2 tests)**
- Queue status with mixed task statuses
- Queue status updates after completions

**Execution Plan (2 tests)**
- Execution plan for linear chain
- Execution plan with parallel branches

**Error Scenarios (3 tests)**
- Circular dependency rejected
- Prerequisite not found rejected
- Cancel/complete nonexistent task raises error

**Priority-Based Dequeue (2 tests)**
- Higher priority tasks dequeued first
- FIFO tiebreaker for equal priority

**Session Integration (2 tests)**
- Tasks can be linked to sessions
- Parent-child task hierarchy

**Concurrent Access (3 tests)**
- Concurrent task enqueue (10 tasks)
- Concurrent task dequeue (5 agents)
- Concurrent task completion

#### How to Run:

```bash
# Run all integration tests
pytest tests/integration/mcp/test_task_queue_mcp_integration.py -v

# Run specific workflow
pytest tests/integration/mcp/test_task_queue_mcp_integration.py::test_complete_task_workflow_enqueue_get_complete -v

# Run only cascade tests
pytest tests/integration/mcp/test_task_queue_mcp_integration.py -k "cascade" -v
```

---

### 3. Performance Tests: `tests/performance/test_task_queue_mcp_performance.py`

**Purpose**: Validate performance targets and scalability
**Approach**: Measure latency, throughput, and resource usage
**Test Count**: 15+ tests covering all performance requirements

#### Performance Targets:

| Operation | Target | Test |
|-----------|--------|------|
| Task enqueue (simple) | <10ms (P95) | `test_enqueue_simple_task_latency` |
| Task enqueue (with deps) | <20ms (P95) | `test_enqueue_task_with_dependencies_latency` |
| Task get by ID | <5ms (P99) | `test_get_task_by_id_latency` |
| Get next task | <5ms (P99) | `test_get_next_task_latency` |
| Queue statistics | <20ms (P95) | `test_queue_status_latency` |
| Task cancel (10 deps) | <50ms | `test_cancel_task_with_dependents_latency` |
| Execution plan (100 tasks) | <30ms | `test_execution_plan_latency` |
| Enqueue throughput | >50 tasks/sec | `test_enqueue_throughput` |
| Query throughput | >100 queries/sec | `test_query_throughput` |
| Concurrent agents | 100 agents | `test_concurrent_dequeue_100_agents` |

#### Test Categories:

**Single Operation Latency (7 tests)**
- Enqueue simple task
- Enqueue with dependencies
- Get task by ID
- Get next task
- Queue status
- Cancel with cascade
- Execution plan

**Throughput (2 tests)**
- Enqueue throughput
- Query throughput

**Scalability (2 tests)**
- Queue status scales linearly with task count
- Dependency depth scales linearly

**Concurrent Access (3 tests)**
- 50 concurrent agents enqueuing
- 100 concurrent agents dequeuing
- Mixed concurrent operations

**Database Query Performance (2 tests)**
- EXPLAIN QUERY PLAN for get_next_task
- EXPLAIN QUERY PLAN for queue status

**Memory Usage (1 test)**
- Memory leak detection with 10,000 tasks

#### How to Run:

```bash
# Run all performance tests (may be slow)
pytest tests/performance/ -v -s --durations=10

# Run only latency tests
pytest tests/performance/ -k "latency" -v -s

# Run only throughput tests
pytest tests/performance/ -k "throughput" -v -s

# Run with performance marker
pytest -m performance -v -s
```

**Note**: Performance tests should be run on dedicated hardware for accurate results.

---

## Running All Tests

### Quick Test Run:
```bash
# Unit tests only (fast)
pytest tests/unit/mcp/ -v

# Integration tests (moderate)
pytest tests/integration/mcp/ -v

# Performance tests (slow)
pytest tests/performance/ -v -s
```

### Comprehensive Test Run:
```bash
# All tests with coverage
pytest tests/ -v --cov=abathur.mcp --cov-report=html --cov-report=term-missing

# Generate coverage report
open htmlcov/index.html
```

### Continuous Integration:
```bash
# Fast tests for CI (unit + integration)
pytest tests/unit/mcp/ tests/integration/mcp/ -v --maxfail=5

# Full test suite (including performance)
pytest tests/ -v --cov=abathur.mcp --cov-report=xml
```

---

## Test Fixtures

Reusable test fixtures defined in:
- `tests/conftest.py` - Global fixtures (database, services)
- Individual test files - Local fixtures for specific test needs

### Key Fixtures:

```python
@pytest.fixture
async def memory_db() -> Database:
    """In-memory database for fast tests."""

@pytest.fixture
async def task_queue_service(memory_db: Database) -> TaskQueueService:
    """TaskQueueService with all dependencies."""

@pytest.fixture
def sample_task() -> Task:
    """Sample task for testing."""

@pytest.fixture
async def populated_queue(task_queue_service: TaskQueueService) -> dict[str, UUID]:
    """Database with pre-populated task chain."""
```

---

## Test Coverage Goals

### Unit Tests (MCP Server)
- **Target**: >90% code coverage
- **Focus**: All tool handlers, input validation, error handling
- **Mock**: TaskQueueService, Database

### Integration Tests
- **Target**: All critical user workflows covered
- **Focus**: End-to-end flows, database integration, state transitions
- **Real**: Database, TaskQueueService, all components

### Performance Tests
- **Target**: All performance requirements validated
- **Focus**: Latency, throughput, scalability, concurrency
- **Measure**: Actual timings, resource usage

---

## Expected Test Results (Before Implementation)

**Current Status**: ALL TESTS WILL FAIL (implementation doesn't exist yet)

### Expected Failures:

1. **Import Errors**: `abathur.mcp.task_queue_server` module not found
2. **Missing Class**: `AbathurTaskQueueServer` class doesn't exist
3. **Missing Methods**: Tool handler methods not implemented

### After Implementation:

1. **Phase 1**: Unit tests should pass (basic tool handlers)
2. **Phase 2**: Integration tests should pass (end-to-end workflows)
3. **Phase 3**: Performance tests should pass (meet targets)

---

## Test-First Development Workflow

1. **Read Test**: Understand what the test expects
2. **Write Minimal Code**: Write just enough code to make test pass
3. **Run Test**: Verify test passes
4. **Refactor**: Improve code while keeping tests passing
5. **Repeat**: Move to next test

### Example:

```bash
# 1. Run test (should fail)
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -v

# 2. Read error message, understand what's needed

# 3. Implement minimal code to pass test

# 4. Run test again (should pass)
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -v

# 5. Move to next test
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_missing_description -v
```

---

## Test Maintenance

### When to Update Tests:

- **Requirements Change**: Update tests to reflect new requirements
- **Bug Found**: Add test case reproducing the bug
- **Performance Regression**: Add performance test for the scenario
- **New Feature**: Add tests for new tool or functionality

### Test Quality Checks:

- ✓ Tests are isolated (no dependencies between tests)
- ✓ Tests are deterministic (same input → same output)
- ✓ Tests are fast (unit tests <1s, integration <10s)
- ✓ Tests have clear names describing what they test
- ✓ Tests have comments explaining complex scenarios
- ✓ Tests clean up after themselves (no data leaks)

---

## Troubleshooting Tests

### Common Issues:

**1. Tests hang indefinitely**
- Cause: Async fixture not awaited
- Fix: Ensure all async fixtures use `async def` and are awaited

**2. Database locked errors**
- Cause: Concurrent writes to SQLite
- Fix: Use WAL mode, ensure proper transaction handling

**3. Flaky tests (sometimes pass, sometimes fail)**
- Cause: Timing issues, shared state, non-deterministic order
- Fix: Add explicit waits, isolate tests, use deterministic data

**4. Performance tests fail in CI**
- Cause: Shared CI runners, different hardware
- Fix: Relax targets for CI, use dedicated performance testing environment

### Debug Mode:

```bash
# Run with verbose output
pytest tests/unit/mcp/test_task_queue_server.py -vv -s

# Run single test with pdb
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -vv --pdb

# Show local variables on failure
pytest tests/unit/mcp/test_task_queue_server.py -vv -l
```

---

## CI/CD Integration

### GitHub Actions Example:

```yaml
name: Test Task Queue MCP Server

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Set up Python
        uses: actions/setup-python@v2
        with:
          python-version: '3.11'

      - name: Install dependencies
        run: |
          pip install -r requirements-dev.txt

      - name: Run unit tests
        run: |
          pytest tests/unit/mcp/ -v --cov=abathur.mcp --cov-report=xml

      - name: Run integration tests
        run: |
          pytest tests/integration/mcp/ -v

      - name: Upload coverage
        uses: codecov/codecov-action@v2
        with:
          file: ./coverage.xml
```

---

## Next Steps

### Implementation Order:

1. **Create MCP Server Class**: `src/abathur/mcp/task_queue_server.py`
2. **Implement Tool Handlers**: Start with `_handle_task_enqueue`
3. **Run Unit Tests**: Verify each handler as implemented
4. **Add CLI Integration**: `abathur mcp start task-queue`
5. **Run Integration Tests**: Verify end-to-end workflows
6. **Optimize Performance**: Profile and optimize to meet targets
7. **Run Performance Tests**: Verify all targets met

### Development Checklist:

- [ ] Create `AbathurTaskQueueServer` class
- [ ] Implement `_handle_task_enqueue` handler
- [ ] Implement `_handle_task_get` handler
- [ ] Implement `_handle_task_list` handler
- [ ] Implement `_handle_task_queue_status` handler
- [ ] Implement `_handle_task_cancel` handler
- [ ] Implement `_handle_task_execution_plan` handler
- [ ] Add input validation for all handlers
- [ ] Add error handling for all handlers
- [ ] Add logging for all operations
- [ ] Pass all unit tests (>90% coverage)
- [ ] Pass all integration tests
- [ ] Pass all performance tests
- [ ] Add CLI commands (`abathur mcp start/stop/status task-queue`)
- [ ] Update documentation
- [ ] Code review and refactor

---

## References

- **Requirements**: `/design_docs/12-task-queue-mcp/requirements.md`
- **Technical Specs**: `/design_docs/12-task-queue-mcp/technical-specifications.md`
- **MCP Protocol**: https://spec.modelcontextprotocol.io/
- **Python MCP SDK**: https://github.com/modelcontextprotocol/python-sdk
- **Pytest Documentation**: https://docs.pytest.org/
- **Performance Guide**: `tests/performance/README.md`

---

**Last Updated**: 2025-10-11
**Test Suite Version**: 1.0
**Status**: Ready for implementation
