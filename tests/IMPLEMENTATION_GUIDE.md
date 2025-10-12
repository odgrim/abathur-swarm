# Task Queue MCP Server - Implementation Guide

## Overview

This guide provides step-by-step instructions for implementing the Task Queue MCP server based on the comprehensive test suite.

**Test-First Development (TFD) Approach**: All tests are written first and will guide the implementation.

---

## Prerequisites

1. Read the requirements and technical specifications:
   - `/design_docs/12-task-queue-mcp/requirements.md`
   - `/design_docs/12-task-queue-mcp/technical-specifications.md`

2. Review the test suite:
   - `/tests/TEST_SUITE_SUMMARY.md` - Overview of all tests
   - `/tests/unit/mcp/test_task_queue_server.py` - Unit tests (38 tests)
   - `/tests/integration/mcp/test_task_queue_mcp_integration.py` - Integration tests (21 tests)
   - `/tests/performance/test_task_queue_mcp_performance.py` - Performance tests (15 tests)

3. Understand the existing architecture:
   - `TaskQueueService` - Business logic (already implemented)
   - `DependencyResolver` - Dependency graph operations (already implemented)
   - `PriorityCalculator` - Priority calculation (already implemented)
   - `Database` - Data access layer (already implemented)

---

## Implementation Steps

### Phase 1: Create MCP Server Class (2-3 hours)

**File**: `src/abathur/mcp/task_queue_server.py`

#### 1.1 Create Basic Server Structure

```python
"""Task Queue MCP Server for Abathur."""

from pathlib import Path
import logging
from typing import Any

from mcp import server as mcp_server
from mcp.server import Server
from mcp.types import Tool, TextContent

from abathur.infrastructure.database import Database
from abathur.services.task_queue_service import TaskQueueService
from abathur.services.dependency_resolver import DependencyResolver
from abathur.services.priority_calculator import PriorityCalculator

logger = logging.getLogger(__name__)


class AbathurTaskQueueServer:
    """MCP server for Abathur task queue management.

    Exposes task queue operations to Claude agents via MCP protocol.
    """

    def __init__(self, db_path: Path):
        """Initialize task queue server.

        Args:
            db_path: Path to SQLite database
        """
        self.db_path = db_path
        self.db: Database | None = None
        self.task_queue_service: TaskQueueService | None = None
        self.dependency_resolver: DependencyResolver | None = None
        self.priority_calculator: PriorityCalculator | None = None

        # Create MCP server
        self.server = Server("abathur-task-queue")

        logger.info(f"Initializing Task Queue MCP server with db: {db_path}")
```

**Test to pass**: `test_server_initialization`

**Run test**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py::test_server_initialization -v
```

#### 1.2 Initialize Services

```python
async def _initialize_services(self) -> None:
    """Initialize database and service layer."""
    self.db = Database(self.db_path)
    await self.db.initialize()

    self.dependency_resolver = DependencyResolver(self.db)
    self.priority_calculator = PriorityCalculator(self.dependency_resolver)
    self.task_queue_service = TaskQueueService(
        self.db,
        self.dependency_resolver,
        self.priority_calculator
    )

    logger.info("Services initialized successfully")
