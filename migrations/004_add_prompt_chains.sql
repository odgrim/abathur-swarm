-- Migration: Add Prompt Chains Tables
-- Description: Create tables for storing prompt chains and their executions
-- Date: 2025-11-03

-- Table for storing prompt chain definitions
CREATE TABLE IF NOT EXISTS prompt_chains (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    steps TEXT NOT NULL,  -- JSON array of PromptStep objects
    validation_rules TEXT NOT NULL DEFAULT '[]',  -- JSON array of ValidationRule objects
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Table for tracking chain executions
CREATE TABLE IF NOT EXISTS chain_executions (
    id TEXT PRIMARY KEY,
    chain_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    current_step INTEGER NOT NULL DEFAULT 0,
    step_results TEXT NOT NULL DEFAULT '[]',  -- JSON array of StepResult objects
    status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed', 'validation_failed')),
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    FOREIGN KEY (chain_id) REFERENCES prompt_chains(id) ON DELETE CASCADE,
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

-- Indexes for efficient querying
CREATE INDEX IF NOT EXISTS idx_prompt_chains_name ON prompt_chains(name);
CREATE INDEX IF NOT EXISTS idx_chain_executions_chain_id ON chain_executions(chain_id);
CREATE INDEX IF NOT EXISTS idx_chain_executions_task_id ON chain_executions(task_id);
CREATE INDEX IF NOT EXISTS idx_chain_executions_status ON chain_executions(status);
CREATE INDEX IF NOT EXISTS idx_chain_executions_started_at ON chain_executions(started_at);

-- Trigger to update the updated_at timestamp
CREATE TRIGGER IF NOT EXISTS update_prompt_chains_timestamp
AFTER UPDATE ON prompt_chains
BEGIN
    UPDATE prompt_chains
    SET updated_at = datetime('now')
    WHERE id = NEW.id;
END;

-- Comments (for documentation purposes - SQLite doesn't natively support column comments)
--
-- prompt_chains table:
--   - id: Unique identifier for the chain (UUID)
--   - name: Human-readable name for the chain
--   - description: Detailed description of the chain's purpose
--   - steps: JSON array containing PromptStep objects with prompt templates, roles, and output formats
--   - validation_rules: JSON array of ValidationRule objects for output validation
--   - created_at: ISO 8601 timestamp of chain creation
--   - updated_at: ISO 8601 timestamp of last modification
--
-- chain_executions table:
--   - id: Unique identifier for this execution (UUID)
--   - chain_id: Reference to the prompt chain being executed
--   - task_id: Reference to the associated task
--   - current_step: Index of the currently executing step (0-based)
--   - step_results: JSON array of StepResult objects with outputs and metadata
--   - status: Current execution status (running, completed, failed, validation_failed)
--   - started_at: ISO 8601 timestamp when execution began
--   - completed_at: ISO 8601 timestamp when execution finished (NULL if still running)
