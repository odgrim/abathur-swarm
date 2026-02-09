-- Scheduled events persistence table.
-- Allows one-shot and interval schedules to survive restarts.
CREATE TABLE IF NOT EXISTS scheduled_events (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    schedule_type TEXT NOT NULL,     -- "once", "interval", "cron"
    schedule_data TEXT NOT NULL,     -- JSON: {"at": "2024-..."} or {"every_secs": 30} or {"cron": "..."}
    payload TEXT NOT NULL,           -- JSON EventPayload
    category TEXT NOT NULL,
    severity TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    last_fired TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0
);

-- Add registration timestamp to handler watermarks.
-- New handlers only replay events from their registration time, not from sequence 0.
ALTER TABLE handler_watermarks ADD COLUMN registered_at TEXT;

INSERT INTO schema_migrations (version, description) VALUES (8, 'Event consistency: scheduled events persistence');
