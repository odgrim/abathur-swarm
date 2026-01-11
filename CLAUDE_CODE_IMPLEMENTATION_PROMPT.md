# Claude Code Implementation Prompt: Abathur Swarm System

> **Objective**: Implement the complete Abathur self-evolving agentic swarm system from scratch following the design in `design_docs/abathur_implementation.md`. This implementation will be executed by specialized agents working in a coordinated manner.

---

## Overview

Abathur is a self-evolving agentic swarm orchestrator built in Rust. The system consists of 16 phases that build upon each other. This prompt organizes the implementation into agent-assigned work packages that can be executed with clear dependencies and handoffs.

### Available Agents

| Agent | Tier | Primary Responsibilities |
|-------|------|-------------------------|
| `meta-planner` | Meta | Task analysis, capability gap detection, execution planning |
| `rust-architect` | Execution | Project scaffolding, hexagonal architecture, core infrastructure |
| `database-specialist` | Specialist | SQLite schema, migrations, repository implementations |
| `cli-developer` | Execution | CLI commands using clap, output formatting |
| `goal-system-developer` | Execution | Goal domain model, persistence, constraints |
| `task-system-developer` | Execution | Task lifecycle, dependencies, state machine |
| `memory-system-developer` | Execution | Three-tier memory, decay, conflict resolution |
| `agent-system-developer` | Execution | Agent templates, registry, A2A cards |
| `worktree-specialist` | Execution | Git worktrees, branching, merge queue |
| `dag-execution-developer` | Execution | DAG execution engine, wave-based parallelism |
| `orchestration-developer` | Execution | Central orchestrator, swarm lifecycle |
| `substrate-integration-developer` | Execution | LLM backend integration (Claude Code CLI) |
| `meta-planning-developer` | Execution | Meta-planner agent, agent genesis, evolution |
| `a2a-protocol-developer` | Execution | A2A protocol, gateway server, messaging |
| `mcp-integration-developer` | Execution | MCP servers for memory, tasks, A2A |
| `integration-verifier` | Specialist | Integration verification, goal compliance |
| `safety-observability-developer` | Execution | Execution limits, audit logging, monitoring |
| `test-engineer` | Execution | Unit tests, integration tests, property tests |

---

## Implementation Execution Plan

### Phase 1: Foundation (rust-architect, database-specialist, cli-developer)

**Lead Agent**: `rust-architect`

#### Step 1.1: Project Scaffolding
**Agent**: `rust-architect`

```
Task: Initialize the Rust workspace with hexagonal architecture

Actions:
1. Create Cargo.toml with dependencies:
   - clap (4.x) with derive and env features
   - tokio (1.x) with full features
   - serde (1.x) with derive
   - serde_json (1.x)
   - sqlx (0.8.x) with runtime-tokio and sqlite
   - uuid (1.x) with v4 and serde
   - thiserror (2.x)
   - anyhow (1.x)
   - chrono (0.4.x) with serde
   - tracing (0.1.x)
   - tracing-subscriber (0.3.x) with env-filter
   - toml (0.8.x)
   - async-trait (0.1.x)

2. Create module structure:
   src/
   ├── lib.rs
   ├── main.rs
   ├── domain/
   │   ├── mod.rs
   │   ├── models/
   │   │   └── mod.rs
   │   ├── ports/
   │   │   └── mod.rs
   │   └── errors.rs
   ├── adapters/
   │   ├── mod.rs
   │   └── sqlite/
   │       └── mod.rs
   ├── services/
   │   └── mod.rs
   └── cli/
       ├── mod.rs
       └── commands/
           └── mod.rs

3. Set up error handling:
   - DomainError enum using thiserror in domain/errors.rs
   - Application Result type using anyhow

4. Create basic lib.rs re-exporting modules

Success Criteria:
- `cargo build` succeeds
- Module structure matches hexagonal architecture
- Basic error types defined
```

**Handoff**: → `database-specialist` (for Step 1.3), → `cli-developer` (for Step 1.4 framework)

#### Step 1.2: Configuration System
**Agent**: `rust-architect`

```
Task: Implement configuration loading from abathur.toml

Actions:
1. Create src/services/config.rs with Config struct:
   - LimitsConfig (max_depth, max_subtasks, max_descendants)
   - MemoryConfig (decay rates, thresholds)
   - WorktreeConfig (base path, cleanup settings)
   - A2AConfig (gateway port, timeouts)

2. Implement configuration loading:
   - Load from abathur.toml in current directory
   - Support environment variable overrides (ABATHUR_ prefix)
   - Provide sensible defaults

3. Add validation with meaningful error messages

Success Criteria:
- Config loads from TOML file
- Environment overrides work
- Validation catches invalid values
```

