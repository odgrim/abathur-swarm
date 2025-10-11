# Schema Relationships and ER Diagrams

## 1. Core Entity Relationships

### 1.1 Primary Relationships Diagram

```
┌─────────────┐        1:N         ┌──────────┐
│  sessions   │◄──────────────────│  tasks   │
│             │                    │          │
│ • id        │        1:N         │ •session_│
│ • user_id   │◄──────────────────│  id      │
│ • project_id│                    └──────────┘
│ • events    │        1:N              │
│ • state     │◄──────────────────┐     │ 1:N
└─────────────┘                    │     │
                                   │     ▼
                              ┌────────────┐
                              │  agents    │
                              │            │
                              │ •session_id│
                              │ •task_id   │
                              └────────────┘
```

### 1.2 Memory System Relationships

```
┌──────────────────┐
│  memory_entries  │         Referenced by
│                  │         audit trail
│ • namespace      │◄────────────┐
│ • key            │             │
│ • memory_type    │             │
│ • version        │             │
│ • is_deleted     │             │
└──────────────────┘             │
                                 │
                            ┌────────┐
                            │ audit  │
                            │        │
                            │ •memory│
                            │  _entry│
                            │  _id   │
                            └────────┘
```

### 1.3 Document Index (Standalone)

```
┌────────────────┐
│ document_index │        No direct FK relationships
│                │        (indexed via file_path)
│ • file_path    │
│ • embedding    │        Links to files on disk:
│ • metadata     │        /design_docs/phase1_design/*.md
└────────────────┘
```

## 2. Namespace Hierarchy Relationships

### 2.1 Hierarchical Access Pattern

```
project:<project_id>:                   ┐
  ├── app:<app_name>:                   │ All sessions can READ
  │   └── <key>                         │ (shared application memory)
  │                                     │
  ├── user:<user_id>:                   │ User's sessions can READ/WRITE
  │   ├── preferences                   │ (user-specific memory)
  │   └── history                       │
  │                                     │
  └── session:<session_id>:             │ Session-specific READ/WRITE
      ├── current_task                  │ (ephemeral session state)
      └── temp:<temp_key>               ┘ Never persisted (current turn only)
```

**Access Rules:**
- Sessions READ: `temp:` > `session:` > `user:` > `app:` > `project:`
- Sessions WRITE: `temp:`, `session:`, `user:` only
- `app:` and `project:` require elevated permissions

### 2.2 Memory Entry Namespace Examples

```sql
-- Project-wide shared memory
namespace = 'project:schema_redesign:status'
key = 'architecture_complete'
value = '{"phase": "phase1_design", "approved": true}'

-- User-specific preference
namespace = 'user:alice:preferences'
key = 'communication_style'
value = '{"language": "concise", "technical_level": "expert"}'

-- Session-specific temporary state
namespace = 'session:abc123:progress'
key = 'validation_phase'
value = '{"current_step": 3, "total_steps": 10}'

-- Temp (never persisted to memory_entries, only in sessions.state)
namespace = 'temp:validation'
key = 'errors_found'
value = '[]'  -- Cleared after current turn
```

## 3. Foreign Key Specifications

### 3.1 Foreign Key Definitions

```sql
-- tasks table FKs
FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL

-- agents table FKs
FOREIGN KEY (task_id) REFERENCES tasks(id)
FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL

-- audit table FKs
FOREIGN KEY (agent_id) REFERENCES agents(id)
FOREIGN KEY (task_id) REFERENCES tasks(id)
FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL

-- checkpoints table FKs
FOREIGN KEY (task_id) REFERENCES tasks(id)
FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
```

### 3.2 Cascade Behavior

**ON DELETE SET NULL Rationale:**

When a session is deleted (or archived):
- Tasks persist independently (session_id → NULL)
- Agents persist independently (session_id → NULL)
- Audit trail remains intact (memory_entry_id → NULL, but audit record preserved)

**No CASCADE DELETE:**
- Preserve historical data for audit and analysis
- Enable orphaned task recovery
- Maintain referential integrity without data loss

## 4. Data Flow Diagrams

### 4.1 Task Execution Flow

```
User/System → CREATE task
    ↓
Initialize session (if new)
    ↓
Link task.session_id → sessions.id
    ↓
Agent spawns (agents.session_id, agents.task_id)
    ↓
Agent reads context:
  - tasks table (task details)
  - sessions.state (session state)
  - memory_entries (relevant memories via namespace query)
    ↓
Agent executes task
    ↓
Agent updates:
  - sessions.events (append new event)
  - sessions.state (merge state_delta)
  - memory_entries (create new memories)
  - audit (log all operations)
    ↓
Task completes → tasks.status = 'completed'
    ↓
Session terminates (if no more tasks)
    ↓
Extract learnings → memory_entries (episodic, procedural)
```

### 4.2 Memory Retrieval Flow

