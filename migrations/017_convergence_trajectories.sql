-- Convergence trajectory persistence (spec Part 10.2)

CREATE TABLE IF NOT EXISTS convergence_trajectories (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    goal_id TEXT,
    phase TEXT NOT NULL DEFAULT 'preparing',
    total_fresh_starts INTEGER NOT NULL DEFAULT 0,
    -- Serialized JSON for complex nested types
    specification_json TEXT NOT NULL DEFAULT '{}',
    observations_json TEXT NOT NULL DEFAULT '[]',
    attractor_state_json TEXT NOT NULL DEFAULT '{}',
    budget_json TEXT NOT NULL DEFAULT '{}',
    policy_json TEXT NOT NULL DEFAULT '{}',
    strategy_log_json TEXT NOT NULL DEFAULT '[]',
    context_health_json TEXT NOT NULL DEFAULT '{}',
    hints_json TEXT NOT NULL DEFAULT '[]',
    forced_strategy_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_task_id
    ON convergence_trajectories(task_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_goal_id
    ON convergence_trajectories(goal_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_updated_at
    ON convergence_trajectories(updated_at);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_phase
    ON convergence_trajectories(phase);

INSERT INTO schema_migrations (version, description) VALUES (17, 'Convergence trajectory persistence');
