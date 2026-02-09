-- Webhook subscriptions for external event delivery
CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    secret TEXT,
    filter_json TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1,
    max_failures INTEGER NOT NULL DEFAULT 10,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_delivered_sequence INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
