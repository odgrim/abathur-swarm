-- Migration: Add decomposition fields for fan-out/fan-in pattern
-- These fields enable chain steps to spawn multiple child tasks and wait for them

-- Add awaiting_children field (JSON array of child task UUIDs)
-- When set, task is in AwaitingChildren status waiting for these tasks to complete
ALTER TABLE tasks ADD COLUMN awaiting_children TEXT;

-- Add spawned_by_task_id field (UUID of parent task that spawned this via decomposition)
-- Links child task back to its spawner for fan-in tracking
ALTER TABLE tasks ADD COLUMN spawned_by_task_id TEXT;

-- Index for efficient lookup of tasks spawned by a parent
CREATE INDEX IF NOT EXISTS idx_tasks_spawned_by_task_id ON tasks(spawned_by_task_id);

-- Index for finding tasks that are awaiting children
CREATE INDEX IF NOT EXISTS idx_tasks_awaiting_children ON tasks(status) WHERE status = 'awaiting_children';