#### Step 1.3: Database Layer
**Agent**: `database-specialist`

```
Task: Implement SQLite database infrastructure

Actions:
1. Create migrations/ directory structure:
   migrations/
   └── 001_initial_schema.sql

2. Create initial schema migration with:
   - schema_migrations table for version tracking
   - Placeholder tables (goals, tasks, memories, etc.)

3. Implement database initialization:
   - src/adapters/sqlite/connection.rs - Pool creation
   - src/adapters/sqlite/migrations.rs - Migration runner
   - Database path: .abathur/abathur.db

4. Configure connection pool:
   - Max 5 connections
   - WAL journal mode
   - Foreign keys enabled

Success Criteria:
- Database initializes at .abathur/abathur.db
- Migrations run on startup
- Connection pooling works
```

**Handoff**: → `goal-system-developer`, `task-system-developer`, `memory-system-developer` (for entity-specific tables)

#### Step 1.4: CLI Framework
**Agent**: `cli-developer`

```
Task: Implement CLI framework and init command

Actions:
1. Create CLI structure in src/cli/mod.rs:
   - Top-level Cli struct with clap derive
   - Global --json flag
   - Global --config flag
   - Global -v/--verbose flag
   - Commands enum

2. Implement src/cli/output.rs:
   - CommandOutput trait (to_human, to_json)
   - output() function for mode switching
   - Error formatting for both modes

3. Implement src/cli/commands/init.rs:
   - Create .abathur/ directory (database, worktrees, logs subdirs)
   - Initialize/migrate database
   - Create .claude/ directory
   - Copy baseline agents from template
   - Support --force for reinitialization

4. Set up main.rs entry point:
   - Parse CLI args
   - Initialize tracing
   - Load configuration
   - Route to command handlers

Success Criteria:
- `abathur --help` shows all commands
- `abathur init` creates directory structure
- `abathur init --force` reinitializes
- --json flag works on init output
```

---

### Phase 2: Goal System (goal-system-developer, database-specialist)

**Lead Agent**: `goal-system-developer`

#### Step 2.1: Goal Domain Model
**Agent**: `goal-system-developer`

```
Task: Define Goal entity and related types

Actions:
1. Create src/domain/models/goal.rs:
   - Goal struct with: id, name, description, status, priority, 
     constraints, metadata, parent_id, timestamps
   - GoalStatus enum: Active, Paused, Retired
   - GoalPriority enum: Low, Normal, High, Critical
   - GoalConstraint struct with name, description, type
   - ConstraintType enum: Invariant, Preference, Boundary
   - GoalMetadata with tags and custom fields

2. Implement state machine:
   - valid_transitions() for each status
   - can_transition_to() method
   - is_terminal() helper (Retired only)

3. Implement Goal builder pattern

4. Add validation logic:
   - Name non-empty and <= 255 chars
   - Valid status transitions

Success Criteria:
- All Goal types compile and derive required traits
- State transitions validated
- Builder pattern works
```

**Handoff**: → `test-engineer` (for unit tests)

#### Step 2.2: Goal Persistence
**Agent**: `database-specialist`

```
Task: Implement goal persistence layer

Actions:
1. Create migration for goals table:
   migrations/002_add_goals.sql

2. Create src/domain/ports/goal_repository.rs:
   - GoalFilter struct
   - GoalRepository trait with async methods:
     create, get, update, delete, list,
     get_active_with_constraints, get_tree

3. Create src/adapters/sqlite/repositories/goal_repository.rs:
   - SqliteGoalRepository implementation
   - JSON serialization for constraints/metadata

Success Criteria:
- Goals CRUD operations work
- Filtering by status, priority, parent works
- JSON fields serialize/deserialize correctly
```

#### Step 2.3: Goal CLI Commands
**Agent**: `cli-developer`

```
Task: Implement goal subcommands

Actions:
1. Create src/cli/commands/goal.rs:
   - GoalArgs enum with subcommands
   - Set (create) with name, description, priority, constraints
   - List with filters (status, priority, tree view)
   - Show single goal by ID
   - Pause/Resume/Retire lifecycle commands

2. Implement command handlers:
   - Wire to GoalService
   - Format output for human and JSON modes
   - Handle errors gracefully

3. Add --from-file support for complex goal definitions

Success Criteria:
- All goal commands work
- Both human and JSON output correct
- Error messages helpful
```

