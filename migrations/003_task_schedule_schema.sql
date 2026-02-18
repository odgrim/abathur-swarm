-- Task schedule definitions for periodic task creation.
CREATE TABLE IF NOT EXISTS task_schedules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',

    -- Schedule configuration (JSON: {"type":"cron","expression":"..."})
    schedule_type TEXT NOT NULL,
    schedule_data TEXT NOT NULL,

    -- Task template
    task_title TEXT NOT NULL,
    task_description TEXT NOT NULL,
    task_priority TEXT NOT NULL DEFAULT 'normal',
    task_agent_type TEXT,

    -- Behavior
    overlap_policy TEXT NOT NULL DEFAULT 'skip',
    status TEXT NOT NULL DEFAULT 'active',

    -- Tracking
    scheduled_event_id TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0,
    last_fired_at TEXT,
    last_task_id TEXT,

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_task_schedules_status ON task_schedules(status);
CREATE INDEX IF NOT EXISTS idx_task_schedules_name ON task_schedules(name);
