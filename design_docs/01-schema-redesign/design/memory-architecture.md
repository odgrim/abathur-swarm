# Memory Architecture Design - Abathur Schema Redesign

## Executive Summary

This document specifies the comprehensive memory management architecture for the Abathur AI agent swarm system. The design incorporates all memory patterns from Chapter 8: Memory Management, including short-term (contextual) and long-term (persistent) memory, session state management, and hierarchical namespace organization.

**Design Principles:**
- **Separation of Concerns:** Short-term (ephemeral session state) vs long-term (persistent memory)
- **Hierarchical Namespaces:** Clear scoping with session:, user:, app:, temp:, project: prefixes
- **Memory Type Taxonomy:** Semantic (facts), Episodic (experiences), Procedural (rules)
- **ACID Compliance:** All critical memory operations are transactional
- **Concurrent Access:** Design supports 50+ concurrent agent sessions
- **Searchability:** Both exact-match (SQL) and semantic similarity (vector embeddings)

---

## 1. Memory Types Taxonomy

### 1.1 Short-Term Memory (Contextual Memory)

**Purpose:** Holds information currently being processed or recently accessed within a single conversation session.

**Implementation:**
- **Storage Location:** `sessions` table → `state` JSON column
- **Scope:** Single session only (isolated between concurrent sessions)
- **Lifetime:** Exists only while session is active, cleared on termination
- **Size Limit:** Constrained by LLM context window (~8K-128K tokens depending on model)
- **Access Pattern:** Fast key-value retrieval within session context

**Use Cases:**
- Current conversation flow and turn tracking
- Temporary task progress indicators
- Active reasoning state and decision trees
- Tool invocation results awaiting processing
- Incremental data collection during multi-turn interaction

**Namespace Prefixes:**
- `temp:<key>` - Exists only for current processing turn, never persisted
- `session:<session_id>:<key>` - Session-specific, cleared on session termination

**Example State Structure:**
```json
{
  "session:abc123:current_task": "schema_redesign",
  "session:abc123:progress": {"steps_completed": 3, "total_steps": 10},
  "temp:validation_needed": true,
  "temp:intermediate_result": {...}
}
```

### 1.2 Long-Term Memory (Persistent Memory)

**Purpose:** Repository for information that must persist across sessions, tasks, and extended time periods.

**Implementation:**
- **Storage Location:** `memory_entries` table
- **Scope:** Hierarchical (project, user, app namespaces)
- **Lifetime:** Varies by memory type (see lifecycle policies below)
- **Size Limit:** 10GB working set, archival for older memories
- **Access Pattern:** Namespace-based retrieval + semantic search

#### 1.2.1 Semantic Memory (Facts & Preferences)

**Definition:** Specific facts, user preferences, domain knowledge, and conceptual understanding.

**Characteristics:**
- **Permanence:** Long-lived, manually managed
- **Update Pattern:** Versioned updates with conflict resolution
- **Structure:** Key-value pairs with optional metadata
- **Search:** Both exact-match and semantic similarity

**Schema Representation:**
```
memory_type = 'semantic'
namespace = 'user:<user_id>:preferences' or 'project:<project_id>:domain_knowledge'
```

**Use Cases:**
- User preferences (language, tone, response length)
- Domain-specific knowledge graphs
- Entity relationships and facts
- Continuously updated user profiles
- Project-specific conventions and standards

**Example Entries:**
```
namespace: user:alice:preferences
key: communication_style
value: {"language": "concise", "technical_level": "expert", "code_comments": true}
memory_type: semantic

namespace: project:schema_redesign:domain_knowledge
key: sqlite_best_practices
value: {"wal_mode": "required", "foreign_keys": "enabled", "pragma_synchronous": "NORMAL"}
memory_type: semantic
```

#### 1.2.2 Episodic Memory (Experiences & Events)

**Definition:** Recollections of past events, actions, and their outcomes - the "what happened" memory.

