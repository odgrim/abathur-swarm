-- ================================================================
-- Initial Schema Migration
-- Purpose: Create agents table with indexes and constraints
-- Author: rust-sqlx-database-specialist
-- Date: 2025-10-25
-- ================================================================
-- NOTE: PRAGMA statements for WAL mode, foreign keys, etc. are set
--       at connection time in DatabaseConnection::new()
-- ================================================================

-- ================================================================
-- TABLE: tasks (stub for foreign key reference)
-- ================================================================
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL
);

-- ================================================================
-- TABLE: agents
-- PURPOSE: Agent lifecycle tracking with heartbeat monitoring
-- ================================================================
CREATE TABLE IF NOT EXISTS agents (
    -- Primary identifiers
    id TEXT PRIMARY KEY NOT NULL,           -- UUID as text
    agent_type TEXT NOT NULL,               -- Type of agent

    -- Agent state
    status TEXT NOT NULL                    -- idle|busy|terminated
        CHECK(status IN ('idle', 'busy', 'terminated')),

    -- Current task assignment
    current_task_id TEXT,                   -- Currently executing task ID

    -- Health monitoring
    heartbeat_at TEXT NOT NULL,             -- Last heartbeat timestamp (ISO 8601)

    -- Resource tracking
    memory_usage_bytes INTEGER NOT NULL DEFAULT 0,      -- Current memory usage in bytes
    cpu_usage_percent REAL NOT NULL DEFAULT 0.0,        -- Current CPU usage percentage

    -- Lifecycle timestamps
    created_at TEXT NOT NULL,               -- Creation timestamp (ISO 8601)
    terminated_at TEXT,                     -- Termination timestamp (ISO 8601)

    -- Foreign key constraints
    FOREIGN KEY (current_task_id) REFERENCES tasks(id) ON DELETE SET NULL
);

-- ================================================================
-- INDEXES
-- ================================================================

-- Index for filtering by status
CREATE INDEX IF NOT EXISTS idx_agents_status
    ON agents(status);

-- Index for finding stale agents by heartbeat
CREATE INDEX IF NOT EXISTS idx_agents_heartbeat
    ON agents(heartbeat_at);

-- Composite index for status + heartbeat queries
CREATE INDEX IF NOT EXISTS idx_agents_status_heartbeat
    ON agents(status, heartbeat_at);

-- ================================================================
-- EXECUTION NOTES
-- ================================================================
-- 1. Run this migration using: sqlx migrate run
-- 2. Verify tables exist: SELECT name FROM sqlite_master WHERE type='table';
-- 3. Check foreign key integrity: PRAGMA foreign_key_check;
-- ================================================================
