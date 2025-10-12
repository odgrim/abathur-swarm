# Task Queue MCP Server - Requirements Specification

**Version:** 1.0
**Date:** 2025-10-11
**Author:** Technical Requirements Analyst
**Status:** Draft

---

## Executive Summary

This document specifies the requirements for an MCP (Model Context Protocol) server that exposes Abathur's task queue operations to Claude agents. The MCP server will enable agents to directly enqueue tasks, query task status, manage task dependencies, and retrieve queue statistics without relying on the TodoWrite tool.

### Objectives

1. **Direct Task Enqueuing**: Enable agents to create tasks in Abathur's task queue via MCP tools
2. **Queue Visibility**: Provide agents with visibility into task queue state and statistics
3. **Dependency Management**: Allow agents to create and manage task dependencies
4. **Seamless Integration**: Integrate with existing TaskQueueService without code duplication
5. **Consistent Experience**: Mirror memory MCP server patterns for consistency

---

## 1. Requirements Traceability Matrix

| Req ID | Category | Requirement | Priority | Maps To | Test Case |
|--------|----------|-------------|----------|---------|-----------|
| FR-001 | Task Enqueuing | Enqueue task with description and parameters | MUST-HAVE | enqueue_task() | TC-001 |
| FR-002 | Task Enqueuing | Support task prerequisites | MUST-HAVE | enqueue_task() | TC-002 |
| FR-003 | Task Enqueuing | Support parent task hierarchy | SHOULD-HAVE | enqueue_task() | TC-003 |
| FR-004 | Task Querying | Get task by ID | MUST-HAVE | get_task() | TC-004 |
| FR-005 | Task Querying | List tasks by status | MUST-HAVE | list_tasks() | TC-005 |
| FR-006 | Task Querying | Query queue statistics | MUST-HAVE | get_queue_status() | TC-006 |
| FR-007 | Task Management | Cancel pending/running task | SHOULD-HAVE | cancel_task() | TC-007 |
| FR-008 | Task Management | Get task execution plan | NICE-TO-HAVE | get_task_execution_plan() | TC-008 |
| FR-009 | Dependency Mgmt | Validate circular dependencies | MUST-HAVE | Dependency validation | TC-009 |
| FR-010 | Error Handling | Return structured error responses | MUST-HAVE | Error formatting | TC-010 |
| NFR-001 | Performance | Tool invocation latency <100ms | MUST-HAVE | Tool handlers | TC-011 |
| NFR-002 | Performance | Queue statistics query <50ms | MUST-HAVE | get_queue_status() | TC-012 |
| NFR-003 | Scalability | Support 100 concurrent tool calls | SHOULD-HAVE | Server architecture | TC-013 |
| NFR-004 | Reliability | 99.9% uptime during swarm execution | MUST-HAVE | Process management | TC-014 |
| NFR-005 | Security | Validate all input parameters | MUST-HAVE | Input validation | TC-015 |
| NFR-006 | Security | Prevent SQL injection | MUST-HAVE | Parameterized queries | TC-016 |
| NFR-007 | Maintainability | Share code with TaskQueueService | MUST-HAVE | Service layer | TC-017 |
| NFR-008 | Observability | Log all tool invocations | SHOULD-HAVE | Logging middleware | TC-018 |
| IR-001 | Integration | Connect to same database as main system | MUST-HAVE | Database integration | TC-019 |
| IR-002 | Integration | Auto-start with swarm orchestrator | SHOULD-HAVE | CLI integration | TC-020 |
| IR-003 | Integration | Configure via MCP config file | MUST-HAVE | Configuration | TC-021 |

---

## 2. Functional Requirements

### 2.1 Task Enqueuing Operations

#### FR-001: Enqueue Task (MUST-HAVE)
**Description:** Allow agents to enqueue a new task into the Abathur task queue.

**Specification:**
- **MCP Tool Name:** `task_enqueue`
- **Input Parameters:**
  - `description` (string, required): Task description/instruction
  - `agent_type` (string, optional, default="requirements-gatherer"): Agent type to execute task
  - `source` (string, required): Task source ("human", "agent_requirements", "agent_planner", "agent_implementation")
  - `base_priority` (integer, optional, default=5, range=[0-10]): User-specified priority
  - `deadline` (ISO 8601 timestamp, optional): Task deadline
  - `estimated_duration_seconds` (integer, optional): Estimated execution time
  - `session_id` (string, optional): Session ID for memory context
  - `input_data` (object, optional): Additional context data