**Characteristics:**
- **Permanence:** Time-limited (30-90 day TTL, configurable)
- **Update Pattern:** Append-only (events are immutable once recorded)
- **Structure:** Event sequences with timestamps and outcomes
- **Search:** Temporal queries + semantic similarity

**Schema Representation:**
```
memory_type = 'episodic'
namespace = 'user:<user_id>:history' or 'project:<project_id>:task_history'
```

**Use Cases:**
- Task execution history (successes and failures)
- Few-shot learning examples (past successful interaction patterns)
- Debugging context (what was tried before)
- Temporal pattern recognition
- Conversation history across sessions

**Example Entries:**
```
namespace: project:schema_redesign:task_history
key: migration_attempt_2025-10-09
value: {
  "task": "schema_migration",
  "approach": "full_migration",
  "outcome": "failed",
  "error": "data_loss_detected",
  "lesson": "require_rollback_capability"
}
memory_type: episodic
created_at: 2025-10-09T14:32:00Z

namespace: user:alice:interaction_history
key: successful_query_pattern_2025-10-10
value: {
  "query": "Find all tasks with status=pending ordered by priority",
  "sql": "SELECT * FROM tasks WHERE status = 'pending' ORDER BY priority DESC",
  "execution_time_ms": 12,
  "success": true
}
memory_type: episodic
created_at: 2025-10-10T09:15:00Z
```

#### 1.2.3 Procedural Memory (Rules & Instructions)

**Definition:** Knowledge of how to perform tasks - the agent's core instructions, learned behaviors, and operational rules.

**Characteristics:**
- **Permanence:** Long-lived, manually managed (permanent)
- **Update Pattern:** Versioned with reflection-based refinement
- **Structure:** Instructional text, rules, workflows
- **Search:** Semantic retrieval for relevant procedures

**Schema Representation:**
```
memory_type = 'procedural'
namespace = 'app:<app_name>:instructions' or 'project:<project_id>:workflows'
```

**Use Cases:**
- Agent system prompts and core instructions
- Learned best practices and strategies
- Task-specific workflows and procedures
- Error handling patterns
- Reflection-based instruction refinement

**Example Entries:**
```
namespace: app:abathur:agent_instructions
key: schema_redesign_workflow
value: {
  "steps": [
    "1. Analyze existing schema and identify integration points",
    "2. Design new tables with proper foreign key relationships",
    "3. Create migration scripts with rollback capability",
    "4. Test migration on copy of production data",
    "5. Execute migration during maintenance window"
  ],
  "error_handling": "Always create backup before migration",
  "success_criteria": "Zero data loss, <5% performance degradation"
}
memory_type: procedural
version: 3
updated_at: 2025-10-10T10:00:00Z

namespace: project:schema_redesign:reflection_instructions
key: current_agent_instructions
value: {
  "instructions": "When designing database schemas, prioritize ACID compliance and foreign key integrity. Always include rollback procedures. Test on non-production data first.",
  "refined_from_interaction": "session:xyz789",
  "refinement_reason": "Previous migration failed due to missing rollback procedure"
}
memory_type: procedural
version: 2
```

**Reflection-Based Updates:**

Procedural memory supports self-improvement through reflection. Agents periodically:
1. Retrieve current instructions from `app:<app_name>:instructions`
2. Analyze recent interaction history (episodic memory)
3. Prompt LLM to refine instructions based on successes/failures
4. Store updated instructions as new version

---

## 2. Hierarchical Namespace Architecture

### 2.1 Namespace Design Philosophy

Namespaces provide **clear scoping rules** for memory access, preventing data leakage between sessions, users, and projects while enabling controlled sharing.

**Hierarchy Structure:**
```
project:<project_id>:                  # Top-level project scope
  user:<user_id>:                      # User-specific within project
    session:<session_id>:              # Session-specific within user
      temp:<temp_key>                  # Temporary (ephemeral, never persisted)
  app:<app_name>:                      # Application-wide within project
```

**Namespace Prefix Semantics:**

