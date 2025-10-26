-- Migration 002: Python Schema Compatibility Layer
-- Provides views and compatibility structures for migrating data from Python implementation
-- This migration facilitates the transition from Python SQLite DB to Rust SQLite DB

-- =============================================================================
-- Compatibility Views
-- =============================================================================

-- View: v_tasks_python_compatible
-- Purpose: Provides a view that matches Python's task table structure for migration scripts
CREATE VIEW IF NOT EXISTS v_tasks_python_compatible AS
SELECT
    id,
    summary,
    description,
    agent_type,
    priority,
    calculated_priority,
    status,
    dependencies,
    dependency_type,
    dependency_depth,
    input_data,
    result_data,
    error_message,
    retry_count,
    max_retries,
    max_execution_timeout_seconds,
    submitted_at,
    started_at,
    completed_at,
    last_updated_at,
    created_by,
    parent_task_id,
    session_id,
    source,
    deadline,
    estimated_duration_seconds,
    feature_branch,
    task_branch,
    worktree_path
FROM tasks;

-- View: v_agents_python_compatible
-- Purpose: Provides a view for agent data migration
CREATE VIEW IF NOT EXISTS v_agents_python_compatible AS
SELECT
    id,
    agent_type,
    status,
    current_task_id,
    heartbeat_at,
    memory_usage_bytes,
    cpu_usage_percent,
    created_at,
    terminated_at
FROM agents;

-- View: v_memories_python_compatible
-- Purpose: Provides a view for memory data migration
CREATE VIEW IF NOT EXISTS v_memories_python_compatible AS
SELECT
    id,
    namespace,
    key,
    value,
    memory_type,
    version,
    is_deleted,
    metadata,
    created_by,
    updated_by,
    created_at,
    updated_at
FROM memories;

-- View: v_sessions_python_compatible
-- Purpose: Provides a view for session data migration
CREATE VIEW IF NOT EXISTS v_sessions_python_compatible AS
SELECT
    id,
    app_name,
    user_id,
    project_id,
    state,
    created_at,
    updated_at
FROM sessions;

-- View: v_session_events_python_compatible
-- Purpose: Provides a view for session event data migration
CREATE VIEW IF NOT EXISTS v_session_events_python_compatible AS
SELECT
    id,
    session_id,
    event_id,
    event_type,
    actor,
    content,
    timestamp
FROM session_events;

-- =============================================================================
-- Migration Helper Functions (via triggers if needed)
-- =============================================================================

-- Trigger: validate_task_status_transition
-- Purpose: Ensure task status transitions are valid during migration
-- This helps catch data inconsistencies from the Python database
CREATE TRIGGER IF NOT EXISTS validate_task_status_transition
BEFORE UPDATE OF status ON tasks
FOR EACH ROW
WHEN NEW.status != OLD.status
BEGIN
    -- Log the status transition in audit table
    INSERT INTO audit (operation, actor, resource_id, timestamp, metadata, success)
    VALUES (
        'task_status_update',
        'system',
        NEW.id,
        datetime('now'),
        json_object(
            'old_status', OLD.status,
            'new_status', NEW.status
        ),
        1
    );

    -- Validate timestamp updates
    SELECT CASE
        WHEN NEW.status = 'running' AND NEW.started_at IS NULL THEN
            RAISE(ABORT, 'Task status changed to running but started_at is NULL')
        WHEN NEW.status IN ('completed', 'failed', 'cancelled') AND NEW.completed_at IS NULL THEN
            RAISE(ABORT, 'Task status changed to terminal state but completed_at is NULL')
    END;
END;

-- Trigger: update_task_last_updated
-- Purpose: Automatically update last_updated_at on any task modification
CREATE TRIGGER IF NOT EXISTS update_task_last_updated
AFTER UPDATE ON tasks
FOR EACH ROW
WHEN NEW.last_updated_at = OLD.last_updated_at
BEGIN
    UPDATE tasks
    SET last_updated_at = datetime('now')
    WHERE id = NEW.id;
END;

-- Trigger: update_session_updated_at
-- Purpose: Automatically update updated_at on session modifications
CREATE TRIGGER IF NOT EXISTS update_session_updated_at
AFTER UPDATE ON sessions
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE sessions
    SET updated_at = datetime('now')
    WHERE id = NEW.id;
END;

-- Trigger: update_memory_updated_at
-- Purpose: Automatically update updated_at on memory modifications
CREATE TRIGGER IF NOT EXISTS update_memory_updated_at
AFTER UPDATE ON memories
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE memories
    SET updated_at = datetime('now')
    WHERE id = NEW.id;
END;

-- =============================================================================
-- Migration Validation Views
-- =============================================================================

-- View: v_migration_validation_summary
-- Purpose: Provides summary statistics for validating Python → Rust migration
CREATE VIEW IF NOT EXISTS v_migration_validation_summary AS
SELECT
    'tasks' as table_name,
    COUNT(*) as total_rows,
    COUNT(CASE WHEN status = 'pending' THEN 1 END) as pending_count,
    COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed_count,
    COUNT(CASE WHEN status = 'failed' THEN 1 END) as failed_count,
    COUNT(CASE WHEN dependencies IS NOT NULL THEN 1 END) as tasks_with_dependencies,
    COUNT(CASE WHEN parent_task_id IS NOT NULL THEN 1 END) as tasks_with_parent