#### Step 2.4: Goal Service
**Agent**: `goal-system-developer`

```
Task: Implement goal business logic service

Actions:
1. Create src/services/goal_service.rs:
   - GoalService struct generic over GoalRepository
   - create_goal() with validation and parent check
   - transition_status() with state machine validation
   - get_effective_constraints() aggregating from ancestors

2. Wire service to CLI commands

Success Criteria:
- Business rules enforced
- Constraint inheritance works
- Service testable with mock repository
```

---

### Phase 3: Task System Core (task-system-developer, database-specialist)

**Lead Agent**: `task-system-developer`

#### Step 3.1: Task Domain Model
**Agent**: `task-system-developer`

```
Task: Define Task entity and related types

Actions:
1. Create src/domain/models/task.rs:
   - Task struct with full property set:
     id, parent_id, title, description, goal_id,
     agent_type, routing_hints, depends_on, status,
     priority, retry_count, max_retries, artifacts,
     worktree_path, context, evaluated_constraints, timestamps
   
   - TaskStatus enum: Pending, Ready, Blocked, Running,
     Complete, Failed, Canceled
   
   - TaskPriority enum: Low, Normal, High, Critical
   
   - RoutingHints with preferred_agent, required_tools, complexity
   
   - Complexity enum: Trivial, Simple, Moderate, Complex
   
   - ArtifactRef with uri, type, checksum
   
   - ArtifactType enum
   
   - TaskContext with input, hints, relevant_files, custom

2. Implement state machine:
   - valid_transitions() for each status
   - can_transition_to() method
   - is_terminal() and is_active() helpers
   - Transition guards (dependency check, retry limit)

3. Create TransitionContext for guard evaluation

Success Criteria:
- All Task types defined with proper derives
- State machine validates all transitions
- Guards prevent invalid operations
```

#### Step 3.2: Task Persistence
**Agent**: `database-specialist`

```
Task: Implement task persistence layer

Actions:
1. Create migration:
   migrations/003_add_tasks.sql
   - tasks table with all columns
   - task_dependencies junction table
   - Appropriate indexes

2. Create src/domain/ports/task_repository.rs:
   - TaskFilter struct
   - TaskRepository trait with:
     CRUD operations
     Dependency management (add, remove, get, get_dependents)
     Bulk operations (get_all_dependencies, get_ready_tasks)
     Subtask operations (get_subtasks, count_descendants)

3. Implement SqliteTaskRepository

Success Criteria:
- Task CRUD works
- Dependencies stored and queried correctly
- Ready task detection efficient
```

#### Step 3.3-3.4: State Machine & Dependency Resolution
**Agent**: `task-system-developer`

```
Task: Implement state machine and DAG-based dependency resolver

Actions:
1. Create TaskStateMachine in task.rs:
   - transition() method with guard checks
   - check_guards() for each transition type

2. Create src/services/step_dependency_resolver.rs:
   - DependencyResolver struct
   - would_create_cycle() using DFS
   - find_ready_tasks() 
   - topological_sort() using Kahn's algorithm
   - calculate_waves() for parallel execution groups

3. Implement blocking propagation:
   - When upstream fails, downstream becomes Blocked
   - Recovery when upstream retries

Success Criteria:
- No invalid state transitions allowed
- Cycle detection prevents circular deps
- Wave calculation correct for complex DAGs
```

**Handoff**: → `test-engineer` (property tests for DAG invariants)

#### Step 3.5: Task CLI Commands
**Agent**: `cli-developer`

```
Task: Implement task subcommands

Actions:
1. Create src/cli/commands/task.rs:
   - Submit command with title, description, goal, depends-on, agent
   - List with filters (status, goal, parent)
   - Show with optional --subtasks
   - Cancel with optional --recursive
   - Status showing queue statistics

2. Implement pretty output:
   - Tree view for subtasks
   - Dependency graph visualization (ASCII)
   - Color-coded status indicators

Success Criteria:
- All task commands work
- Dependency visualization clear
- Queue statistics accurate
```

#### Step 3.6: Goal-Task Integration
**Agent**: `task-system-developer`

```
Task: Link tasks to goals and aggregate constraints

Actions:
1. Modify task creation to:
   - Link to active goal if specified
   - Query and store evaluated_constraints from goal hierarchy

2. Create TaskService:
   - create_task() with goal linking
   - Aggregate constraints at creation time

Success Criteria:
- Tasks linked to goals
- Constraints properly inherited
```

