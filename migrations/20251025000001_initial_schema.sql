-- Initial schema for Abathur task queue and memory management
-- Run with sqlx migrate run

-- ================================================================
-- Memory Management Tables (must be created first for FK references)
-- ================================================================

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    app_name TEXT NOT NULL,
    user_id TEXT NOT NULL,
    project_id TEXT,
    status TEXT NOT NULL DEFAULT 'created',
    events TEXT NOT NULL DEFAULT '[]',
    state TEXT NOT NULL DEFAULT '{}',
    metadata TEXT DEFAULT '{}',
    created_at TEXT NOT NULL,
    last_update_time TEXT NOT NULL,
    terminated_at TEXT,
    archived_at TEXT,

    CHECK(status IN ('created', 'active', 'paused', 'terminated', 'archived')),
    CHECK(json_valid(events)),
    CHECK(json_valid(state)),
    CHECK(json_valid(metadata))
);

CREATE TABLE IF NOT EXISTS memory_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    metadata TEXT DEFAULT '{}',
    created_by TEXT,
    updated_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    CHECK(memory_type IN ('semantic', 'episodic', 'procedural')),
    CHECK(json_valid(value)),
    CHECK(json_valid(metadata)),
    CHECK(version > 0),
    UNIQUE(namespace, key, version)
);

CREATE TABLE IF NOT EXISTS document_index (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    document_type TEXT,
    content_hash TEXT NOT NULL,
    chunk_count INTEGER DEFAULT 1,
    embedding_model TEXT,
    embedding_blob BLOB,
    metadata TEXT DEFAULT '{}',
    last_synced_at TEXT,
    sync_status TEXT DEFAULT 'pending',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    CHECK(sync_status IN ('pending', 'synced', 'failed', 'stale')),
    CHECK(json_valid(metadata))
);

-- ================================================================
-- Core Tables
-- ================================================================

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT 'general',
    priority INTEGER NOT NULL DEFAULT 5,
    status TEXT NOT NULL,
    input_data TEXT NOT NULL,
    result_data TEXT,
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    max_execution_timeout_seconds INTEGER DEFAULT 3600,
    submitted_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    last_updated_at TEXT NOT NULL,
    created_by TEXT,
    parent_task_id TEXT,
    dependencies TEXT,
    session_id TEXT,

    FOREIGN KEY (parent_task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    specialization TEXT NOT NULL,
    task_id TEXT NOT NULL,
    state TEXT NOT NULL,
    model TEXT NOT NULL,
    spawned_at TEXT NOT NULL,
    terminated_at TEXT,
    resource_usage TEXT,
    session_id TEXT,

    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    UNIQUE(task_id, key),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    agent_id TEXT,
    task_id TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_data TEXT,
    result TEXT,
    memory_operation_type TEXT,
    memory_namespace TEXT,
    memory_entry_id INTEGER,

    FOREIGN KEY (agent_id) REFERENCES agents(id),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (memory_entry_id) REFERENCES memory_entries(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value REAL NOT NULL,
    labels TEXT,

    CHECK(metric_value >= 0)
);

CREATE TABLE IF NOT EXISTS checkpoints (
    task_id TEXT NOT NULL,
    iteration INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    session_id TEXT,

    PRIMARY KEY (task_id, iteration),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);
