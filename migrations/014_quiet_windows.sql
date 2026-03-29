-- Quiet windows for cost-control scheduling.
-- Each row defines a recurring time window during which
-- the swarm should not dispatch new work.

CREATE TABLE IF NOT EXISTS quiet_windows (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    -- Cron expression for window start (5-field: min hour dom month dow)
    start_cron  TEXT NOT NULL,
    -- Cron expression for window end
    end_cron    TEXT NOT NULL,
    -- IANA timezone (e.g. "America/New_York")
    timezone    TEXT NOT NULL DEFAULT 'UTC',
    -- enabled / disabled
    status      TEXT NOT NULL DEFAULT 'enabled',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
