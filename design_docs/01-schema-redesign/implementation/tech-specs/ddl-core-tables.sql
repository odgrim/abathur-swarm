-- ================================================================
-- DDL Script: Enhanced Core Tables
-- Purpose: Enhanced existing Abathur tables with memory management integration
-- Phase: Phase 2 Technical Specifications
-- Author: technical-specifications-writer
-- Date: 2025-10-10
-- ================================================================

-- Prerequisites: SQLite 3.35+ with JSON functions support
-- Execution: Run AFTER setting PRAGMA configurations (see implementation-guide.md)
-- Dependencies: Run BEFORE ddl-memory-tables.sql (contains foreign key targets)

-- ================================================================
-- TABLE: tasks (Enhanced)
-- PURPOSE: Store task definitions with session linkage for context
-- CHANGES: Added session_id foreign key for memory-assisted task execution
-- ================================================================

CREATE TABLE IF NOT EXISTS tasks (
    -- Primary identifiers
    id TEXT PRIMARY KEY,                          -- UUID for task
    prompt TEXT NOT NULL,                         -- Natural language task description
    agent_type TEXT NOT NULL DEFAULT 'general',   -- Agent specialization type

    -- Task configuration
    priority INTEGER NOT NULL DEFAULT 5,          -- 1 (highest) to 10 (lowest)
    status TEXT NOT NULL,                         -- pending|running|completed|failed|cancelled

    -- Task data
    input_data TEXT NOT NULL,                     -- JSON: Task input parameters
    result_data TEXT,                             -- JSON: Task execution results
    error_message TEXT,                           -- Error details if status=failed

    -- Retry configuration
    retry_count INTEGER DEFAULT 0,                -- Current retry attempt count
    max_retries INTEGER DEFAULT 3,                -- Maximum retry attempts
    max_execution_timeout_seconds INTEGER DEFAULT 3600,  -- 1 hour default timeout

    -- Timestamps
    submitted_at TIMESTAMP NOT NULL,              -- When task was created
    started_at TIMESTAMP,                         -- When execution began
    completed_at TIMESTAMP,                       -- When execution finished
    last_updated_at TIMESTAMP NOT NULL,           -- Last status update

    -- Relationships
    created_by TEXT,                              -- User or agent ID that created task
    parent_task_id TEXT,                          -- Parent task for hierarchical execution
    dependencies TEXT,                            -- JSON array of task IDs this depends on

    -- NEW: Session linkage for memory context
    session_id TEXT,                              -- Link to conversation session

    -- Foreign key constraints
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- ================================================================
-- TABLE: agents (Enhanced)
-- PURPOSE: Track spawned agents with session context
-- CHANGES: Added session_id foreign key to link agents to conversation sessions
-- ================================================================

CREATE TABLE IF NOT EXISTS agents (
    -- Primary identifiers
    id TEXT PRIMARY KEY,                          -- UUID for agent
    name TEXT NOT NULL,                           -- Agent name (human readable)
    specialization TEXT NOT NULL,                 -- Agent specialization/role

    -- Agent configuration
    task_id TEXT NOT NULL,                        -- Task this agent is executing
    state TEXT NOT NULL,                          -- idle|working|waiting|terminated
    model TEXT NOT NULL,                          -- LLM model (e.g., "claude-sonnet-4")

    -- Lifecycle tracking
    spawned_at TIMESTAMP NOT NULL,                -- Agent creation time
    terminated_at TIMESTAMP,                      -- Agent termination time

    -- Resource tracking
    resource_usage TEXT,                          -- JSON: CPU, memory, token usage

    -- NEW: Session linkage
    session_id TEXT,                              -- Link to conversation session

    -- Foreign key constraints
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- ================================================================
-- TABLE: state (Deprecated but Maintained)
-- PURPOSE: Legacy task state storage (superseded by sessions.state)
-- STATUS: Maintained for backward compatibility, will be removed in v2.1
-- RECOMMENDATION: Use sessions.state JSON column for new code
-- ================================================================

CREATE TABLE IF NOT EXISTS state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,                        -- Task identifier
    key TEXT NOT NULL,                            -- State key
    value TEXT NOT NULL,                          -- JSON-serialized value

    -- Timestamps
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,

    -- Constraints
    UNIQUE(task_id, key),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

-- ================================================================
-- TABLE: audit (Enhanced)
-- PURPOSE: Comprehensive audit logging with memory operation tracking
-- CHANGES: Added memory_operation_type, memory_namespace, memory_entry_id columns
-- ================================================================

CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Event identification
    timestamp TIMESTAMP NOT NULL,                 -- When action occurred
    agent_id TEXT,                                -- Agent that performed action (NULL for system)
    task_id TEXT NOT NULL,                        -- Task context

    -- Action details
    action_type TEXT NOT NULL,                    -- Type of action (e.g., task_create, memory_update)
    action_data TEXT,                             -- JSON: Action-specific data
    result TEXT,                                  -- Action result or status

    -- NEW: Memory operation tracking
    memory_operation_type TEXT,                   -- create|update|delete|consolidate|publish (NULL for non-memory ops)
    memory_namespace TEXT,                        -- Namespace of affected memory (for filtering)
    memory_entry_id INTEGER,                      -- Foreign key to memory_entries.id

    -- Foreign key constraints
    FOREIGN KEY (agent_id) REFERENCES agents(id),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL
);

-- ================================================================
-- TABLE: metrics (Unchanged)
-- PURPOSE: Store performance and operational metrics
-- CHANGES: None required for memory management
-- ================================================================

CREATE TABLE IF NOT EXISTS metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Metric identification
    timestamp TIMESTAMP NOT NULL,                 -- When metric was recorded
    metric_name TEXT NOT NULL,                    -- Metric name (e.g., "task_duration_ms")
    metric_value REAL NOT NULL,                   -- Numeric metric value

    -- Metadata
    labels TEXT,                                  -- JSON: Metric labels/tags

    -- Constraints
    CHECK(metric_value >= 0)                      -- Metrics must be non-negative
);