| Prefix | Scope | Persistence | Write Access | Read Access |
|--------|-------|-------------|--------------|-------------|
| `temp:` | Current turn only | Never persisted | Current session | Current session |
| `session:` | Single session | Until session termination | Current session | Current session |
| `user:` | User across sessions | Permanent/versioned | Current user's sessions | User's sessions + elevated |
| `app:` | Application-wide | Permanent/versioned | Elevated permissions | All sessions |
| `project:` | Project-wide | Permanent/versioned | Elevated permissions | Project members |

### 2.2 Namespace Access Rules

**Read Access (Hierarchical Inheritance):**

When a session reads memory, it can access:
1. **temp:** keys - only within current processing turn
2. **session:<current_session_id>:** - session-specific state
3. **user:<current_user_id>:** - user's persistent memories
4. **app:<current_app>:** - application-wide shared memory
5. **project:<current_project>:** - project-scoped shared memory

**Write Access (Restricted):**

- **temp:** - Session can create/update, automatically cleared after turn
- **session:** - Session can create/update, cleared on session termination
- **user:** - Session can create/update user's own memories
- **app:** - Requires elevated permissions (system-level operations only)
- **project:** - Requires project membership + write permissions

**Conflict Resolution:**

When multiple namespaces contain the same key, resolution order:
1. `temp:` (highest precedence - current turn override)
2. `session:` (session-specific override)
3. `user:` (user preference override)
4. `app:` (application default)
5. `project:` (project default)

### 2.3 Namespace Examples

**Example 1: User Preference Override**

```
project:schema_redesign:default_model = "claude-sonnet-4"
user:alice:preferred_model = "claude-opus-4"
session:abc123:override_model = "claude-haiku-4"
```

Resolution: session > user > project → Uses "claude-haiku-4" for this session.

**Example 2: Temporary Validation State**

```
temp:validation_needed = true                    # Cleared after this turn
temp:intermediate_schema_errors = [...]          # Cleared after this turn
session:abc123:validation_history = [...]        # Persists until session ends
user:alice:validation_preferences = {...}        # Persists across sessions
```

**Example 3: Cross-Agent Memory Sharing (Project-Scoped)**

```
project:schema_redesign:memory_architecture_status = "complete"
project:schema_redesign:next_phase = "technical_specifications"
```

All agents working on `project:schema_redesign` can read this shared state.

---

## 3. Session Management Architecture

### 3.1 Session Lifecycle

**Lifecycle States:**

1. **CREATED** - Session initialized, ready for first message
2. **ACTIVE** - Currently processing messages and events
3. **PAUSED** - Temporarily suspended, can be resumed
4. **TERMINATED** - Completed normally, awaiting archival
5. **ARCHIVED** - Historical record only, read-only access

**State Transitions:**
```
CREATED → ACTIVE → (PAUSED ↔ ACTIVE)* → TERMINATED → ARCHIVED
```

