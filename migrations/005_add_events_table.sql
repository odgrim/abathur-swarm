-- Migration: Add events table for event persistence
-- Version: 5

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    timestamp TEXT NOT NULL,
    severity TEXT NOT NULL,
    category TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT,
    correlation_id TEXT,
    payload TEXT NOT NULL
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_events_sequence ON events(sequence);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_category ON events(category);
CREATE INDEX IF NOT EXISTS idx_events_goal_id ON events(goal_id) WHERE goal_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_correlation_id ON events(correlation_id) WHERE correlation_id IS NOT NULL;

-- Track migration
INSERT INTO schema_migrations (version, description) VALUES (5, 'Add events table');