- **Output:**
  - `task_id` (UUID string): Created task ID
  - `status` (string): Initial task status ("ready" or "blocked")
  - `calculated_priority` (float): Calculated priority score
  - `dependency_depth` (integer): Dependency depth in graph

- **Error Conditions:**
  - Invalid priority range (must be 0-10)
  - Invalid source value
  - Invalid agent_type
  - Database connection failure

**Rationale:** Core functionality for agent task delegation. Matches TaskQueueService.enqueue_task() signature.

**Acceptance Criteria:**
- Task successfully inserted into database
- Priority calculation performed
- Initial status determined (READY if no prerequisites)
- Task ID returned to caller
- Execution time <10ms for simple task

---

#### FR-002: Enqueue Task with Prerequisites (MUST-HAVE)
**Description:** Allow agents to enqueue tasks with prerequisite dependencies.

**Specification:**
- **Additional Parameters for `task_enqueue`:**
  - `prerequisites` (array of UUID strings, optional): List of prerequisite task IDs

- **Validation:**
  - All prerequisite task IDs must exist in database
  - Circular dependency detection must pass
  - Prerequisite tasks must be in valid states (not cancelled)

- **Output:**
  - If prerequisites are unmet, status is "blocked"
  - `dependency_depth` reflects position in dependency graph

- **Error Conditions:**
  - Prerequisite task IDs not found (400 error)
  - Circular dependency detected (400 error)
  - Too many dependencies (>100) (400 error)

**Rationale:** Essential for complex multi-step agent workflows. Prevents orphaned blocked tasks.

**Acceptance Criteria:**
- Circular dependencies rejected before database insertion
- Task marked as BLOCKED if prerequisites not completed
- Task dependencies recorded in task_dependencies table
- Dependency depth calculated correctly
- Error messages include cycle path for debugging

---

#### FR-003: Enqueue Child Task (SHOULD-HAVE)
**Description:** Allow agents to enqueue tasks with parent-child hierarchy.

**Specification:**
- **Additional Parameters for `task_enqueue`:**
  - `parent_task_id` (UUID string, optional): Parent task ID

- **Validation:**
  - Parent task ID must exist if provided
  - Parent task should be in RUNNING or READY state

- **Behavior:**
  - Child task inherits session_id from parent if not specified
  - Hierarchical relationship recorded for tracking

**Rationale:** Supports task decomposition patterns. Enables task tree visualization.

**Acceptance Criteria:**
- Parent task reference validated
- Child task linked to parent in database
- Session ID inherited when not specified
- Hierarchical queries supported (get all children of task)

---

### 2.2 Task Querying Operations

#### FR-004: Get Task by ID (MUST-HAVE)
**Description:** Retrieve full task details by task ID.

**Specification:**
- **MCP Tool Name:** `task_get`
- **Input Parameters:**
  - `task_id` (UUID string, required): Task ID

- **Output:** Complete Task object as JSON:
  ```json
  {
    "id": "uuid",
    "prompt": "task description",
    "agent_type": "requirements-gatherer",
    "priority": 5,
    "status": "ready",
    "calculated_priority": 7.5,
    "dependency_depth": 2,
    "source": "agent_planner",
    "prerequisites": ["uuid1", "uuid2"],
    "parent_task_id": "uuid3",
    "session_id": "session-123",
    "submitted_at": "2025-10-11T...",
    "started_at": null,
    "completed_at": null,
    "deadline": null,
    "estimated_duration_seconds": 300,
    "input_data": {...},
    "result_data": null,
    "error_message": null
  }
  ```

- **Error Conditions:**
  - Task not found (404 error)
  - Invalid UUID format (400 error)

**Rationale:** Agents need to check task status and retrieve results.

**Acceptance Criteria:**
- Returns complete task data including all fields
- Query performance <5ms (indexed by ID)
- Error message when task not found
- UUID validation before query

---

#### FR-005: List Tasks by Status (MUST-HAVE)
**Description:** Query tasks filtered by status with pagination.

**Specification:**
- **MCP Tool Name:** `task_list`
- **Input Parameters:**
  - `status` (string, optional): Filter by status ("pending", "blocked", "ready", "running", "completed", "failed", "cancelled")
  - `limit` (integer, optional, default=50, max=500): Maximum results
  - `source` (string, optional): Filter by task source
  - `agent_type` (string, optional): Filter by agent type