**Session Schema (sessions table):**

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,                     -- UUID or session identifier
    app_name TEXT NOT NULL,                  -- Application context
    user_id TEXT NOT NULL,                   -- User identifier
    project_id TEXT,                         -- Optional project association
    status TEXT NOT NULL DEFAULT 'created',  -- Lifecycle state
    events TEXT NOT NULL DEFAULT '[]',       -- JSON array of Event objects
    state TEXT NOT NULL DEFAULT '{}',        -- JSON dict of session state
    created_at TIMESTAMP NOT NULL,
    last_update_time TIMESTAMP NOT NULL,
    terminated_at TIMESTAMP,
    archived_at TIMESTAMP
);
```

### 3.2 Event Tracking

**Event Structure (stored in sessions.events JSON array):**

```json
{
  "event_id": "evt_abc123",
  "timestamp": "2025-10-10T10:00:00Z",
  "event_type": "message|action|tool_call|reflection",
  "actor": "user|agent:<agent_id>|system",
  "content": {
    "message": "User input or agent response",
    "tool_name": "read_file",
    "tool_inputs": {...},
    "tool_outputs": {...},
    "state_delta": {"key": "value"}  // State changes from this event
  },
  "is_final_response": false
}
```

**Event Types:**

- **message** - User input or agent text response
- **action** - Agent action (tool call, function execution)
- **tool_call** - Specific tool invocation with inputs/outputs
- **reflection** - Agent internal reasoning or self-assessment
- **state_change** - Explicit state update event

**Event Append Process:**

1. Session receives new event (user message, agent action)
2. Event appended to `sessions.events` JSON array
3. If event contains `state_delta`, merge into `sessions.state`
4. Update `sessions.last_update_time`
5. Commit transaction (ACID compliance)

### 3.3 State Management

**State Structure (sessions.state JSON dict):**

```json
{
  "session:abc123:current_task": "schema_redesign",
  "session:abc123:progress_steps": [1, 2, 3],
  "temp:awaiting_user_confirmation": true,
  "user:alice:last_interaction": "2025-10-10T09:00:00Z"
}
```

**State Update Mechanisms:**

1. **output_key (Agent Response):**
   - Agent configured with `output_key="last_greeting"`
   - Agent response automatically stored in state: `{"last_greeting": "Hello!"}`

2. **EventActions.state_delta (Complex Updates):**
   ```python
   state_delta = {
       "user:login_count": state.get("user:login_count", 0) + 1,
       "session:task_status": "active",
       "temp:validation_needed": True
   }
   ```

3. **Tool-Based Updates (Recommended):**
   ```python
   def update_session_state(tool_context: ToolContext) -> dict:
       state = tool_context.state
       state["session:phase"] = "validation"
       state["temp:errors_found"] = []
       return {"status": "success"}
   ```

**State Cleanup:**

- **temp:** keys - Cleared immediately after turn processing completes
- **session:** keys - Cleared when session status → TERMINATED
- **user:/app:/project:** keys - Persist permanently in `memory_entries`

---

## 4. Memory Consolidation and Conflict Resolution

### 4.1 Versioning Strategy

**Purpose:** Maintain history of all critical memory changes to enable rollback, conflict resolution, and audit trails.

**Versioned Memory Types:**
- `user:` namespace entries (user preferences, profiles)
- `app:` namespace entries (application configuration, instructions)
- `project:` namespace entries (project state, shared knowledge)

**Version Schema (memory_entries table):**

```sql
CREATE TABLE memory_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,              -- Hierarchical namespace path
    key TEXT NOT NULL,                    -- Memory key
    value TEXT NOT NULL,                  -- JSON-serialized value
    memory_type TEXT NOT NULL,            -- semantic|episodic|procedural
    version INTEGER NOT NULL DEFAULT 1,   -- Version number (increments on update)
    is_deleted BOOLEAN NOT NULL DEFAULT 0,-- Soft-delete flag
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    created_by TEXT,                      -- Session or agent that created this
    updated_by TEXT,                      -- Session or agent that last updated
    UNIQUE(namespace, key, version)       -- Enforce unique versions
);
```

**Version Management:**

- **Create:** Insert with `version=1`
- **Update:** Insert new row with `version=previous_version+1`, mark old row as superseded
- **Delete:** Set `is_deleted=1` on current version (soft-delete)
- **Retrieve:** Query for `MAX(version) WHERE is_deleted=0` for current version
- **Rollback:** Set current version `is_deleted=1`, restore previous version

### 4.2 Conflict Detection

**Conflict Scenarios:**

1. **Simple Update Conflict:**
   - User changes preference from "verbose" to "concise"
   - Later changes to "technical"
   - Resolution: Last-write-wins (version increments)

2. **Contradictory Facts:**
   - Memory A: "User prefers Python"
   - Memory B: "User prefers JavaScript"
   - Resolution: LLM-based consolidation

3. **Temporal Conflicts:**
   - Old episodic memory: "Task failed with approach X"
   - New episodic memory: "Task succeeded with approach X"
   - Resolution: Both retained, semantic search returns most relevant

**Conflict Detection Logic:**

```sql
-- Detect potential conflicts: multiple active versions of same key
SELECT namespace, key, COUNT(*) as version_count
FROM memory_entries
WHERE is_deleted = 0
GROUP BY namespace, key
HAVING COUNT(*) > 1;
```

### 4.3 Conflict Resolution Strategies

**Strategy 1: Last-Write-Wins (Default)**

- Applicable to: Simple preference updates, configuration changes
- Implementation: Version increment, old version remains accessible
- Use Case: User changes theme from "light" to "dark"

**Strategy 2: LLM-Based Consolidation**

- Applicable to: Complex semantic contradictions
- Implementation:
  1. Detect conflict via query above
  2. Retrieve all conflicting versions
  3. Prompt LLM: "Consolidate these conflicting memories: [list]. Provide single coherent entry."
  4. Store consolidated version, mark originals with `consolidated_into=<new_version_id>`

**Example LLM Prompt:**
```
You are consolidating conflicting user memories. Analyze these entries and create a single coherent memory:

