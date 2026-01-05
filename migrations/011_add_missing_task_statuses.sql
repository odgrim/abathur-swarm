-- Migration: Add missing task statuses to CHECK constraint
--
-- The TaskStatus enum includes statuses that were not in the original CHECK constraint:
-- - awaiting_children: Used in fan-out decomposition pattern
-- - awaiting_validation: Task complete, validation spawned
-- - validation_running: Validation task is executing
-- - validation_failed: Validation found issues
--
-- SQLite doesn't support ALTER TABLE to modify CHECK constraints,
-- so we must recreate the table with the updated constraint.

-- Step 1: Create new table with updated constraint
CREATE TABLE IF NOT EXISTS tasks_new (
    id TEXT PRIMARY KEY NOT NULL,
    summary TEXT NOT NULL,
    description TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    priority INTEGER NOT NULL CHECK(priority >= 0 AND priority <= 10),
    calculated_priority REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL CHECK(status IN (
        'pending',
        'blocked',
        'ready',
        'running',
        'awaiting_children',
        'awaiting_validation',
        'validation_running',
        'validation_failed',
        'completed',
        'failed',
        'cancelled'
    )),
    dependencies TEXT,
    dependency_type TEXT NOT NULL DEFAULT 'sequential' CHECK(dependency_type IN ('sequential', 'parallel')),
    dependency_depth INTEGER NOT NULL DEFAULT 0,
    input_data TEXT,
    result_data TEXT,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    max_execution_timeout_seconds INTEGER NOT NULL DEFAULT 3600,
    submitted_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    last_updated_at TEXT NOT NULL,
    created_by TEXT,
    parent_task_id TEXT,
    session_id TEXT,
    source TEXT NOT NULL DEFAULT 'direct',
    deadline TEXT,
    estimated_duration_seconds INTEGER,
    feature_branch TEXT,
    branch TEXT,
    worktree_path TEXT,
    validation_requirement TEXT NOT NULL DEFAULT 'none',
    validation_task_id TEXT,
    validating_task_id TEXT,
    remediation_count INTEGER NOT NULL DEFAULT 0,
    is_remediation INTEGER NOT NULL DEFAULT 0,
    workflow_state TEXT,
    workflow_expectations TEXT,
    chain_id TEXT,
    chain_step_index INTEGER NOT NULL DEFAULT 0,
    chain_handoff_state TEXT,
    awaiting_children TEXT,
    spawned_by_task_id TEXT,
    idempotency_key TEXT UNIQUE,
    version INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- Step 2: Copy all data from old table
INSERT INTO tasks_new SELECT * FROM tasks;

-- Step 3: Drop old table
DROP TABLE tasks;

-- Step 4: Rename new table
ALTER TABLE tasks_new RENAME TO tasks;

-- Step 5: Recreate all indexes
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority);
CREATE INDEX IF NOT EXISTS idx_tasks_agent_type ON tasks(agent_type);
CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at ON tasks(submitted_at);
CREATE INDEX IF NOT EXISTS idx_tasks_parent_task_id ON tasks(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks(session_id);
CREATE INDEX IF NOT EXISTS idx_tasks_chain_id ON tasks(chain_id);
CREATE INDEX IF NOT EXISTS idx_tasks_spawned_by_task_id ON tasks(spawned_by_task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_awaiting_children ON tasks(status) WHERE status = 'awaiting_children';