- **Output:** Array of Task objects (same structure as FR-004)

- **Error Conditions:**
  - Invalid status value (400 error)
  - Limit exceeds maximum (400 error)

**Rationale:** Enables agents to discover tasks, monitor progress, identify blocked tasks.

**Acceptance Criteria:**
- Status filtering works for all TaskStatus values
- Results sorted by calculated_priority DESC, submitted_at ASC
- Query performance <20ms for 50 results
- Empty array returned when no matches (not error)

---

#### FR-006: Get Queue Statistics (MUST-HAVE)
**Description:** Retrieve aggregated queue statistics for monitoring.

**Specification:**
- **MCP Tool Name:** `task_queue_status`
- **Input Parameters:** None

- **Output:**
  ```json
  {
    "total_tasks": 1234,
    "pending": 10,
    "blocked": 5,
    "ready": 8,
    "running": 3,
    "completed": 1200,
    "failed": 5,
    "cancelled": 3,
    "avg_priority": 6.5,
    "max_depth": 4,
    "oldest_pending": "2025-10-10T...",
    "newest_task": "2025-10-11T..."
  }
  ```

**Rationale:** Provides agents with queue health visibility. Enables workload planning.

**Acceptance Criteria:**
- All status counts accurate
- Average priority calculated correctly
- Query performance <20ms (aggregate queries)
- Timestamps in ISO 8601 format

---

### 2.3 Task Management Operations

#### FR-007: Cancel Task (SHOULD-HAVE)
**Description:** Cancel a pending, blocked, or running task.

**Specification:**
- **MCP Tool Name:** `task_cancel`
- **Input Parameters:**
  - `task_id` (UUID string, required): Task ID to cancel

- **Output:**
  ```json
  {
    "cancelled_task_id": "uuid",
    "cascaded_task_ids": ["uuid1", "uuid2"],
    "total_cancelled": 3
  }
  ```

- **Behavior:**
  - Marks task as CANCELLED
  - Recursively cancels all dependent tasks (cascade)
  - Returns list of all cancelled task IDs

- **Error Conditions:**
  - Task not found (404 error)
  - Task already in terminal state (400 error)
  - Database transaction failure (500 error)

**Rationale:** Agents need to cancel obsolete or incorrect tasks. Prevents orphaned blocked tasks.

**Acceptance Criteria:**
- Task status updated to CANCELLED
- All transitively dependent tasks cancelled
- Cascade completes in <50ms for 10 dependents
- Audit log entry created

---

#### FR-008: Get Task Execution Plan (NICE-TO-HAVE)
**Description:** Calculate execution plan for a set of tasks (topological sort).

**Specification:**
- **MCP Tool Name:** `task_execution_plan`
- **Input Parameters:**
  - `task_ids` (array of UUID strings, required): Tasks to plan

- **Output:**
  ```json
  {
    "batches": [
      ["task-uuid-a"],
      ["task-uuid-b", "task-uuid-c"],
      ["task-uuid-d"]
    ],
    "total_batches": 3,
    "max_parallelism": 2
  }
  ```

- **Error Conditions:**
  - Circular dependency detected (400 error)
  - Task IDs not found (404 error)

**Rationale:** Helps agents understand execution order and parallelization opportunities.

**Acceptance Criteria:**
- Topological sort correctly ordered
- Tasks in same batch have no dependencies between them
- Query performance <30ms for 100 tasks
- Circular dependencies rejected

---

### 2.4 Dependency Management

#### FR-009: Circular Dependency Validation (MUST-HAVE)
**Description:** Validate that new task dependencies don't create cycles.

**Specification:**
- **Integration Point:** Built into `task_enqueue` prerequisite validation
- **Algorithm:** Use DependencyResolver.detect_circular_dependencies()
- **Error Response:**
  ```json
  {
    "error": "CircularDependencyError",
    "message": "Circular dependency detected: A -> B -> C -> A",
    "cycle_path": ["uuid-a", "uuid-b", "uuid-c", "uuid-a"]
  }
  ```

**Rationale:** Prevents deadlocks in task queue. Critical for system reliability.