Memory 1 (created 2025-10-01): {"preference": "Python", "reason": "clean syntax"}
Memory 2 (created 2025-10-05): {"preference": "JavaScript", "reason": "web development"}

Provide a consolidated memory that resolves the contradiction, preserving context.
```

**Strategy 3: Manual Review Flagging**

- Applicable to: Critical conflicts requiring human judgment
- Implementation:
  1. Set `requires_review=true` flag on conflicting entries
  2. Generate alert for human reviewer
  3. Human resolves conflict via CLI: `abathur memory resolve --conflict-id=123`

### 4.4 Memory Extraction and Consolidation Process

**Automatic Extraction (Session → Memory):**

1. Session reaches TERMINATED state
2. Extract procedural patterns:
   - Identify successful task sequences from events
   - Store as episodic memory with `memory_type='episodic'`
   - Example: "Successfully completed schema_redesign by following steps [...]"

3. Extract semantic facts:
   - Parse state for user preferences expressed during session
   - Store as semantic memory with `memory_type='semantic'`
   - Example: "User prefers detailed explanations with code examples"

4. Update procedural memory:
   - If session involved reflection on instructions
   - Store refined instructions with version increment
   - Example: Updated agent instructions based on session feedback

**Periodic Consolidation (Background Process):**

1. Daily: Scan for duplicate semantic memories across namespaces
2. Weekly: Consolidate episodic memories (summarize similar events)
3. Monthly: Archive old episodic memories (TTL expiration)

---

## 5. Cross-Agent Memory Sharing

### 5.1 Project-Scoped Sharing Model

**Design Principle:** Agents working on the same project share a common memory pool via `project:<project_id>:` namespace.

**Access Control:**

- **Read Access:** All agents assigned to `project_id` can read `project:<project_id>:*`
- **Write Access:** Requires `project_write_permission` flag (prevents accidental overwrites)
- **Isolation:** Different projects have completely separate namespaces

**Example Project Memory:**

```
namespace: project:schema_redesign:architecture_complete
key: memory_system_design
value: {
  "status": "approved",
  "architect": "agent:memory-systems-architect",
  "deliverables": [
    "/path/to/memory-architecture.md",
    "/path/to/schema-tables.md"
  ],
  "approval_timestamp": "2025-10-10T12:00:00Z"
}
memory_type: procedural
```

All agents in project `schema_redesign` can now read this status and know the memory architecture is complete.

### 5.2 Agent-Type-Scoped Sharing (Optional Extension)

**Use Case:** Specialist agents of the same type share learned patterns.

**Implementation:**

```
namespace: app:abathur:agent_type:database-redesign-specialist:best_practices
key: migration_checklist
value: ["backup_first", "test_on_copy", "rollback_ready", "performance_benchmark"]
memory_type: procedural
```

All `database-redesign-specialist` agents can access this shared procedural memory.

### 5.3 Memory Publication Workflow

**Explicit Sharing (Recommended for Critical Memories):**

1. Agent creates memory in `session:` namespace (private)
2. Agent validates memory quality and relevance
3. Agent publishes to `project:` or `user:` namespace (shared)
4. Audit log records publication event

**Example API:**
```python
# Create session-private memory
memory_service.create(
    namespace="session:abc123:draft",
    key="schema_proposal",
    value={"tables": [...], "indexes": [...]},
    memory_type="procedural"
)

