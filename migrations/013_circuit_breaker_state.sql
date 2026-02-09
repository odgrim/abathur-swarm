-- Persist circuit breaker state across restarts.
-- Prevents a repeatedly-crashing handler from consuming resources after restart.

CREATE TABLE IF NOT EXISTS circuit_breaker_state (
    handler_name TEXT PRIMARY KEY,
    failure_count INTEGER NOT NULL DEFAULT 0,
    tripped INTEGER NOT NULL DEFAULT 0,
    tripped_at TEXT,
    last_failure TEXT
);
