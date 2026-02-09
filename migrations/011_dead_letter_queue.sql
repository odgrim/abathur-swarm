-- Dead letter queue for handler failures.
-- When a handler fails to process an event (circuit breaker trips, errors, timeouts),
-- the event+handler pair is recorded here for later retry.

CREATE TABLE IF NOT EXISTS dead_letter_events (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    event_sequence INTEGER NOT NULL,
    handler_name TEXT NOT NULL,
    error_message TEXT NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3,
    next_retry_at TEXT,
    created_at TEXT NOT NULL,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_dle_pending ON dead_letter_events(resolved_at, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_dle_handler ON dead_letter_events(handler_name, resolved_at);