-- ================================================================
-- TABLE: checkpoints (Enhanced)
-- PURPOSE: Store loop execution checkpoints with session context
-- CHANGES: Added session_id foreign key for session-based checkpointing
-- ================================================================

CREATE TABLE IF NOT EXISTS checkpoints (
    -- Composite primary key
    task_id TEXT NOT NULL,                        -- Task identifier
    iteration INTEGER NOT NULL,                   -- Loop iteration number

    -- Checkpoint data
    state TEXT NOT NULL,                          -- JSON: Checkpoint state
    created_at TIMESTAMP NOT NULL,                -- Checkpoint creation time

    -- NEW: Session linkage (optional)
    session_id TEXT,                              -- Link checkpoint to session

    -- Constraints
    PRIMARY KEY (task_id, iteration),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- ================================================================
-- EXECUTION NOTES
-- ================================================================

-- 1. Run this script AFTER configuring PRAGMA settings:
--    PRAGMA journal_mode = WAL;
--    PRAGMA synchronous = NORMAL;
--    PRAGMA foreign_keys = ON;
--    PRAGMA busy_timeout = 5000;

-- 2. Session table must exist BEFORE running this script (foreign key dependency)
--    Run ddl-memory-tables.sql first, or remove session_id foreign keys temporarily

-- 3. Verify execution with:
--    SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;

-- 4. Check foreign key integrity with:
--    PRAGMA foreign_key_check;

-- 5. All indexes for these tables are in ddl-indexes.sql

-- ================================================================
-- MIGRATION NOTES
-- ================================================================

-- For fresh start projects:
-- - Execute this script directly (no migration required)

-- For existing databases:
-- - Backup database before running: cp abathur.db abathur.db.backup
-- - Add session_id column to tasks: ALTER TABLE tasks ADD COLUMN session_id TEXT;
-- - Add session_id column to agents: ALTER TABLE agents ADD COLUMN session_id TEXT;
-- - Add memory columns to audit:
--   ALTER TABLE audit ADD COLUMN memory_operation_type TEXT;
--   ALTER TABLE audit ADD COLUMN memory_namespace TEXT;
--   ALTER TABLE audit ADD COLUMN memory_entry_id INTEGER;
-- - Add session_id column to checkpoints: ALTER TABLE checkpoints ADD COLUMN session_id TEXT;
-- - Recreate foreign key constraints (requires table recreation in SQLite)

-- ================================================================
-- END OF SCRIPT
-- ================================================================