**Acceptance Criteria:**
- All cycles detected before database insertion
- Cycle path included in error message
- Validation completes in <10ms
- No false positives or false negatives

---

## 3. Non-Functional Requirements

### 3.1 Performance Requirements

#### NFR-001: Tool Invocation Latency (MUST-HAVE)
**Target:** Tool invocation latency <100ms for simple operations

**Measurement:**
- Enqueue simple task: <10ms
- Get task by ID: <5ms
- List tasks (50 results): <20ms
- Queue statistics: <20ms

**Rationale:** MCP tools must feel responsive to agents. Long latencies hurt agent productivity.

**Acceptance Criteria:**
- 95th percentile latency meets targets
- Latencies measured via logging middleware
- Performance tests included in test suite

---

#### NFR-002: Queue Statistics Performance (MUST-HAVE)
**Target:** Queue statistics query <50ms even with 10,000 tasks

**Implementation:**
- Use indexed aggregate queries
- Cache results for 1 second (optional)
- Pre-computed counts via triggers (future optimization)

**Acceptance Criteria:**
- Query plan uses indexes
- Performance regression tests included
- Scales linearly with task count

---

#### NFR-003: Concurrent Tool Call Scalability (SHOULD-HAVE)
**Target:** Support 100 concurrent tool calls without degradation

**Implementation:**
- SQLite WAL mode for concurrent reads
- Async/await for non-blocking I/O
- Connection pooling (if needed)

**Acceptance Criteria:**
- Load test with 100 concurrent agents
- 95th percentile latency <200ms under load
- No database locking errors

---

### 3.2 Reliability Requirements

#### NFR-004: High Availability (MUST-HAVE)
**Target:** 99.9% uptime during swarm execution

**Implementation:**
- Automatic restart on crash
- Health check endpoint
- Graceful degradation on database errors

**Acceptance Criteria:**
- Process supervision via MemoryServerManager pattern
- Automatic restart within 5 seconds of crash
- Monitoring alerts on repeated failures

---

#### NFR-005: Error Recovery (MUST-HAVE)
**Target:** Graceful handling of all error conditions

**Implementation:**
- All exceptions caught and logged
- Structured error responses returned to agents
- Database transactions rolled back on error
- Retry logic for transient failures

**Acceptance Criteria:**
- No unhandled exceptions
- All error paths tested
- Error responses include actionable messages
- Database consistency maintained on all errors

---

### 3.3 Security Requirements

#### NFR-006: Input Validation (MUST-HAVE)
**Target:** Validate all input parameters before processing

**Validation Rules:**
- UUID format validation (RFC 4122)
- Priority range validation (0-10)
- Status enum validation
- Source enum validation
- String length limits (description <10KB)
- Array size limits (prerequisites <100)

**Acceptance Criteria:**
- All parameters validated before database queries
- Validation errors return 400 status
- Input validation tests for all parameters

---

#### NFR-007: SQL Injection Prevention (MUST-HAVE)
**Target:** Zero SQL injection vulnerabilities

**Implementation:**
- Use parameterized queries exclusively
- Never concatenate user input into SQL
- Leverage aiosqlite parameter binding

**Acceptance Criteria:**
- Code review confirms no string concatenation
- Security audit passes
- Penetration testing with SQLMap or similar

---

### 3.4 Maintainability Requirements

#### NFR-008: Code Reuse (MUST-HAVE)
**Target:** Share business logic with TaskQueueService

**Implementation:**
- MCP server uses TaskQueueService for all operations
- No duplicate business logic in MCP layer
- MCP server only handles protocol translation

**Architecture:**
```
Agent -> MCP Tool Call -> MCP Server
                            |
                            v
                       TaskQueueService
                            |
                            v
                         Database
```

**Acceptance Criteria:**
- No business logic in MCP server
- All operations delegate to TaskQueueService
- Code coverage >90% via TaskQueueService tests

---

#### NFR-009: Observability (SHOULD-HAVE)
**Target:** Comprehensive logging and monitoring

**Implementation:**
- Log all tool invocations with parameters
- Log execution time for all operations
- Log error conditions with stack traces
- Export metrics to metrics table

**Log Format:**
```json
{
  "timestamp": "2025-10-11T...",
  "level": "INFO",
  "tool": "task_enqueue",
  "parameters": {...},
  "result": "success",
  "duration_ms": 8.5,
  "agent_session": "session-123"
}
```