```

#### 1.3 Register Tools

```python
def _register_tools(self) -> None:
    """Register all MCP tools with server."""

    # Tool 1: task_enqueue
    @self.server.call_tool()
    async def task_enqueue(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_enqueue(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    # Tool 2: task_get
    @self.server.call_tool()
    async def task_get(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_get(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    # Tool 3: task_list
    @self.server.call_tool()
    async def task_list(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_list(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    # Tool 4: task_queue_status
    @self.server.call_tool()
    async def task_queue_status(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_queue_status(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    # Tool 5: task_cancel
    @self.server.call_tool()
    async def task_cancel(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_cancel(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    # Tool 6: task_execution_plan
    @self.server.call_tool()
    async def task_execution_plan(arguments: dict) -> list[TextContent]:
        result = await self._handle_task_execution_plan(arguments)
        return [TextContent(type="text", text=json.dumps(result))]

    logger.info("Registered 6 MCP tools")
```

---

### Phase 2: Implement Tool Handlers (4-6 hours)

For each handler, follow TFD workflow:
1. Read the test
2. Implement handler to pass test
3. Run test
4. Refactor
5. Move to next test

#### 2.1 Implement task_enqueue Handler

**Tests to pass**:
- `test_task_enqueue_success_minimal`
- `test_task_enqueue_success_full_parameters`
- `test_task_enqueue_missing_description`
- `test_task_enqueue_missing_source`
- `test_task_enqueue_invalid_priority_range`
- `test_task_enqueue_invalid_source`
- `test_task_enqueue_invalid_prerequisite_uuid`
- `test_task_enqueue_invalid_parent_uuid`
- `test_task_enqueue_invalid_deadline_format`
- `test_task_enqueue_circular_dependency`
- `test_task_enqueue_prerequisite_not_found`

**Implementation approach**:
1. Validate required parameters (description, source)
2. Validate optional parameters (priority, UUIDs, deadline)
3. Parse and convert types
4. Call `task_queue_service.enqueue_task()`
5. Format response
6. Handle errors

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "task_enqueue" -v
```

#### 2.2 Implement task_get Handler

**Tests to pass**:
- `test_task_get_success`
- `test_task_get_not_found`
- `test_task_get_missing_task_id`
- `test_task_get_invalid_uuid`

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "task_get" -v
```

#### 2.3 Implement task_list Handler

**Tests to pass**:
- `test_task_list_success_no_filters`
- `test_task_list_with_status_filter`
- `test_task_list_with_limit`
- `test_task_list_invalid_status`
- `test_task_list_invalid_limit`
- `test_task_list_empty_result`

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "task_list" -v
```

#### 2.4 Implement task_queue_status Handler

**Tests to pass**:
- `test_queue_status_success`
- `test_queue_status_empty_queue`

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "queue_status" -v
```

#### 2.5 Implement task_cancel Handler

**Tests to pass**:
- `test_task_cancel_success_no_cascade`
- `test_task_cancel_success_with_cascade`
- `test_task_cancel_not_found`
- `test_task_cancel_missing_task_id`
- `test_task_cancel_invalid_uuid`

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "task_cancel" -v
```

#### 2.6 Implement task_execution_plan Handler

**Tests to pass**:
- `test_execution_plan_success`
- `test_execution_plan_empty_task_ids`
- `test_execution_plan_circular_dependency`
- `test_execution_plan_missing_task_ids`
- `test_execution_plan_invalid_task_ids_type`
- `test_execution_plan_invalid_uuid_in_array`

**Run tests**:
```bash
pytest tests/unit/mcp/test_task_queue_server.py -k "execution_plan" -v
```

#### 2.7 Implement Helper Methods

**Helper: `_serialize_task()`**

Tests to pass:
- `test_task_serialize_with_all_fields`
- `test_task_serialize_with_minimal_fields`

**Helper: `_format_error()`**

Test to pass:
- `test_handler_exception_handling`

---

### Phase 3: Run All Unit Tests (30 min)

```bash
# Run all unit tests
pytest tests/unit/mcp/test_task_queue_server.py -v

# With coverage
pytest tests/unit/mcp/test_task_queue_server.py --cov=abathur.mcp.task_queue_server --cov-report=html --cov-report=term-missing

# Target: >90% coverage
```

**Expected result**: All 38 unit tests should pass

---

### Phase 4: Integration Testing (1-2 hours)

Run integration tests to verify end-to-end workflows work with real database.

```bash
# Run all integration tests
pytest tests/integration/mcp/test_task_queue_mcp_integration.py -v

# Run specific workflow
pytest tests/integration/mcp/test_task_queue_mcp_integration.py::test_complete_task_workflow_enqueue_get_complete -v
```

**Expected result**: All 21 integration tests should pass

**Common issues**:
- Database connection errors → Check database initialization
- Async fixture errors → Ensure proper async/await usage
- Transaction errors → Verify commit/rollback logic

---

### Phase 5: Performance Optimization (2-3 hours)

Run performance tests and optimize to meet targets.

```bash
# Run performance tests
pytest tests/performance/test_task_queue_mcp_performance.py -v -s

# Run specific category
pytest tests/performance/ -k "latency" -v -s
```

**Performance targets to meet**:
- Task enqueue: <10ms (P95)
- Task get: <5ms (P99)
- Queue status: <20ms (P95)
- Cancel with 10 deps: <50ms
- Execution plan (100 tasks): <30ms

**Optimization strategies**:
1. **Database Indexes**: Verify indexes are being used
   ```bash
   pytest tests/performance/ -k "explain" -v -s
   ```

2. **Query Optimization**: Use EXPLAIN QUERY PLAN

3. **Caching**: Cache dependency graph if needed

4. **Batch Operations**: Minimize database round trips

**Expected result**: All 15 performance tests should pass

---

### Phase 6: CLI Integration (1-2 hours)

Add CLI commands to manage the MCP server.

**File**: `src/abathur/cli/mcp_commands.py` (or add to existing CLI)

```bash
# Start task queue MCP server
abathur mcp start task-queue [--db-path PATH] [--foreground]

# Stop task queue MCP server
abathur mcp stop task-queue

# Check status
abathur mcp status task-queue

# List all MCP servers
abathur mcp list
```

**Implementation**:
1. Add server manager for task queue MCP server
2. Add CLI commands using Click
3. Support foreground/background modes
4. Add PID tracking for background processes

---

### Phase 7: Documentation (1 hour)

1. **Update README**: Add task queue MCP server to main README
2. **API Documentation**: Document all MCP tools
3. **Examples**: Add usage examples
4. **Configuration**: Document MCP config setup

---

## Testing Strategy

### Development Cycle

For each handler:
```bash
# 1. Read test to understand requirements
cat tests/unit/mcp/test_task_queue_server.py

# 2. Implement handler

# 3. Run specific test
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -v

# 4. If fails, debug
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -vv --pdb

# 5. Once passes, run all related tests
pytest tests/unit/mcp/test_task_queue_server.py -k "task_enqueue" -v

# 6. Refactor and re-run tests
```

### Continuous Testing

```bash
# Watch mode (requires pytest-watch)
pip install pytest-watch
ptw tests/unit/mcp/ -- -v

# Run on file save
```

### Coverage Monitoring

```bash
# Check coverage
pytest tests/unit/mcp/ --cov=abathur.mcp.task_queue_server --cov-report=term-missing

# Generate HTML report
pytest tests/unit/mcp/ --cov=abathur.mcp.task_queue_server --cov-report=html
open htmlcov/index.html
```

---

## Validation Checklist

### Unit Tests
- [ ] All 38 unit tests pass
- [ ] >90% code coverage
- [ ] No unhandled exceptions
- [ ] All error paths tested

### Integration Tests
- [ ] All 21 integration tests pass
- [ ] End-to-end workflows work
- [ ] Database transactions handled correctly
- [ ] Concurrent access works

### Performance Tests
- [ ] All 15 performance tests pass
- [ ] All latency targets met
- [ ] Throughput targets met
- [ ] Scalability verified
- [ ] Concurrent access tested

### Code Quality
- [ ] Code follows project style guide
- [ ] All functions have docstrings
- [ ] Logging added for key operations
- [ ] Error messages are clear and actionable
- [ ] No code duplication

### Documentation
- [ ] README updated
- [ ] API docs complete
- [ ] Usage examples added
- [ ] Configuration documented

---

## Troubleshooting

### Common Issues

**1. Import errors**
```
ModuleNotFoundError: No module named 'abathur.mcp.task_queue_server'
```
**Fix**: Ensure file exists at correct path and is importable

**2. Async fixture errors**
```
TypeError: object NoneType can't be used in 'await' expression
```
**Fix**: Ensure all async fixtures use `async def` and are awaited

**3. Database locked**
```
sqlite3.OperationalError: database is locked
```
**Fix**: Enable WAL mode, ensure proper transaction handling

**4. Performance test failures**
```
AssertionError: P95 latency 15.2ms exceeds 10ms target
```
**Fix**: Profile code, optimize hot paths, check indexes

**5. MCP protocol errors**
```
MCP tool not found: task_enqueue
```
**Fix**: Verify tool registration, check tool names match

### Debug Mode

```bash
# Run with verbose output
pytest tests/unit/mcp/test_task_queue_server.py -vv -s

# Run with debugger on failure
pytest tests/unit/mcp/test_task_queue_server.py --pdb

# Show local variables
pytest tests/unit/mcp/test_task_queue_server.py -l

# Run specific test with full trace
pytest tests/unit/mcp/test_task_queue_server.py::test_task_enqueue_success_minimal -vv --tb=long
```

---

## Timeline Estimate

| Phase | Task | Time Estimate |
|-------|------|---------------|
| 1 | Create server class & structure | 2-3 hours |
| 2 | Implement tool handlers | 4-6 hours |
| 3 | Pass all unit tests | 30 min |
| 4 | Pass integration tests | 1-2 hours |
| 5 | Performance optimization | 2-3 hours |
| 6 | CLI integration | 1-2 hours |
| 7 | Documentation | 1 hour |
| **Total** | **Full implementation** | **12-18 hours** |

**Note**: Times assume familiarity with MCP protocol and existing codebase

---

## Success Criteria

Implementation is complete when:
1. ✅ All 38 unit tests pass
2. ✅ >90% code coverage for MCP server
3. ✅ All 21 integration tests pass
4. ✅ All 15 performance tests pass
5. ✅ All performance targets met
6. ✅ CLI commands work
7. ✅ Documentation complete
8. ✅ Code review passed

---

## Next Steps After Implementation

1. **Manual Testing**: Test with real Claude Desktop client
2. **Load Testing**: Test with 100+ concurrent agents
3. **Security Review**: Audit input validation and error handling
4. **User Testing**: Get feedback from agents using the server
5. **Monitoring**: Set up metrics and alerting
6. **Production Deploy**: Deploy to production environment

---

## Resources

- **MCP Protocol Spec**: https://spec.modelcontextprotocol.io/
- **Python MCP SDK**: https://github.com/modelcontextprotocol/python-sdk
- **Pytest Docs**: https://docs.pytest.org/
- **SQLite Performance**: https://www.sqlite.org/optoverview.html
- **Async Python**: https://docs.python.org/3/library/asyncio.html

---

**Last Updated**: 2025-10-11
**Version**: 1.0
**Status**: Ready for implementation
