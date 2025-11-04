-- Migration: Add chain_id to tasks table
-- Description: Add foreign key to link tasks to prompt chains
-- Date: 2025-11-03

-- Add chain_id column to tasks table
ALTER TABLE tasks ADD COLUMN chain_id TEXT;

-- Add foreign key constraint to prompt_chains table
-- Note: SQLite doesn't support ADD CONSTRAINT, so we can't add the FK after creation
-- The constraint will be enforced at the application level

-- Create index on chain_id for efficient queries
CREATE INDEX IF NOT EXISTS idx_tasks_chain_id ON tasks(chain_id);

-- Comments (for documentation purposes - SQLite doesn't natively support column comments)
--
-- chain_id: Optional reference to a prompt chain (UUID)
--   - If NULL, task executes with standard single-prompt flow
--   - If set, task executes through the multi-step prompt chain
--   - Links to prompt_chains(id) table
