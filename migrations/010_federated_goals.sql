-- Federated goals table for tracking goals delegated to child swarms.
CREATE TABLE IF NOT EXISTS federated_goals (
    id TEXT PRIMARY KEY NOT NULL,
    local_goal_id TEXT NOT NULL,
    cerebrate_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_federated_goals_local_goal_id ON federated_goals(local_goal_id);
CREATE INDEX IF NOT EXISTS idx_federated_goals_cerebrate_id ON federated_goals(cerebrate_id);
CREATE INDEX IF NOT EXISTS idx_federated_goals_state ON federated_goals(state);
