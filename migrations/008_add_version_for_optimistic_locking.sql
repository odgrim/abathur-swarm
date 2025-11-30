-- Migration: Add version column for optimistic locking
-- This enables detection of concurrent modifications and prevents lost updates.

-- Add version column with default value of 1 for existing rows
ALTER TABLE tasks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- Create index for potential future queries by version
CREATE INDEX IF NOT EXISTS idx_tasks_version ON tasks(version);
