-- Add agent instances table

CREATE TABLE IF NOT EXISTS agent_instances (
    id TEXT PRIMARY KEY,
    template_id TEXT NOT NULL,
    template_name TEXT NOT NULL,
    current_task_id TEXT,
    turn_count INTEGER DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'idle',
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_instances_template ON agent_instances(template_name);
CREATE INDEX IF NOT EXISTS idx_agent_instances_status ON agent_instances(status);
CREATE INDEX IF NOT EXISTS idx_agent_instances_task ON agent_instances(current_task_id);

-- Add description column to agent_templates if it doesn't exist
ALTER TABLE agent_templates ADD COLUMN description TEXT;

INSERT OR IGNORE INTO schema_migrations (version, description) VALUES (3, 'Add agent instances table');
