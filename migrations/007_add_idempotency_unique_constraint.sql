-- Migration 007: Add UNIQUE constraint on idempotency_key
-- Purpose: Enforce atomic idempotency at database level to prevent race conditions
-- when multiple concurrent executions try to spawn the same tasks

-- SQLite doesn't support adding constraints to existing tables directly,
-- so we need to recreate the table. However, since idempotency_key can be NULL
-- for tasks created before this feature, we use a UNIQUE index instead which
-- allows multiple NULLs but enforces uniqueness for non-NULL values.

-- Drop the existing non-unique index
DROP INDEX IF EXISTS idx_tasks_idempotency_key;

-- Create a UNIQUE index on idempotency_key
-- This enforces uniqueness at the database level while allowing NULLs
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_idempotency_key_unique
    ON tasks(idempotency_key)
    WHERE idempotency_key IS NOT NULL;
