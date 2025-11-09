-- Add chain_step_index field to tasks table for parallel chain execution
-- This enables each chain step to be a separate task in the queue

ALTER TABLE tasks ADD COLUMN chain_step_index INTEGER NOT NULL DEFAULT 0;

-- Create index for efficient lookups of chain tasks by step
CREATE INDEX idx_tasks_chain_step ON tasks(chain_id, chain_step_index) WHERE chain_id IS NOT NULL;
