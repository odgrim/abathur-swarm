-- Composite index for get_ready_tasks: SELECT * FROM tasks WHERE status = 'ready' ORDER BY priority DESC
CREATE INDEX IF NOT EXISTS idx_tasks_status_priority ON tasks(status, priority DESC);

-- Partial index for deadline-based queries
CREATE INDEX IF NOT EXISTS idx_tasks_deadline ON tasks(deadline) WHERE deadline IS NOT NULL;

-- Partial index for goal-related task lookups
CREATE INDEX IF NOT EXISTS idx_tasks_goal_id ON tasks(goal_id) WHERE goal_id IS NOT NULL;