---

### Phase 4: Memory System (memory-system-developer, database-specialist)

**Lead Agent**: `memory-system-developer`

#### Step 4.1-4.2: Memory Domain Model & Persistence
**Agent**: `memory-system-developer` + `database-specialist`

```
Task: Implement three-tier memory system

Actions (memory-system-developer):
1. Create src/domain/models/memory.rs:
   - Memory struct with: id, namespace, key, value, memory_type,
     confidence, access_count, state, decay_rate, version,
     parent_id, provenance, timestamps
   
   - MemoryType enum: Semantic, Episodic, Procedural
   
   - MemoryState enum: Active, Cooling, Archived
   
   - Provenance struct with source, task_id, agent, merged_from
   
   - ProvenanceSource enum: ColdStart, Agent, Synthesis, Promotion, User

Actions (database-specialist):
2. Create migration:
   migrations/004_add_memories.sql
   - memories table
   - FTS5 virtual table for search
   - Triggers to keep FTS in sync

3. Implement MemoryRepository trait and SqliteMemoryRepository:
   - CRUD operations
   - Namespace and type filtering
   - Full-text search using FTS5
   - Version history queries
   - Bulk state updates
   - Access recording

Success Criteria:
- All memory types stored correctly
- FTS search works
- Versioning tracks history
```

#### Step 4.3-4.6: Memory Operations, Decay & Conflict Resolution
**Agent**: `memory-system-developer`

```
Task: Implement memory lifecycle features

Actions:
1. Create DecayCalculator in memory.rs:
   - calculate_effective_confidence() with exponential decay
   - should_transition() for state changes
   - process_decay() batch operation

2. Create ConflictResolver:
   - detect_conflict() for same namespace/key
   - find_conflicts() in memory set
   - create_synthesis_request() for resolution

3. Create MemoryPromoter:
   - detect_promotion_candidates() from episodic
   - Pattern detection for failures/successes

4. Implement MemoryService:
   - store/retrieve/update operations
   - Namespace-based queries
   - Semantic search (keyword-based initially)
   - Background decay processing

Success Criteria:
- Decay correctly reduces confidence over time
- Conflicts detected and flagged
- Promotion candidates identified
```

#### Step 4.7: Memory CLI Commands
**Agent**: `cli-developer`

```
Task: Implement memory subcommands

Actions:
1. Create src/cli/commands/memory.rs:
   - List with namespace, type, state filters
   - Show memory details
   - Count with grouping options (by-type, by-state, by-namespace)

2. Add search subcommand for FTS queries

Success Criteria:
- All memory commands work
- Statistics accurate
- Search returns relevant results
```

---

### Phase 5: Agent System Foundation (agent-system-developer, database-specialist)

**Lead Agent**: `agent-system-developer`

#### Step 5.1-5.3: Agent Template Model & Registry
**Agent**: `agent-system-developer` + `database-specialist`

```
Task: Implement agent template system

Actions:
1. Create src/domain/models/agent.rs:
   - AgentTemplate struct with: id, name, tier, version,
     system_prompt, tools, constraints, handoff_targets, max_turns
   
   - AgentTier enum: Meta, Strategic, Execution, Specialist
   
   - AgentCard struct for A2A protocol

2. Create migration:
   migrations/005_add_agents.sql

3. Implement AgentRegistry trait:
   - CRUD for templates
   - Version history tracking
   - Active version queries

4. Load baseline agents from .claude/agents/ directory

Success Criteria:
- Templates stored and versioned
- Baseline agents loaded on init
- Version history preserved
```

#### Step 5.4: Core Agent Templates
**Agent**: `agent-system-developer`

```
Task: Ensure all core agent templates exist

Actions:
1. Verify/create agent definitions in template repository:
   - Meta tier: meta-planner
   - Strategic: product-strategist, requirements-analyst,
     technical-architect, task-decomposer, integration-verifier
   - Execution: code-implementer, test-writer, test-runner,
     documentation-writer, refactorer
   - Specialists: security-auditor, performance-optimizer,
     database-specialist, diagnostic-analyst, etc.

2. Ensure YAML frontmatter format consistent

Success Criteria:
- All core agents defined
- Format validated
- Loaded into registry on init
```

#### Step 5.5: Agent CLI Commands
**Agent**: `cli-developer`

