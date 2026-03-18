-- Event outbox table for transactional outbox pattern.
-- Events are inserted here within the same transaction as domain mutations,
-- then a background poller reads and publishes them to the EventBus.
CREATE TABLE IF NOT EXISTS event_outbox (
    id TEXT PRIMARY KEY,
    event_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    published_at TEXT
);

-- Index for efficient polling of unpublished events.
CREATE INDEX IF NOT EXISTS idx_event_outbox_unpublished
    ON event_outbox (created_at)
    WHERE published_at IS NULL;