# Validate and publish to project scope
memory_service.publish(
    from_namespace="session:abc123:draft",
    to_namespace="project:schema_redesign:approved",
    key="schema_proposal"
)
```

---

## 6. Integration with Task Execution Context

### 6.1 Task-Memory Linkage

**Design:** Tasks are executed within sessions, sessions contain state and link to memories.

**Schema Relationships:**

```
tasks.session_id → sessions.id       # Task belongs to session
sessions.state → Contains temp: and session: keys
memory_entries.namespace → Contains user: and project: keys
```

**Task Execution Flow:**

1. **Task Created:** `INSERT INTO tasks (id, session_id, ...)`
2. **Session Initialized:** `INSERT INTO sessions (id, project_id, user_id, ...)`
3. **Agent Execution:**
   - Reads task context from `tasks` table
   - Reads session state from `sessions.state`
   - Reads relevant memories from `memory_entries` (filtered by namespace)
   - Executes task logic
   - Updates session state and appends events
   - Creates new memories if learned something valuable

4. **Task Completion:**
   - Update `tasks.status = 'completed'`
   - Extract learnings to `memory_entries` (episodic: what happened, procedural: what worked)
   - Session transitions to TERMINATED (or remains ACTIVE for multi-task sessions)

### 6.2 Context Synthesis for Agent Prompts

**Memory-Assisted Prompt Construction:**

When agent begins task, synthesize context from multiple sources:

```python
def synthesize_context(session_id: str, task_id: str) -> str:
    # 1. Retrieve session state
    session = get_session(session_id)
    session_state = session.state

    # 2. Retrieve task details
    task = get_task(task_id)

    # 3. Query relevant memories
    user_memories = query_memories(namespace=f"user:{session.user_id}", limit=10)
    project_memories = query_memories(namespace=f"project:{session.project_id}", limit=20)
    app_memories = query_memories(namespace=f"app:abathur", limit=5)

    # 4. Semantic search for task-relevant episodic memories
    relevant_episodes = semantic_search(
        query=task.prompt,
        namespace=f"project:{session.project_id}",
        memory_type="episodic",
        limit=5
    )

    # 5. Construct context
    context = f"""
    Current Task: {task.prompt}
    Session State: {json.dumps(session_state)}

    User Preferences (from semantic memory):
    {format_memories(user_memories)}

    Project Context:
    {format_memories(project_memories)}

    Relevant Past Experiences (episodic):
    {format_memories(relevant_episodes)}

    Application Instructions (procedural):
    {format_memories(app_memories)}
    """

    return context
```

This creates a rich, memory-assisted prompt that includes:
- Current task and session state (short-term)
- User preferences (semantic long-term)
- Project-specific knowledge (semantic long-term)
- Relevant past experiences (episodic long-term)
- Learned procedures and best practices (procedural long-term)

---

## 7. Memory Lifecycle Policies

### 7.1 Retention Policies by Memory Type

| Memory Type | Retention Policy | Cleanup Trigger | Archival Strategy |
|-------------|------------------|-----------------|-------------------|
| temp: | Immediate (end of turn) | After event processing | Never archived (ephemeral) |
| session: | Session lifetime | session.status = TERMINATED | Archived with session |
| Semantic | Permanent (manual deletion) | User-initiated or admin cleanup | Move to cold storage after 1 year |
| Episodic | 30-90 days (configurable TTL) | Daily cleanup job | Summarize and compress |
| Procedural | Permanent (versioned) | Never (keeps all versions) | Old versions to cold storage |

### 7.2 Automatic Cleanup Processes

**Daily Cleanup Job:**

```sql
-- Mark expired episodic memories for archival
UPDATE memory_entries
SET is_deleted = 1
WHERE memory_type = 'episodic'
  AND updated_at < datetime('now', '-90 days')
  AND is_deleted = 0;