```
Task: Implement agent subcommands

Actions:
1. Create src/cli/commands/agent.rs:
   - List with tier filter
   - Show agent details
   - Cards subcommand: list, show, validate

Success Criteria:
- Agent listing works
- Details display correctly
- Card validation catches errors
```

---

### Phase 6: Artifact System (worktree-specialist, database-specialist)

**Lead Agent**: `worktree-specialist`

#### Step 6.1-6.3: Worktree Domain Model & Git Operations
**Agent**: `worktree-specialist`

```
Task: Implement git worktree infrastructure

Actions:
1. Create src/domain/models/worktree.rs:
   - Worktree struct: id, task_id, path, branch, base_ref, status
   - WorktreeStatus enum: Active, Merged, Orphaned, Failed
   - BranchNaming helper with task_branch(), agent_branch()

2. Create src/domain/ports/git_operations.rs:
   - GitOperations trait with:
     Repo info (root, current branch, default branch)
     Worktree ops (create, remove, list)
     Branch ops (create, delete, exists, checkout)
     Commit ops (commit, has_changes, stage_all)
     Merge ops (merge, rebase, abort)
     Status and diff

3. Create src/adapters/git/shell_git.rs:
   - ShellGitAdapter implementing GitOperations
   - All operations via tokio::process::Command

4. Create WorktreeManager:
   - create_for_task() at .abathur/worktrees/<task-id>/
   - Copy .claude/ to worktree
   - Add .claude/ to worktree's exclude
   - remove() with branch cleanup
   - prune() orphaned worktrees

Success Criteria:
- Worktrees created in correct location
- .claude/ copied and excluded
- Git operations work via shell
```

#### Step 6.4-6.5: Two-Stage Merge Queue & Conflict Resolution
**Agent**: `worktree-specialist`

```
Task: Implement merge queue

Actions:
1. Create MergeQueue:
   - merge_agent_to_task() - Stage 1
   - merge_task_to_main() - Stage 2
   - Retry-with-rebase strategy

2. Handle merge conflicts:
   - Detect conflicts
   - Abort and report
   - Escalation flag for specialist

Success Criteria:
- Two-stage merge works
- Conflicts detected and handled
- Rebase fallback works
```

#### Step 6.6: Worktree CLI Commands
**Agent**: `cli-developer`

```
Task: Implement worktree subcommands

Actions:
1. Create src/cli/commands/worktree.rs:
   - List with status filter
   - Create for task
   - Show details
   - Remove and prune
   - Merge queue commands
   - Status of merge queue

Success Criteria:
- All worktree commands work
- Merge queue visible
```

#### Step 6.7: Artifact URI Scheme
**Agent**: `worktree-specialist`

```
Task: Implement worktree:// URI scheme

Actions:
1. Create ArtifactUri in worktree.rs:
   - parse() extracts task_id and path
   - create() generates URI
   - resolve() maps to filesystem

Success Criteria:
- URIs round-trip correctly
- Resolution finds correct files
```

---

### Phase 7: Substrate Integration (substrate-integration-developer)

**Lead Agent**: `substrate-integration-developer`

#### Step 7.1-7.3: Substrate Trait & Claude Code Adapter
**Agent**: `substrate-integration-developer`

```
Task: Implement LLM backend integration

Actions:
1. Create src/domain/ports/substrate.rs:
   - Substrate trait with:
     invoke(request) -> response
     continue_session(id, message) -> response
     terminate_session(id)
     health_check()
   
   - SubstrateRequest with: request_id, agent_name, task,
     context, system_prompt, tools, max_turns, timeout
   
   - SubstrateResponse with: request_id, session_id, output,
     artifacts, tool_calls, turns_used, status, timing
   
   - CompletionStatus enum: Complete, MaxTurns, Error, Timeout

2. Create src/adapters/claude/mod.rs:
   - ClaudeCodeAdapter implementing Substrate
   - Invoke claude CLI with proper arguments
   - Session isolation per agent
   - Tool binding at invocation time
   - Parse output and extract artifacts

3. Create AgentInvocationService:
   - invoke_agent() builds full request from template + task
   - Handle turn tracking
   - Extract artifacts and handoff requests

Success Criteria:
- Claude CLI invoked correctly
- Sessions isolated
- Artifacts captured
```

---

### Phase 8: DAG Execution Engine (dag-execution-developer)

**Lead Agent**: `dag-execution-developer`

#### Step 8.1-8.4: Execution DAG & Parallel Engine
**Agent**: `dag-execution-developer`

