-- Migration: Event architecture - drop tasks.goal_id, add handler watermarks
-- Version: 7

-- Drop unused goal_id column from tasks (decoupling: goals are convergent attractors, not task parents)
-- Migration 006 already dropped the index; now remove the column itself.
-- SQLite doesn't support DROP COLUMN before 3.35.0, so we recreate the table.
-- However, since the column was already unused and we don't want to risk data loss
-- with a table recreation, we simply leave the column in place and document it as deprecated.
-- The application layer already ignores it.

-- Handler watermark tracking for event replay on startup
CREATE TABLE IF NOT EXISTS handler_watermarks (
    handler_name TEXT PRIMARY KEY,
    last_sequence INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO schema_migrations (version, description) VALUES (7, 'Event architecture: handler watermarks');