**Acceptance Criteria:**
- All tool calls logged
- Structured JSON logging
- Log aggregation compatible (ELK, etc.)
- Metrics queryable via SQL

---

### 3.5 Usability Requirements

#### NFR-010: Error Messages (MUST-HAVE)
**Target:** Clear, actionable error messages for agents

**Error Format:**
```json
{
  "error": "CircularDependencyError",
  "message": "Cannot add task: circular dependency detected",
  "details": {
    "cycle_path": ["uuid-a", "uuid-b", "uuid-c", "uuid-a"],
    "suggestion": "Remove one of the dependencies to break the cycle"
  },
  "timestamp": "2025-10-11T..."
}
```

**Acceptance Criteria:**
- All error types have human-readable messages
- Error messages include troubleshooting hints
- Error responses include error type for programmatic handling

---

## 4. Integration Requirements

### 4.1 Database Integration

#### IR-001: Shared Database Connection (MUST-HAVE)
**Specification:**
- MCP server connects to same SQLite database as main system
- Database path provided via command-line argument or config
- Use same Database class from abathur.infrastructure.database
- Support both file and :memory: databases for testing

**Configuration:**
```bash
# Command line
abathur-mcp-task-queue --db-path ~/.abathur/abathur.db

# Or via MCP config
{
  "mcpServers": {
    "abathur-task-queue": {
      "command": "abathur-mcp-task-queue",
      "args": ["--db-path", "~/.abathur/abathur.db"]
    }
  }
}
```

**Acceptance Criteria:**
- Single source of truth (same database)
- No data synchronization needed
- Database initialized if not exists
- Migrations applied automatically

---

### 4.2 CLI Integration

#### IR-002: Auto-Start with Swarm (SHOULD-HAVE)
**Specification:**
- Add `--no-task-queue-mcp` flag to `abathur swarm start` command
- By default, auto-start task queue MCP server alongside memory MCP server
- Graceful shutdown when swarm stops

**CLI Changes:**
```bash
# Auto-start both memory and task queue MCP servers
abathur swarm start

# Disable auto-start
abathur swarm start --no-mcp

# Start only memory MCP server
abathur swarm start --no-task-queue-mcp
```

**Acceptance Criteria:**
- Task queue MCP server starts automatically with swarm
- Server process supervised and auto-restarted
- Clean shutdown on SIGTERM/SIGINT
- Status visible via `abathur mcp list`

---

### 4.3 MCP Server Management

#### IR-003: MCP Server Lifecycle (MUST-HAVE)
**Specification:**
- Add `abathur mcp start task-queue` command
- Add `abathur mcp stop task-queue` command
- Add `abathur mcp status task-queue` command
- Support foreground and background modes

**Commands:**
```bash
# Start in background
abathur mcp start task-queue

# Start in foreground (for debugging)
abathur mcp start task-queue --foreground

# Stop server
abathur mcp stop task-queue

# Check status
abathur mcp status task-queue
```

**Acceptance Criteria:**
- Server can be started/stopped independently
- PID tracking for background processes
- Health check verifies server responding
- Logs accessible via stdout (foreground) or log file (background)

---

## 5. Constraints and Assumptions

### 5.1 Technical Constraints

**TC-001: SQLite Limitations**
- SQLite has limited concurrency for writes (single writer)
- WAL mode enables concurrent reads but not concurrent writes
- Implication: High write contention may cause delays

**TC-002: MCP Protocol Limitations**
- MCP tools are synchronous (request-response)
- No streaming or long-polling support
- Implication: Agents must poll for task status updates

**TC-003: Python Async Runtime**
- Python asyncio event loop required
- Implication: Server must be async-native throughout

---

### 5.2 Business Constraints

**BC-001: Compatibility**
- Must maintain compatibility with existing TaskQueueService API
- No breaking changes to database schema

**BC-002: Timeline**
- Implementation should complete in 1 sprint (2 weeks)

---

### 5.3 Assumptions

**A-001: Agent Usage Patterns**
- Agents will primarily enqueue tasks and check status
- Write:read ratio approximately 1:5
- Average task enqueue rate: 10/minute per agent

**A-002: Database Size**
- Maximum 100,000 tasks in database
- Maximum 10 concurrent agents
- Database file size <1GB

**A-003: Network Locality**
- MCP server and main system run on same machine
- Database access is local (not remote)