```
Agent needs context for task
    ↓
Query memory_entries with namespace hierarchy:
  WHERE namespace LIKE 'temp:%'                    -- Current turn temp data
     OR namespace LIKE 'session:abc123:%'          -- Session-specific state
     OR namespace LIKE 'user:alice:%'              -- User memories
     OR namespace LIKE 'app:abathur:%'             -- App-wide shared
     OR namespace LIKE 'project:schema_redesign:%' -- Project-wide shared
    AND is_deleted = 0                             -- Active memories only
ORDER BY namespace, version DESC                   -- Latest versions first
    ↓
Construct context from retrieved memories
    ↓
Pass context to agent for task execution
```

### 4.3 Memory Consolidation Flow

```
Daily/weekly consolidation job
    ↓
Detect conflicts:
  SELECT namespace, key, COUNT(*) FROM memory_entries
  WHERE is_deleted = 0
  GROUP BY namespace, key
  HAVING COUNT(*) > 1
    ↓
For each conflict:
  - Retrieve all versions
  - Apply resolution strategy (LLM-based or last-write-wins)
  - Create consolidated entry (new version)
  - Mark old versions with consolidated_into metadata
    ↓
Update audit table with consolidation event
```

## 5. Session-Memory Integration

### 5.1 Session State vs Memory Entries

**sessions.state (Short-term):**
- Ephemeral data for active session only
- Fast access (no JOINs, direct JSON parsing)
- Cleared on session termination
- Contains: temp:, session: keys

**memory_entries (Long-term):**
- Persistent data across sessions
- Queryable via namespace hierarchy
- Survives session termination
- Contains: user:, app:, project: keys

**Migration on Session Termination:**

```python
async def terminate_session(session_id: str):
    session = await get_session(session_id)

    # Extract user: keys from state and persist to memory_entries
    for key, value in session.state.items():
        if key.startswith('user:'):
            await create_memory_entry(
                namespace=key.rsplit(':', 1)[0],  # Extract namespace
                key=key.rsplit(':', 1)[-1],       # Extract key
                value=value,
                memory_type='semantic'  # or infer from content
            )

    # Clear session state
    session.state = {}
    session.status = 'terminated'
    session.terminated_at = datetime.now()
    await save_session(session)
```

## 6. Concurrency Relationships

### 6.1 Multi-Session Isolation

```
Session A (user:alice)          Session B (user:bob)
    ↓                                ↓
Read memory_entries:            Read memory_entries:
  - session:A:*                   - session:B:*
  - user:alice:*                  - user:bob:*
  - project:schema_redesign:*     - project:schema_redesign:*
    ↓                                ↓
Write memory_entries:           Write memory_entries:
  - session:A:progress            - session:B:progress
  - user:alice:preferences        - user:bob:preferences
    ↓                                ↓
No conflicts (different namespaces)
```

**Concurrency Control:**
- WAL mode allows concurrent reads from both sessions
- Writes to different namespaces don't block each other
- Version numbers prevent lost updates on same key

### 6.2 Shared Project Memory Conflicts

**Scenario:** Two sessions update same project: memory simultaneously.

```
Session A                       Session B
    ↓                               ↓
Read: project:status            Read: project:status
  version=1                       version=1
    ↓                               ↓
Update to "phase2"              Update to "phase3"
    ↓                               ↓
INSERT version=2                INSERT version=2 (UNIQUE constraint violation)
SUCCESS                         ROLLBACK and retry
    ↓                               ↓
                                Read: project:status
                                  version=2 (sees Session A's update)
                                    ↓
                                UPDATE to "phase3"
                                INSERT version=3
                                SUCCESS
```

**Resolution:** Optimistic locking via UNIQUE(namespace, key, version) constraint forces retry.

## 7. Backward Compatibility Relationships

### 7.1 Deprecated state Table

**Old Pattern (still supported):**
```python
# Old code uses state table
await db.set_state(task_id, "current_phase", {"phase": "validation"})
value = await db.get_state(task_id, "current_phase")
```

**New Pattern (recommended):**
```python
# New code uses sessions.state
session.state["session:abc123:current_phase"] = {"phase": "validation"}
await session_service.append_event(session, event)
```

**Coexistence:**
- Both patterns work simultaneously
- No data corruption (separate storage)
- Gradual migration path

### 7.2 Task-Session Linkage

**Old tasks (no session_id):**
- Still queryable and functional
- session_id = NULL (orphaned tasks)
- Can manually link to session if needed

**New tasks (with session_id):**
- Full session context available
- Rich event history and state
- Memory-assisted execution

---

**Summary:**

This schema design provides:
✅ Clear hierarchical relationships (sessions → tasks → agents)
✅ Flexible namespace hierarchy for memory access control
✅ Proper foreign key constraints with data preservation
✅ Concurrent access patterns with isolation
✅ Backward compatibility with existing tables
✅ Data flow optimized for memory-assisted task execution

---

**Document Version:** 1.0
**Author:** memory-systems-architect (orchestrated)
**Date:** 2025-10-10
**Status:** Phase 1 Design - Awaiting Validation
