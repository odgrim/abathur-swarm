-- Migration 006: Add idempotency_key to tasks table
-- Purpose: Prevent duplicate task creation when chain steps retry

-- Add idempotency_key column to tasks table
ALTER TABLE tasks ADD COLUMN idempotency_key TEXT;

-- Create index on idempotency_key for fast lookups
CREATE INDEX IF NOT EXISTS idx_tasks_idempotency_key
    ON tasks(idempotency_key)
    WHERE idempotency_key IS NOT NULL;
