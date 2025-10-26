-- Migration 001: Initial Schema
-- Creates all 8 tables with indexes, constraints, and foreign keys
-- SQLite version: 3.x with WAL mode enabled

-- Enable WAL mode and configure pragmas
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;

-- =============================================================================
-- Table: tasks
-- Purpose: Task queue with priority scheduling and dependencies
-- =============================================================================

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    summary TEXT NOT NULL CHECK(length(summary) <= 140),
    description TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    priority INTEGER NOT NULL CHECK(priority >= 0 AND priority <= 10),
    calculated_priority REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL CHECK(status IN ('pending', 'blocked', 'ready', 'running', 'completed', 'failed', 'cancelled')),
    dependencies TEXT,  -- JSON array of UUIDs
    dependency_type TEXT NOT NULL DEFAULT 'sequential' CHECK(dependency_type IN ('sequential', 'parallel')),
    dependency_depth INTEGER NOT NULL DEFAULT 0,
    input_data TEXT,  -- JSON
    result_data TEXT,  -- JSON
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    max_execution_timeout_seconds INTEGER NOT NULL DEFAULT 3600,
    submitted_at TEXT NOT NULL,  -- ISO 8601
    started_at TEXT,  -- ISO 8601
    completed_at TEXT,  -- ISO 8601
    last_updated_at TEXT NOT NULL,  -- ISO 8601
    created_by TEXT,
    parent_task_id TEXT,
    session_id TEXT,
    source TEXT NOT NULL CHECK(source IN ('human', 'agent_requirements', 'agent_planner', 'agent_implementation')),
    deadline TEXT,  -- ISO 8601
    estimated_duration_seconds INTEGER,
    feature_branch TEXT,
    task_branch TEXT,
    worktree_path TEXT,
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id) ON DELETE SET NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- Indexes for tasks table
CREATE INDEX IF NOT EXISTS idx_tasks_priority_status
    ON tasks(calculated_priority DESC, status);

CREATE INDEX IF NOT EXISTS idx_tasks_status
    ON tasks(status);

CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
    ON tasks(submitted_at);

CREATE INDEX IF NOT EXISTS idx_tasks_deadline
    ON tasks(deadline)
    WHERE deadline IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_parent_task_id
    ON tasks(parent_task_id)
    WHERE parent_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_session_id
    ON tasks(session_id)
    WHERE session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_feature_branch
    ON tasks(feature_branch)
    WHERE feature_branch IS NOT NULL;

-- =============================================================================
-- Table: agents
-- Purpose: Agent lifecycle tracking
-- =============================================================================

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY NOT NULL,
    agent_type TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('idle', 'busy', 'terminated')),
    current_task_id TEXT,
    heartbeat_at TEXT NOT NULL,  -- ISO 8601
    memory_usage_bytes INTEGER NOT NULL DEFAULT 0,
    cpu_usage_percent REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL,  -- ISO 8601
    terminated_at TEXT,  -- ISO 8601
    FOREIGN KEY (current_task_id) REFERENCES tasks(id) ON DELETE SET NULL
);

-- Indexes for agents table
CREATE INDEX IF NOT EXISTS idx_agents_status
    ON agents(status);

CREATE INDEX IF NOT EXISTS idx_agents_heartbeat
    ON agents(heartbeat_at);

-- =============================================================================
-- Table: state
-- Purpose: Shared agent state storage
-- =============================================================================

CREATE TABLE IF NOT EXISTS state (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,  -- JSON
    updated_at TEXT NOT NULL  -- ISO 8601
);

-- Indexes for state table
CREATE INDEX IF NOT EXISTS idx_state_updated_at
    ON state(updated_at);

-- =============================================================================
-- Table: audit
-- Purpose: Operation audit trail
-- =============================================================================

CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation TEXT NOT NULL,
    actor TEXT NOT NULL,
    resource_id TEXT,
    timestamp TEXT NOT NULL,  -- ISO 8601
    metadata TEXT,  -- JSON
    success INTEGER NOT NULL DEFAULT 1 CHECK(success IN (0, 1))
);

-- Indexes for audit table
CREATE INDEX IF NOT EXISTS idx_audit_timestamp
    ON audit(timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_audit_operation
    ON audit(operation);

CREATE INDEX IF NOT EXISTS idx_audit_resource_id
    ON audit(resource_id)
    WHERE resource_id IS NOT NULL;

-- =============================================================================
-- Table: metrics
-- Purpose: Performance metrics
-- =============================================================================

CREATE TABLE IF NOT EXISTS metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    metric_type TEXT NOT NULL,
    value REAL NOT NULL,
    timestamp TEXT NOT NULL,  -- ISO 8601
    tags TEXT  -- JSON
);

-- Indexes for metrics table
CREATE INDEX IF NOT EXISTS idx_metrics_type_timestamp
    ON metrics(metric_type, timestamp DESC);

-- =============================================================================
-- Table: memories
-- Purpose: Memory service storage (semantic/episodic/procedural)
-- =============================================================================

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,  -- JSON
    memory_type TEXT NOT NULL CHECK(memory_type IN ('semantic', 'episodic', 'procedural')),
    version INTEGER NOT NULL DEFAULT 1,
    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK(is_deleted IN (0, 1)),
    metadata TEXT,  -- JSON
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    created_at TEXT NOT NULL,  -- ISO 8601
    updated_at TEXT NOT NULL  -- ISO 8601
);

-- Indexes for memories table
CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_namespace_key
    ON memories(namespace, key);

CREATE INDEX IF NOT EXISTS idx_memories_memory_type
    ON memories(memory_type);

CREATE INDEX IF NOT EXISTS idx_memories_namespace
    ON memories(namespace);

-- =============================================================================
-- Table: sessions
-- Purpose: Conversation sessions
-- =============================================================================

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    app_name TEXT NOT NULL,
    user_id TEXT NOT NULL,
    project_id TEXT,
    state TEXT NOT NULL DEFAULT '{}',  -- JSON
    created_at TEXT NOT NULL,  -- ISO 8601
    updated_at TEXT NOT NULL  -- ISO 8601
);

-- Indexes for sessions table
CREATE INDEX IF NOT EXISTS idx_sessions_user_id
    ON sessions(user_id);

CREATE INDEX IF NOT EXISTS idx_sessions_project_id
    ON sessions(project_id)
    WHERE project_id IS NOT NULL;

-- =============================================================================
-- Table: session_events
-- Purpose: Session event history
-- =============================================================================

CREATE TABLE IF NOT EXISTS session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    content TEXT NOT NULL,  -- JSON
    timestamp TEXT NOT NULL,  -- ISO 8601
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- Indexes for session_events table
CREATE INDEX IF NOT EXISTS idx_session_events_session_timestamp
    ON session_events(session_id, timestamp);