---

## 6. Testing Requirements

### 6.1 Unit Testing

**UT-001: Tool Handler Tests**
- Test each MCP tool independently
- Mock TaskQueueService layer
- Verify input validation
- Verify error handling
- Code coverage >90%

**UT-002: Input Validation Tests**
- Test all parameter validation rules
- Test boundary conditions
- Test malformed input
- Test SQL injection attempts

---

### 6.2 Integration Testing

**IT-001: End-to-End Task Flow**
- Enqueue task via MCP
- Verify task in database
- Query task status
- Cancel task
- Verify cascade cancellation

**IT-002: Dependency Management**
- Enqueue tasks with prerequisites
- Verify circular dependency detection
- Verify dependency resolution
- Verify execution plan calculation

**IT-003: Database Integration**
- Verify shared database access
- Verify foreign key constraints
- Verify transaction isolation
- Verify concurrent access

---

### 6.3 Performance Testing

**PT-001: Latency Benchmarks**
- Measure tool invocation latency
- Measure database query performance
- Verify performance targets met
- Identify bottlenecks

**PT-002: Load Testing**
- Simulate 100 concurrent agents
- Measure throughput (ops/second)
- Measure error rate under load
- Verify graceful degradation

**PT-003: Scalability Testing**
- Test with 10,000 tasks in database
- Test with 100 dependencies per task
- Test with deep dependency graphs (depth 10+)

---

### 6.4 Security Testing

**ST-001: Input Validation**
- Fuzz testing with malformed input
- SQL injection testing
- XSS testing (JSON responses)
- Buffer overflow testing

**ST-002: Access Control**
- Verify no unauthorized database access
- Verify parameter tampering detected

---

## 7. Priority Classification

### MUST-HAVE (P0) - Blocking Launch
- FR-001: Enqueue task
- FR-002: Enqueue with prerequisites
- FR-004: Get task by ID
- FR-005: List tasks by status
- FR-006: Queue statistics
- FR-009: Circular dependency validation
- FR-010: Error handling
- NFR-001: Tool latency <100ms
- NFR-004: High availability
- NFR-005: Input validation
- NFR-006: SQL injection prevention
- NFR-007: Code reuse with TaskQueueService
- IR-001: Database integration
- IR-003: MCP server management

### SHOULD-HAVE (P1) - Important but Not Blocking
- FR-003: Parent task hierarchy
- FR-007: Cancel task
- NFR-003: Concurrent scalability
- NFR-008: Observability logging
- IR-002: Auto-start with swarm

### NICE-TO-HAVE (P2) - Future Enhancement
- FR-008: Task execution plan
- NFR-009: Advanced monitoring
- Performance optimizations (caching, etc.)

---

## 8. Open Questions and Decisions

### Q-001: Task Results Retrieval
**Question:** Should agents be able to retrieve task results via MCP?
**Decision:** DEFERRED - Add in Phase 2 if needed. Current focus is enqueuing.

### Q-002: Task Progress Updates
**Question:** Should running tasks report progress via MCP?
**Decision:** OUT OF SCOPE - Use audit logs for progress tracking.

### Q-003: Bulk Operations
**Question:** Should we support bulk enqueue (multiple tasks at once)?
**Decision:** NICE-TO-HAVE - Implement if performance testing shows need.

### Q-004: Authentication
**Question:** Should MCP server authenticate agents?
**Decision:** OUT OF SCOPE - Rely on MCP transport security. All agents trusted.

### Q-005: Rate Limiting
**Question:** Should we rate-limit tool calls per agent?
**Decision:** DEFERRED - Monitor usage first, add if abuse detected.

---

## 9. Success Metrics

### Launch Criteria
- All MUST-HAVE requirements implemented and tested
- Integration tests passing
- Performance benchmarks meet targets
- Security audit passed
- Documentation complete

### Post-Launch Metrics
- Tool invocation success rate >99%
- Average latency <50ms
- Zero SQL injection vulnerabilities
- Agent adoption rate >80% (measured by TodoWrite replacement)

---

## 10. Dependencies

### Internal Dependencies
- TaskQueueService (src/abathur/services/task_queue_service.py)
- DependencyResolver (src/abathur/services/dependency_resolver.py)
- PriorityCalculator (src/abathur/services/priority_calculator.py)
- Database (src/abathur/infrastructure/database.py)