-- Archive terminated sessions older than 30 days
UPDATE sessions
SET status = 'archived', archived_at = datetime('now')
WHERE status = 'terminated'
  AND terminated_at < datetime('now', '-30 days');
```

**Weekly Consolidation Job:**

```sql
-- Identify similar episodic memories for consolidation
SELECT namespace,
       GROUP_CONCAT(id) as memory_ids,
       COUNT(*) as duplicate_count
FROM memory_entries
WHERE memory_type = 'episodic'
  AND is_deleted = 0
GROUP BY namespace, substr(value, 1, 100)  -- Crude similarity check
HAVING COUNT(*) > 3;
```

### 7.3 Manual Memory Management Commands

**CLI Commands:**

```bash
# List all user memories
abathur memory list --namespace="user:alice" --type=semantic

# Delete specific memory
abathur memory delete --namespace="user:alice:preferences" --key="old_preference"

# Archive old episodic memories
abathur memory archive --type=episodic --older-than=90d

# Consolidate conflicting memories
abathur memory consolidate --namespace="user:alice" --auto

# Restore deleted memory (rollback soft-delete)
abathur memory restore --id=12345

# Export memories for backup
abathur memory export --namespace="project:schema_redesign" --output=backup.json
```

---

## 8. Performance Considerations

### 8.1 Index Strategy (See schema-indexes.md for complete DDL)

**Critical Indexes:**

1. **Namespace Hierarchy Index:** `CREATE INDEX idx_memory_namespace ON memory_entries(namespace, key, is_deleted, version DESC);`
   - Optimizes hierarchical namespace queries
   - Enables fast retrieval of current version

2. **Memory Type Index:** `CREATE INDEX idx_memory_type_updated ON memory_entries(memory_type, updated_at DESC) WHERE is_deleted = 0;`
   - Supports queries filtered by memory type
   - Partial index excludes soft-deleted entries

3. **Temporal Index:** `CREATE INDEX idx_memory_temporal ON memory_entries(updated_at DESC) WHERE memory_type = 'episodic';`
   - Fast retrieval of recent episodic memories
   - Supports TTL cleanup queries

4. **Session State Index:** `CREATE INDEX idx_session_status_updated ON sessions(status, last_update_time DESC);`
   - Quickly find active/paused sessions
   - Supports cleanup of terminated sessions

### 8.2 Query Optimization Patterns

**Pattern 1: Current Version Retrieval**

```sql
-- Inefficient: Full table scan with MAX()
SELECT * FROM memory_entries
WHERE namespace = 'user:alice:preferences'
  AND key = 'theme'
  AND version = (SELECT MAX(version) FROM memory_entries WHERE namespace = 'user:alice:preferences' AND key = 'theme');

-- Efficient: Index-optimized with ORDER BY + LIMIT
SELECT * FROM memory_entries
WHERE namespace = 'user:alice:preferences'
  AND key = 'theme'
  AND is_deleted = 0
ORDER BY version DESC
LIMIT 1;
```

**Pattern 2: Namespace Hierarchy Query**

```sql
-- Retrieve all memories accessible to session (hierarchical)
SELECT * FROM memory_entries
WHERE (
    namespace LIKE 'session:abc123:%' OR
    namespace LIKE 'user:alice:%' OR
    namespace LIKE 'project:schema_redesign:%' OR
    namespace LIKE 'app:abathur:%'
)
AND is_deleted = 0
ORDER BY namespace, version DESC;
```

**Pattern 3: Semantic Search (Post-Infrastructure)**

```sql
-- Vector similarity search using sqlite-vss (future)
SELECT me.*, vss_distance(de.embedding, ?) as distance
FROM memory_entries me
JOIN document_embeddings de ON me.id = de.memory_entry_id
WHERE me.memory_type = 'semantic'
  AND me.is_deleted = 0
