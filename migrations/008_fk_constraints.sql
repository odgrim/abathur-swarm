-- Migration 008: Add foreign key constraints to convergence_trajectories and worktrees
--
-- Fixes #61: convergence_trajectories and worktrees have no FK reference to tasks(id),
-- causing orphaned rows when tasks are deleted.
--
-- SQLite does not support ALTER TABLE to add FK constraints, so we must
-- recreate the tables. Data is preserved via INSERT...SELECT.

-- ============================================================================
-- 1. convergence_trajectories: add REFERENCES tasks(id) ON DELETE CASCADE
-- ============================================================================

CREATE TABLE IF NOT EXISTS convergence_trajectories_new (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    goal_id TEXT,
    phase TEXT NOT NULL DEFAULT 'preparing',
    total_fresh_starts INTEGER NOT NULL DEFAULT 0,
    specification_json TEXT NOT NULL DEFAULT '{}',
    observations_json TEXT NOT NULL DEFAULT '[]',
    attractor_state_json TEXT NOT NULL DEFAULT '{}',
    budget_json TEXT NOT NULL DEFAULT '{}',
    policy_json TEXT NOT NULL DEFAULT '{}',
    strategy_log_json TEXT NOT NULL DEFAULT '[]',
    context_health_json TEXT NOT NULL DEFAULT '{}',
    hints_json TEXT NOT NULL DEFAULT '[]',
    forced_strategy_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Copy existing data (skip orphans whose task_id no longer exists)
INSERT OR IGNORE INTO convergence_trajectories_new
SELECT ct.*
FROM convergence_trajectories ct
INNER JOIN tasks t ON ct.task_id = t.id;

-- Swap tables
DROP TABLE IF EXISTS convergence_trajectories;
ALTER TABLE convergence_trajectories_new RENAME TO convergence_trajectories;

-- Recreate indexes
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_task_id ON convergence_trajectories(task_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_goal_id ON convergence_trajectories(goal_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_updated_at ON convergence_trajectories(updated_at);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_phase ON convergence_trajectories(phase);

-- ============================================================================
-- 2. worktrees: add REFERENCES tasks(id) ON DELETE SET NULL
-- ============================================================================
-- Using SET NULL instead of CASCADE because worktree cleanup should be
-- handled explicitly (files on disk need removal). Setting task_id to NULL
-- signals "orphaned worktree" without losing the worktree record.

CREATE TABLE IF NOT EXISTS worktrees_new (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    base_ref TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'creating',
    merge_commit TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

INSERT OR IGNORE INTO worktrees_new
SELECT * FROM worktrees;

DROP TABLE IF EXISTS worktrees;
ALTER TABLE worktrees_new RENAME TO worktrees;

CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status);

-- ============================================================================
-- 3. Clean up any existing orphaned agent_instances referencing deleted tasks
-- ============================================================================
-- agent_instances.current_task_id is ephemeral runtime state, so we just
-- NULL it out for rows referencing nonexistent tasks.

UPDATE agent_instances
SET current_task_id = NULL
WHERE current_task_id IS NOT NULL
  AND current_task_id NOT IN (SELECT id FROM tasks);

-- Record migration
INSERT OR IGNORE INTO schema_migrations (version, description)
VALUES (8, 'Add FK constraints to convergence_trajectories and worktrees, clean orphaned agent_instances');
