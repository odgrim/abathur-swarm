-- Add source_process_id to events table for cross-process event propagation.
-- Events published by a specific EventBus instance are stamped with its process_id,
-- allowing the EventStorePoller to filter out events it already broadcast.

ALTER TABLE events ADD COLUMN source_process_id TEXT;

CREATE INDEX IF NOT EXISTS idx_events_source_process ON events(source_process_id);
