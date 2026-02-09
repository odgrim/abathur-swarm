-- Persistent command deduplication table.
-- Prevents replaying the same command after process restart.
-- Entries older than 24 hours are pruned on the event-pruning schedule.

CREATE TABLE IF NOT EXISTS processed_commands (
    command_id TEXT PRIMARY KEY,
    processed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_processed_commands_time ON processed_commands(processed_at);
