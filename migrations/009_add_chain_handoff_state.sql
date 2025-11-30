-- Add chain_handoff_state column for tracking pending chain step handoffs
-- This column stores JSON representing ChainHandoffState struct
-- When non-null on a completed task, indicates a pending handoff that needs recovery

ALTER TABLE tasks ADD COLUMN chain_handoff_state TEXT;

-- Index for efficiently finding tasks with pending handoffs
CREATE INDEX IF NOT EXISTS idx_tasks_chain_handoff_pending
ON tasks (status, chain_handoff_state)
WHERE chain_handoff_state IS NOT NULL;