FROM tasks
UNION ALL
SELECT
    'agents' as table_name,
    COUNT(*) as total_rows,
    COUNT(CASE WHEN status = 'idle' THEN 1 END) as idle_count,
    COUNT(CASE WHEN status = 'busy' THEN 1 END) as busy_count,
    COUNT(CASE WHEN status = 'terminated' THEN 1 END) as terminated_count,
    0 as tasks_with_dependencies,
    0 as tasks_with_parent
FROM agents
UNION ALL
SELECT
    'memories' as table_name,
    COUNT(*) as total_rows,
    COUNT(CASE WHEN memory_type = 'semantic' THEN 1 END) as semantic_count,
    COUNT(CASE WHEN memory_type = 'episodic' THEN 1 END) as episodic_count,
    COUNT(CASE WHEN memory_type = 'procedural' THEN 1 END) as procedural_count,
    COUNT(CASE WHEN is_deleted = 1 THEN 1 END) as deleted_count,
    0 as tasks_with_parent
FROM memories
UNION ALL
SELECT
    'sessions' as table_name,
    COUNT(*) as total_rows,
    COUNT(DISTINCT user_id) as unique_users,
    COUNT(DISTINCT project_id) as unique_projects,
    0 as count3,
    0 as count4,
    0 as count5
FROM sessions
UNION ALL
SELECT
    'session_events' as table_name,
    COUNT(*) as total_rows,
    COUNT(DISTINCT session_id) as sessions_with_events,
    COUNT(DISTINCT event_type) as unique_event_types,
    0 as count3,
    0 as count4,
    0 as count5
FROM session_events;

-- View: v_orphaned_references
-- Purpose: Identify orphaned foreign key references (data integrity check)
CREATE VIEW IF NOT EXISTS v_orphaned_references AS
SELECT
    'tasks.parent_task_id' as reference_column,
    t.id as referencing_id,
    t.parent_task_id as referenced_id,
    'parent task does not exist' as issue
FROM tasks t
WHERE t.parent_task_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM tasks p WHERE p.id = t.parent_task_id)
UNION ALL
SELECT
    'tasks.session_id' as reference_column,
    t.id as referencing_id,
    t.session_id as referenced_id,
    'session does not exist' as issue
FROM tasks t
WHERE t.session_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = t.session_id)
UNION ALL
SELECT
    'agents.current_task_id' as reference_column,
    a.id as referencing_id,
    a.current_task_id as referenced_id,
    'task does not exist' as issue
FROM agents a
WHERE a.current_task_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.id = a.current_task_id)
UNION ALL
SELECT
    'session_events.session_id' as reference_column,
    CAST(se.id AS TEXT) as referencing_id,
    se.session_id as referenced_id,
    'session does not exist' as issue
FROM session_events se
WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id = se.session_id);

-- =============================================================================
-- Data Quality Checks
-- =============================================================================

-- View: v_data_quality_checks
-- Purpose: Validate data quality constraints that may have been missed in Python
CREATE VIEW IF NOT EXISTS v_data_quality_checks AS
SELECT
    'task_summary_too_long' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM tasks
WHERE length(summary) > 140
UNION ALL
SELECT
    'task_priority_out_of_range' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM tasks
WHERE priority < 0 OR priority > 10
UNION ALL
SELECT
    'task_invalid_status' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM tasks
WHERE status NOT IN ('pending', 'blocked', 'ready', 'running', 'completed', 'failed', 'cancelled')
UNION ALL
SELECT
    'task_running_without_started_at' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM tasks
WHERE status = 'running' AND started_at IS NULL
UNION ALL
SELECT
    'task_completed_without_completed_at' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM tasks
WHERE status IN ('completed', 'failed', 'cancelled') AND completed_at IS NULL
UNION ALL
SELECT
    'memory_invalid_type' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(CAST(id AS TEXT), ', ') as violating_ids
FROM memories
WHERE memory_type NOT IN ('semantic', 'episodic', 'procedural')
UNION ALL
SELECT
    'agent_invalid_status' as check_name,
    COUNT(*) as violation_count,
    GROUP_CONCAT(id, ', ') as violating_ids
FROM agents
WHERE status NOT IN ('idle', 'busy', 'terminated');

-- =============================================================================
-- Migration Complete Indicator
-- =============================================================================

-- Table: migration_metadata
-- Purpose: Track migration status and version information
CREATE TABLE IF NOT EXISTS migration_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Insert initial migration metadata
INSERT OR IGNORE INTO migration_metadata (key, value, updated_at)
VALUES
    ('schema_version', '1.0.0', datetime('now')),
    ('migration_001_applied', 'true', datetime('now')),
    ('migration_002_applied', 'true', datetime('now')),
    ('python_migration_status', 'pending', datetime('now'));
