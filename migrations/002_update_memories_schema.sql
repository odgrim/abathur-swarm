-- Update memories schema for three-tier system

-- Add tier column
ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'working';

-- Add expires_at column
ALTER TABLE memories ADD COLUMN expires_at TEXT;

-- Add metadata column (JSON)
ALTER TABLE memories ADD COLUMN metadata TEXT;

-- Rename 'value' to 'content' by creating new column
ALTER TABLE memories ADD COLUMN content TEXT;
UPDATE memories SET content = value WHERE content IS NULL;

-- Add indexes for new columns
CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(tier);
CREATE INDEX IF NOT EXISTS idx_memories_expires ON memories(expires_at);
CREATE INDEX IF NOT EXISTS idx_memories_key_ns ON memories(key, namespace);

INSERT OR IGNORE INTO schema_migrations (version, description) VALUES (2, 'Update memories schema for three-tier system');
