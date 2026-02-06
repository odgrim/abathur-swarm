-- Migration: Goal-task rebuild - add aspirational goal fields and task source tracking
-- Version: 6

-- Add aspirational goal fields
ALTER TABLE goals ADD COLUMN applicability_domains TEXT NOT NULL DEFAULT '[]';
ALTER TABLE goals ADD COLUMN evaluation_criteria TEXT NOT NULL DEFAULT '[]';

-- Add task source tracking
ALTER TABLE tasks ADD COLUMN source_type TEXT NOT NULL DEFAULT 'human';
ALTER TABLE tasks ADD COLUMN source_ref TEXT;

-- Remove goal-task coupling index (column still exists for backwards compat, but is unused)
DROP INDEX IF EXISTS idx_tasks_goal;

-- Track migration
INSERT INTO schema_migrations (version, description) VALUES (6, 'Goal-task rebuild');
