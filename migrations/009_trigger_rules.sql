-- Trigger rules table for declarative event-driven automation.
CREATE TABLE IF NOT EXISTS trigger_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    filter_json TEXT NOT NULL,
    condition_type TEXT NOT NULL,
    condition_data TEXT,
    action_type TEXT NOT NULL,
    action_data TEXT,
    cooldown_secs INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_fired TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_trigger_rules_enabled ON trigger_rules(enabled);
CREATE INDEX IF NOT EXISTS idx_trigger_rules_name ON trigger_rules(name);

INSERT INTO schema_migrations (version, description) VALUES (9, 'Trigger rules for declarative automation');