ORDER BY distance ASC
LIMIT 10;
```

### 8.3 Concurrency Design

**WAL Mode Configuration:**

```sql
PRAGMA journal_mode = WAL;          -- Concurrent reads + single writer
PRAGMA synchronous = NORMAL;         -- Balance safety and performance
PRAGMA busy_timeout = 5000;          -- Wait 5s for locks
PRAGMA wal_autocheckpoint = 1000;    -- Checkpoint every 1000 pages
```

**Concurrent Access Patterns:**

- **50+ Reader Sessions:** WAL mode allows unlimited concurrent reads
- **Single Writer:** SQLite serializes writes, but 5s busy_timeout prevents failures
- **Session Isolation:** Each session operates on its own `session:` namespace (no contention)
- **Optimistic Locking:** Version numbers prevent lost updates

**Bottleneck Mitigation:**

- **Read-Heavy Workload:** Excellent (WAL mode)
- **Write-Heavy Workload:** Batch inserts, async memory extraction
- **Mixed Workload:** Separate read replicas (future) for analytics queries

---

## 9. Summary and Next Steps

### 9.1 Architecture Summary

This memory architecture provides:

✅ **Complete Memory Type Coverage:** Short-term (session state), long-term (semantic, episodic, procedural)
✅ **Hierarchical Namespaces:** Clear scoping with temp:, session:, user:, app:, project: prefixes
✅ **Session Management:** Complete lifecycle with event tracking and state isolation
✅ **Versioning & Conflict Resolution:** Soft-delete versioning + LLM-based consolidation
✅ **Cross-Agent Sharing:** Project-scoped sharing with access control
✅ **Memory Lifecycle:** Automatic TTL cleanup + manual management
✅ **Performance:** Indexed for <50ms reads, <500ms semantic search (post-infrastructure)
✅ **Concurrency:** Designed for 50+ concurrent sessions
✅ **Auditability:** Comprehensive event and memory operation logging

### 9.2 Implementation Readiness

**Phase 1 Complete (This Document):** Memory architecture design
**Phase 2 Required:** Technical specifications (DDL, APIs, query patterns)
**Phase 3 Required:** Implementation plan (rollout, testing, deployment)

### 9.3 Open Questions for Validation Gate

1. **Vector Search Prioritization:** Should sqlite-vss integration be Phase 1 or Phase 2?
   - Recommendation: Phase 2 (schema supports it, defer infrastructure)

2. **Memory Consolidation Frequency:** Daily, weekly, or on-demand?
   - Recommendation: Weekly automated + on-demand CLI command

3. **Session Archival Strategy:** How long to retain archived sessions?
   - Recommendation: 1 year in SQLite, then export to cold storage

4. **Cross-Project Memory Sharing:** Should users share memories across projects?
   - Recommendation: User memories accessible across all projects, project memories isolated

### 9.4 Validation Checklist

- [ ] All memory types from Chapter 8 addressed (short-term, long-term, semantic, episodic, procedural)
- [ ] Hierarchical namespace design with clear scoping rules (temp:, session:, user:, app:, project:)
- [ ] Session management with event tracking and state isolation
- [ ] Memory consolidation and conflict resolution strategies (versioning + LLM-based)
- [ ] Cross-agent memory sharing via project: namespace
- [ ] Memory lifecycle policies (TTL, archival, cleanup)
- [ ] Performance optimization strategies (indexes, query patterns)
- [ ] Concurrency support for 50+ agents (WAL mode)
- [ ] Integration with task execution context
- [ ] Complete and implementable design

---

**Document Version:** 1.0
**Author:** memory-systems-architect (orchestrated)
**Date:** 2025-10-10
**Status:** Phase 1 Design - Awaiting Validation Gate
**Next Document:** schema-tables.md (Complete DDL specifications)
