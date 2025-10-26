-- Create tasks table with all required fields and constraints
-- Note: WAL mode and pragmas are configured at connection level, not in migrations
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
    started_at TEXT,
    completed_at TEXT,
    last_updated_at TEXT NOT NULL,
    created_by TEXT,
    parent_task_id TEXT,
    session_id TEXT,
    source TEXT NOT NULL CHECK(source IN ('human', 'agent_requirements', 'agent_planner', 'agent_implementation')),
    deadline TEXT,
    estimated_duration_seconds INTEGER,
    feature_branch TEXT,
    task_branch TEXT,
    worktree_path TEXT
);

-- Create indexes for performance optimization

-- Primary query index: priority and status for task selection
CREATE INDEX IF NOT EXISTS idx_tasks_priority_status
    ON tasks(calculated_priority DESC, status);

-- Status-only index for filtering by status
CREATE INDEX IF NOT EXISTS idx_tasks_status
    ON tasks(status);

-- Submission time index for chronological queries
CREATE INDEX IF NOT EXISTS idx_tasks_submitted_at
    ON tasks(submitted_at);

-- Partial index for deadline-based queries (only index non-null deadlines)
CREATE INDEX IF NOT EXISTS idx_tasks_deadline
    ON tasks(deadline)
    WHERE deadline IS NOT NULL;

-- Partial index for parent task queries
CREATE INDEX IF NOT EXISTS idx_tasks_parent_task_id
    ON tasks(parent_task_id)
    WHERE parent_task_id IS NOT NULL;

-- Partial index for session-based queries
CREATE INDEX IF NOT EXISTS idx_tasks_session_id
    ON tasks(session_id)
    WHERE session_id IS NOT NULL;

-- Partial index for feature branch queries
CREATE INDEX IF NOT EXISTS idx_tasks_feature_branch
    ON tasks(feature_branch)
    WHERE feature_branch IS NOT NULL;

-- Composite index for agent type and status
CREATE INDEX IF NOT EXISTS idx_tasks_agent_type_status
    ON tasks(agent_type, status);

-- Index for source filtering
CREATE INDEX IF NOT EXISTS idx_tasks_source
    ON tasks(source);
