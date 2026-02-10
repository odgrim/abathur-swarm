-- Add execution_mode and trajectory_id columns to tasks table.
-- execution_mode stores a JSON-tagged enum; defaults to direct mode.
-- trajectory_id links convergent tasks to the convergence engine state.

ALTER TABLE tasks ADD COLUMN execution_mode TEXT NOT NULL DEFAULT '{"mode":"direct"}';
ALTER TABLE tasks ADD COLUMN trajectory_id TEXT;
