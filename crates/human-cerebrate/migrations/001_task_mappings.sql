CREATE TABLE IF NOT EXISTS task_mappings (
    federation_task_id TEXT PRIMARY KEY,
    correlation_id     TEXT NOT NULL,
    clickup_task_id    TEXT NOT NULL,
    title              TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending',
    priority           TEXT NOT NULL DEFAULT 'normal',
    parent_goal_id     TEXT,
    envelope_json      TEXT NOT NULL,
    clickup_status     TEXT NOT NULL DEFAULT '',
    human_response     TEXT,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at         TEXT NOT NULL DEFAULT (datetime('now')),
    deadline_at        TEXT NOT NULL,
    result_sent        INTEGER NOT NULL DEFAULT 0
);
