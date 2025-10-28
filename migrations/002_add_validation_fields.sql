-- Migration: Add validation and workflow tracking fields
-- Date: 2025-10-27
-- Description: Adds fields to support explicit validation pattern in task state machine

-- Add validation requirement column (stores ValidationRequirement enum as JSON)
ALTER TABLE tasks ADD COLUMN validation_requirement TEXT DEFAULT '{"type":"none"}';

-- Add validation task linkage
ALTER TABLE tasks ADD COLUMN validation_task_id TEXT;
ALTER TABLE tasks ADD COLUMN validating_task_id TEXT;

-- Add remediation tracking
ALTER TABLE tasks ADD COLUMN remediation_count INTEGER DEFAULT 0;
ALTER TABLE tasks ADD COLUMN is_remediation INTEGER DEFAULT 0;

-- Add workflow state tracking (stores WorkflowState as JSON)
ALTER TABLE tasks ADD COLUMN workflow_state TEXT;

-- Add workflow expectations (stores WorkflowExpectations as JSON)
ALTER TABLE tasks ADD COLUMN workflow_expectations TEXT;

-- Create index on validation_task_id for efficient lookups
CREATE INDEX IF NOT EXISTS idx_tasks_validation_task_id ON tasks(validation_task_id);

-- Create index on validating_task_id for efficient lookups
CREATE INDEX IF NOT EXISTS idx_tasks_validating_task_id ON tasks(validating_task_id);

-- Create index on is_remediation for filtering remediation tasks
CREATE INDEX IF NOT EXISTS idx_tasks_is_remediation ON tasks(is_remediation);