```
Task: Implement DAG execution with parallelism

Actions:
1. Create src/domain/models/dag_execution.rs:
   - ExecutionDag struct with nodes, edges, reverse_edges, sync_points
   - DagNode with task_id, agent_type, priority, state
   - DagNodeState enum
   - from_tasks() builder
   - calculate_waves() for parallel groups
   - get_ready_tasks()

2. Create TaskExecutor:
   - execute() single task flow:
     1. Provision worktree
     2. Determine agent
     3. Build context
     4. Invoke via substrate
     5. Register artifacts
     6. Update state

3. Create ParallelExecutionEngine:
   - execute_dag() with wave-based parallelism
   - execute_wave() with semaphore concurrency
   - Respect concurrency limits
   - Handle completion callbacks
   - Propagate failures

Success Criteria:
- DAG builds from tasks
- Waves calculated correctly
- Parallel execution respects limits
- Failures propagate appropriately
```

#### Step 8.5: Retry Logic
**Agent**: `dag-execution-developer`

```
Task: Implement retry with exponential backoff

Actions:
1. Create RetryConfig:
   - max_retries, initial_delay, max_delay, backoff_multiplier

2. Implement execute_with_retry():
   - Exponential backoff between attempts
   - Add context hints on retry
   - Track retry count

Success Criteria:
- Retries happen with backoff
- Context enriched on retry
- Limit respected
```

---

### Phase 9: Swarm Orchestration (orchestration-developer)

**Lead Agent**: `orchestration-developer`

#### Step 9.1-9.6: Orchestrator Core & Lifecycle
**Agent**: `orchestration-developer`

```
Task: Implement central orchestrator

Actions:
1. Create src/services/orchestrator.rs:
   - Orchestrator struct wiring all services
   - OrchestratorConfig with limits and timeouts
   - OrchestratorState enum: Stopped, Starting, Running, Stopping, Paused
   - RunningAgent tracking struct
   - SwarmEvent enum for broadcast

2. Implement orchestrator lifecycle:
   - start() - initialize and begin main loop
   - stop(force) - graceful or immediate shutdown
   - Main polling loop with select!

3. Implement task dispatch:
   - select_agent() with routing priority
   - calculate_priority_score() for scheduling
   - Spawn execution in background

4. Implement agent lifecycle:
   - Track running agents
   - Health check with heartbeat
   - Terminate on timeout

5. Implement failure handling:
   - CircuitBreaker pattern
   - Retry scheduling
   - Diagnostic escalation

6. Implement deadlock detection:
   - check_stuck_tasks()
   - check_circular_dependencies()
   - check_blocked_chains()
   - Recovery actions

7. Implement state persistence:
   - recover_state() on startup
   - Handle interrupted tasks

Success Criteria:
- Orchestrator starts and stops cleanly
- Tasks dispatched and executed
- Failures handled gracefully
- Deadlocks detected and resolved
```

#### Step 9.7: Swarm CLI Commands
**Agent**: `cli-developer`

```
Task: Implement swarm subcommands

Actions:
1. Create src/cli/commands/swarm.rs:
   - Start with optional --daemon
   - Stop with optional --force
   - Status with detailed metrics

2. Add event streaming for real-time status

Success Criteria:
- Swarm starts in foreground and daemon modes
- Stop graceful and force work
- Status shows live state
```

---

### Phase 10: Meta-Planning & Agent Genesis (meta-planning-developer)

**Lead Agent**: `meta-planning-developer`

#### Step 10.1-10.7: Meta-Planner & Evolution
**Agent**: `meta-planning-developer`

```
Task: Implement self-evolving capabilities

Actions:
1. Implement Meta-Planner logic:
   - Capability analysis for incoming tasks
   - Gap detection (missing agent capabilities)
   - Pipeline selection (Full, Moderate, Simple, Trivial)

2. Implement topology design:
   - Generate task-specific DAGs
   - Support pipeline bypass for simple tasks

3. Implement Agent Genesis:
   - Create new specialist templates
   - Search for similar existing agents
   - Register with version 1.0

4. Implement template versioning:
   - Increment on refinement
   - Snapshot binding at task creation
   - Version history in memory

5. Implement evolution loop:
   - Track success rate per template version
   - Trigger refinement below threshold
   - Automatic reversion on regression

6. Implement spawn limits:
   - Depth tracking (max 5)
   - Per-task subtask limit (max 10)
   - Total descendant limit (max 50)
   - Limit evaluation specialist trigger

7. Implement DAG restructuring:
   - Trigger on permanent failure or spawn limit
   - Re-invoke Meta-Planner with context
   - Restructure limits and cooldown

Success Criteria:
- Meta-planner creates appropriate topologies
- New agents generated when needed
- Evolution improves templates over time
- Limits prevent runaway spawning
```

