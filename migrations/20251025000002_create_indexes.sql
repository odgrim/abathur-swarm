-- Performance optimization indexes for all tables

-- Sessions table indexes
CREATE INDEX IF NOT EXISTS idx_sessions_status_updated
ON sessions(status, last_update_time DESC)
WHERE status IN ('active', 'paused');

CREATE INDEX IF NOT EXISTS idx_sessions_user_created
ON sessions(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_sessions_project
ON sessions(project_id, created_at DESC)
WHERE project_id IS NOT NULL;

-- Memory entries indexes
CREATE INDEX IF NOT EXISTS idx_memory_namespace_key_version
ON memory_entries(namespace, key, is_deleted, version DESC);

CREATE INDEX IF NOT EXISTS idx_memory_type_updated
ON memory_entries(memory_type, updated_at DESC)
WHERE is_deleted = 0;

CREATE INDEX IF NOT EXISTS idx_memory_namespace_prefix
ON memory_entries(namespace, updated_at DESC)
WHERE is_deleted = 0;

CREATE INDEX IF NOT EXISTS idx_memory_episodic_ttl
ON memory_entries(memory_type, updated_at)
WHERE memory_type = 'episodic' AND is_deleted = 0;

CREATE INDEX IF NOT EXISTS idx_memory_created_by
ON memory_entries(created_by, created_at DESC);

-- Document index indexes
CREATE INDEX IF NOT EXISTS idx_document_type_created
ON document_index(document_type, created_at DESC)
WHERE document_type IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_document_sync_status
ON document_index(sync_status, last_synced_at)
WHERE sync_status IN ('pending', 'stale');

CREATE INDEX IF NOT EXISTS idx_document_content_hash
ON document_index(content_hash);

-- Tasks table indexes
CREATE INDEX IF NOT EXISTS idx_tasks_status_priority
ON tasks(status, priority DESC, submitted_at ASC);

CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
ON tasks(submitted_at);

CREATE INDEX IF NOT EXISTS idx_tasks_parent
ON tasks(parent_task_id)
WHERE parent_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_running_timeout
ON tasks(status, last_updated_at)
WHERE status = 'running';

CREATE INDEX IF NOT EXISTS idx_tasks_session
ON tasks(session_id, submitted_at DESC)
WHERE session_id IS NOT NULL;

-- Agents table indexes
CREATE INDEX IF NOT EXISTS idx_agents_task
ON agents(task_id);

CREATE INDEX IF NOT EXISTS idx_agents_state
ON agents(state);

CREATE INDEX IF NOT EXISTS idx_agents_session
ON agents(session_id, spawned_at DESC)
WHERE session_id IS NOT NULL;

-- Audit table indexes
CREATE INDEX IF NOT EXISTS idx_audit_task
ON audit(task_id, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_audit_agent
ON audit(agent_id, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_audit_timestamp
ON audit(timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_audit_memory_operations
ON audit(memory_operation_type, timestamp DESC)
WHERE memory_operation_type IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_audit_memory_namespace
ON audit(memory_namespace, timestamp DESC)
WHERE memory_namespace IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_audit_memory_entry
ON audit(memory_entry_id, timestamp DESC)
WHERE memory_entry_id IS NOT NULL;

-- State table index (legacy)
CREATE INDEX IF NOT EXISTS idx_state_task_key
ON state(task_id, key);

-- Metrics table index
CREATE INDEX IF NOT EXISTS idx_metrics_name_timestamp
ON metrics(metric_name, timestamp DESC);

-- Checkpoints table index
CREATE INDEX IF NOT EXISTS idx_checkpoints_task
ON checkpoints(task_id, iteration DESC);