### External Dependencies
- mcp (Python MCP SDK)
- aiosqlite (async SQLite driver)
- pydantic (data validation)

### Process Dependencies
- Database schema must be stable (no breaking changes)
- TaskQueueService API must be stable

---

## 11. Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Database locking contention under high load | Medium | High | Use WAL mode, connection pooling, batch writes |
| Circular dependency detection performance | Low | Medium | Cache dependency graph, optimize algorithm |
| MCP protocol version incompatibility | Low | High | Pin MCP SDK version, test against Claude Desktop |
| Memory leaks in long-running server | Medium | High | Memory profiling, automated restart on threshold |
| SQLite corruption under concurrent access | Low | Critical | Use WAL mode, fsync properly, backup regularly |

---

## 12. Revision History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2025-10-11 | Technical Requirements Analyst | Initial requirements specification |

---

## Appendix A: MCP Tool Definitions Summary

### Task Queue MCP Tools

1. **task_enqueue**
   - Description: Enqueue a new task into Abathur's task queue
   - Parameters: description, agent_type, source, priority, prerequisites, parent_task_id, deadline, session_id, input_data
   - Returns: task_id, status, calculated_priority, dependency_depth

2. **task_get**
   - Description: Retrieve task details by ID
   - Parameters: task_id
   - Returns: Complete Task object

3. **task_list**
   - Description: List tasks with filtering
   - Parameters: status, limit, source, agent_type
   - Returns: Array of Task objects

4. **task_queue_status**
   - Description: Get queue statistics
   - Parameters: None
   - Returns: Queue statistics object

5. **task_cancel**
   - Description: Cancel task and dependents
   - Parameters: task_id
   - Returns: cancelled_task_id, cascaded_task_ids, total_cancelled

6. **task_execution_plan** (NICE-TO-HAVE)
   - Description: Calculate execution plan for tasks
   - Parameters: task_ids
   - Returns: batches, total_batches, max_parallelism

---

## Appendix B: Database Schema Reference

### Relevant Tables

**tasks**
- id (TEXT PRIMARY KEY)
- prompt (TEXT)
- agent_type (TEXT)
- priority (INTEGER)
- status (TEXT)
- calculated_priority (REAL)
- dependency_depth (INTEGER)
- source (TEXT)
- prerequisites (JSON array)
- parent_task_id (TEXT, FK)
- session_id (TEXT, FK)
- submitted_at (TIMESTAMP)
- started_at (TIMESTAMP)
- completed_at (TIMESTAMP)
- deadline (TIMESTAMP)
- estimated_duration_seconds (INTEGER)
- input_data (JSON)
- result_data (JSON)
- error_message (TEXT)

**task_dependencies**
- id (TEXT PRIMARY KEY)
- dependent_task_id (TEXT, FK)
- prerequisite_task_id (TEXT, FK)
- dependency_type (TEXT)
- created_at (TIMESTAMP)
- resolved_at (TIMESTAMP)

---

## Appendix C: Error Codes Reference

| Error Code | Error Type | HTTP Status | Description |
|------------|------------|-------------|-------------|
| ERR-001 | ValidationError | 400 | Invalid input parameter |
| ERR-002 | NotFoundError | 404 | Task ID not found |
| ERR-003 | CircularDependencyError | 400 | Circular dependency detected |
| ERR-004 | InvalidStateError | 400 | Task in invalid state for operation |
| ERR-005 | DatabaseError | 500 | Database operation failed |
| ERR-006 | TimeoutError | 504 | Operation timed out |
| ERR-007 | ConcurrencyError | 409 | Concurrent modification conflict |

---

## Appendix D: Performance Targets Summary

| Operation | Target Latency | Database Queries | Indexes Used |
|-----------|----------------|------------------|--------------|
| task_enqueue | <10ms | INSERT + SELECT | None |
| task_get | <5ms | SELECT by PK | PK index |
| task_list | <20ms | SELECT with WHERE | idx_tasks_status_priority |
| task_queue_status | <20ms | Aggregate COUNT/AVG | Multiple indexes |
| task_cancel | <50ms | UPDATE + SELECT | idx_task_dependencies_* |
| task_execution_plan | <30ms | Multiple SELECTs | idx_task_dependencies_* |

---

**END OF REQUIREMENTS SPECIFICATION**