---

### Phase 11: A2A Protocol (a2a-protocol-developer)

**Lead Agent**: `a2a-protocol-developer`

```
Task: Implement Agent-to-Agent communication

Actions:
1. Implement A2A message format:
   - JSON-RPC 2.0 structures
   - Part types (text, data, file, binary)
   - Message validation

2. Create A2A gateway server:
   - HTTP server for endpoints
   - Agent registration
   - Request routing

3. Implement task operations:
   - tasks/send, tasks/sendStream
   - tasks/get, tasks/cancel
   - Push notification config

4. Implement agent discovery:
   - agent/card endpoint
   - agent/skills endpoint

5. Implement artifact exchange:
   - Worktree artifact handoff
   - Metadata (checksums, etc.)

6. Add streaming support:
   - SSE for status updates
   - Heartbeat mechanism

7. Implement error handling:
   - A2A error codes
   - Retry semantics

Success Criteria:
- Agents can communicate via A2A
- Gateway routes correctly
- Streaming works
```

---

### Phase 12: MCP Integration (mcp-integration-developer)

**Lead Agent**: `mcp-integration-developer`

```
Task: Implement MCP servers for agent access

Actions:
1. Create MCP server framework:
   - HTTP transport
   - Authentication

2. Implement Memory MCP Server:
   - Query, store, update operations
   - abathur mcp memory-http command

3. Implement Tasks MCP Server:
   - Query, submit, status operations
   - abathur mcp tasks-http command

4. Add A2A MCP Gateway:
   - abathur mcp a2a-http command

5. Update CLI:
   - MCP server commands
   - Port configuration

Success Criteria:
- MCP servers run and respond
- Agents can query via MCP
- Authentication works
```

---

### Phase 13: Integration Verification (integration-verifier)

**Lead Agent**: `integration-verifier`

```
Task: Implement integration verification

Actions:
1. Create Integration Verifier logic:
   - Trigger on all subtasks complete
   - Work on task branch (merged subtasks)

2. Implement goal verification:
   - Check constraint satisfaction
   - Holistic goal evaluation
   - Fail on violation

3. Implement integration testing:
   - Run integration tests
   - Cross-component validation
   - Gate merge queue on results

Success Criteria:
- Verification runs automatically
- Constraints checked
- Test failures block merge
```

---

### Phase 14: Specialist Agents (agent-system-developer)

**Lead Agent**: `agent-system-developer`

```
Task: Implement specialist agent templates

Actions:
1. Diagnostic specialists:
   - Diagnostic Analyst
   - Ambiguity Resolver
   - Failure investigation workflow

2. Merge Conflict Specialist:
   - Semantic conflict understanding
   - Integration with conflict resolution flow

3. Limit Evaluation Specialist:
   - Spawn pattern analysis
   - Extension grant logic

4. Domain specialists:
   - Security Auditor
   - Performance Optimizer
   - Database Specialist
   - API Designer
   - DevOps Engineer

Success Criteria:
- All specialist templates defined
- Workflows documented
- Integration points clear
```

---

### Phase 15: Safety & Observability (safety-observability-developer)

**Lead Agent**: `safety-observability-developer`

```
Task: Implement guardrails and monitoring

Actions:
1. Implement execution limits:
   - Turn limit enforcement
   - File change limits
   - Time limits per task
   - Path sandboxing

2. Create audit logging:
   - audit_log table
   - State change logging
   - Decision logging with rationale
   - Actor and timestamp tracking

3. Implement progress tracking:
   - Real-time swarm status
   - Task completion statistics
   - Artifact tracking

4. Enhance CLI observability:
   - Detailed swarm status metrics
   - Task progress visualization
   - Agent activity monitoring

Success Criteria:
- Limits enforced
- All changes audited
- Status visible in real-time
```

---

### Phase 16: Cold Start & Bootstrap (rust-architect, cli-developer)

**Lead Agent**: `rust-architect`

