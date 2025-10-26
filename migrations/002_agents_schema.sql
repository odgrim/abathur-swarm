-- Create agents table
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY NOT NULL,
    agent_type TEXT NOT NULL,
    status TEXT NOT NULL,
    current_task_id TEXT,
    heartbeat_at TEXT NOT NULL,
    memory_usage_bytes INTEGER NOT NULL DEFAULT 0,
    cpu_usage_percent REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL,
    terminated_at TEXT
);

-- Create indexes for common queries
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);
CREATE INDEX IF NOT EXISTS idx_agents_agent_type ON agents(agent_type);
CREATE INDEX IF NOT EXISTS idx_agents_heartbeat ON agents(heartbeat_at);
CREATE INDEX IF NOT EXISTS idx_agents_current_task ON agents(current_task_id) WHERE current_task_id IS NOT NULL;
