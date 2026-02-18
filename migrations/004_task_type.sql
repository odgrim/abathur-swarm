-- Add task_type column to tasks table.
-- Default 'standard' ensures backward compatibility with existing rows.
ALTER TABLE tasks ADD COLUMN task_type TEXT NOT NULL DEFAULT 'standard';

-- Index for filtering by task_type (e.g., `abathur task list --type verification`).
CREATE INDEX IF NOT EXISTS idx_tasks_task_type ON tasks(task_type);