```
Task: Implement initial setup and context gathering

Actions:
1. Create template repository structure:
   - agents/ directory with MD files
   - chains/ for prompt chains
   - settings.json defaults

2. Enhance .claude/ setup in init:
   - Copy baseline agents
   - Configure settings.json
   - Preserve custom on re-init

3. Implement cold start context gathering:
   - Codebase structure analysis
   - Convention detection
   - Dependency analysis
   - Populate initial semantic memories

4. Enhance init command:
   - --template-repo for custom templates
   - --skip-clone option
   - Better progress output

Success Criteria:
- Templates loaded correctly
- Context gathered on init
- Custom templates supported
```

---

## Testing Requirements

**Agent**: `test-engineer` (throughout all phases)

```
Test Coverage Requirements:

1. Unit Tests (each phase):
   - Domain model creation and validation
   - State machine transitions
   - Business logic in services

2. Integration Tests:
   - Repository implementations against SQLite
   - Service interactions
   - CLI command execution

3. Property-Based Tests:
   - DAG invariants (no cycles, topological order)
   - Memory decay monotonicity
   - State machine consistency

4. Mock Implementations:
   - MockGoalRepository, MockTaskRepository, etc.
   - MockSubstrate for execution tests
   - MockGitOperations for worktree tests

5. CLI Tests:
   - Command parsing
   - Output formatting (human and JSON)
   - Error handling

Run tests after each step:
- `cargo test` for unit tests
- `cargo test --test '*'` for integration tests
- `cargo test --features proptest` for property tests
```

---

## Execution Order and Dependencies

```
Phase 1 ─────────────────────────────────────────────────────────►
         │
         ├──► Phase 2 (Goals) ────────────────────────────────────►
         │                                                        │
         ├──► Phase 3 (Tasks) ────────────────────────────────────►
         │         │                                              │
         ├──► Phase 4 (Memory) ───────────────────────────────────►
         │         │                                              │
         └──► Phase 5 (Agents) ───────────────────────────────────►
                   │                                              │
                   └──► Phase 6 (Worktrees) ──────────────────────►
                                              │                   │
Phase 7 (Substrate) ──────────────────────────►                   │
                                              │                   │
                                              └──► Phase 8 (DAG) ─►
                                                              │
                                                              ▼
Phase 9 (Orchestration) ◄─────────────────────────────────────┘
         │
         ├──► Phase 10 (Meta-Planning)
         │
         ├──► Phase 11 (A2A)
         │
         ├──► Phase 12 (MCP)
         │
         ├──► Phase 13 (Integration Verifier)
         │
         ├──► Phase 14 (Specialists)
         │
         └──► Phase 15 (Safety)

Phase 16 (Bootstrap) can begin after Phase 5
```

---

## Key Design Principles to Maintain

Throughout implementation, maintain these invariants:

1. **Never ask questions** - Agents research and proceed
2. **Goals are convergent** - Never complete, always guide
3. **Tasks are discrete** - Have clear completion criteria
4. **Templates are versioned** - Never deleted, always preserved
5. **Worktrees provide isolation** - Primary artifact mechanism
6. **Two-stage merge** - Agent → Task, Task → Main
7. **Success-based evolution** - Only success rate triggers refinement
8. **`.abathur/` for runtime** - Database, worktrees, and logs
9. **`.claude/` for agents** - Agent definitions, copied to worktrees

---

## Starting the Implementation

To begin, invoke the implementation with:

```
I want to implement the Abathur swarm system from scratch following
the design in design_docs/abathur_implementation.md.

Start with Phase 1: Foundation. Use the rust-architect agent to set up
the project scaffolding with hexagonal architecture. Then proceed to
database-specialist for the database layer, and cli-developer for
the CLI framework.

After Phase 1 is complete, proceed through the remaining phases in
order, assigning work to the appropriate specialist agents as documented.

Reference the agent definitions in .claude/agents/ for detailed
responsibilities and handoff criteria.
```

---

## Success Criteria for Complete Implementation

The implementation is complete when:

1. ✅ All 16 phases implemented and tested
2. ✅ `cargo build --release` succeeds with no warnings
3. ✅ `cargo test` passes all tests
4. ✅ `cargo clippy` passes with no warnings
5. ✅ `abathur init` creates proper directory structure
6. ✅ `abathur swarm start` runs and processes tasks
7. ✅ All CLI commands documented with `--help`
8. ✅ JSON output mode works for all commands
9. ✅ Agents can execute tasks via Claude Code
10. ✅ Memory system persists and decays correctly
11. ✅ Worktree isolation works for parallel execution
12. ✅ Meta-planner can create new specialist agents
