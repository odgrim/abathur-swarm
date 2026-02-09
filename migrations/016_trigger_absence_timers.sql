-- Persist absence trigger timers so they survive restarts.
--
-- When a trigger rule with an Absence condition starts a timer, a row is
-- inserted here. If the expected event arrives the row is deleted. If the
-- timer fires (deadline expires) the row is also deleted. On startup, any
-- remaining rows are loaded back into the in-memory absence_timers map.

CREATE TABLE IF NOT EXISTS trigger_absence_timers (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES trigger_rules(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    deadline_secs INTEGER NOT NULL,
    expected_payload_type TEXT NOT NULL,
    scope_task_id TEXT,
    scope_correlation_id TEXT
);

INSERT INTO schema_migrations (version, description) VALUES (16, 'Trigger absence timers');
