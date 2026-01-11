-- Migration 004: Fix worktrees foreign key constraint
-- Remove FK constraint to allow worktree creation without pre-existing tasks
-- Referential integrity maintained through application logic

-- Drop old table and recreate without FK
DROP TABLE IF EXISTS worktrees;

CREATE TABLE IF NOT EXISTS worktrees (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
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

CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status);

INSERT OR IGNORE INTO schema_migrations (version, description) VALUES (4, 'Fix worktrees FK constraint');
