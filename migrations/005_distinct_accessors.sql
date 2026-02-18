-- Track distinct accessors for memory promotion integrity.
-- Stored as a JSON array of accessor objects.
-- Default '[]' ensures existing memories start with empty accessor sets.
ALTER TABLE memories ADD COLUMN distinct_accessors TEXT NOT NULL DEFAULT '[]';
