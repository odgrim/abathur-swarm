-- Abathur Swarm Schema

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now')),
    description TEXT
);

CREATE TABLE IF NOT EXISTS goals (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    priority TEXT NOT NULL DEFAULT 'normal',
    parent_id TEXT REFERENCES goals(id),
    constraints TEXT,
    metadata TEXT,
    applicability_domains TEXT NOT NULL DEFAULT '[]',
    evaluation_criteria TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES tasks(id),
    goal_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    priority TEXT NOT NULL DEFAULT 'normal',
    agent_type TEXT,
    routing TEXT,
    artifacts TEXT,
    context TEXT,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    worktree_path TEXT,
    idempotency_key TEXT UNIQUE,
    version INTEGER DEFAULT 1,
    source_type TEXT NOT NULL DEFAULT 'human',
    source_ref TEXT,
    deadline TEXT,
    execution_mode TEXT NOT NULL DEFAULT '{"mode":"direct"}',
    trajectory_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    started_at TEXT,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on_id),
    CHECK (task_id != depends_on_id)
);

CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    memory_type TEXT NOT NULL,
    confidence REAL DEFAULT 1.0,
    access_count INTEGER DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'active',
    decay_rate REAL DEFAULT 0.1,
    version INTEGER DEFAULT 1,
    parent_id TEXT REFERENCES memories(id),
    tier TEXT NOT NULL DEFAULT 'working',
    expires_at TEXT,
    metadata TEXT,
    content TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_accessed_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(namespace, key, version)
);

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    memory_id,
    key,
    value,
    namespace
);

CREATE TABLE IF NOT EXISTS agent_templates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    tier TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    system_prompt TEXT NOT NULL,
    tools TEXT,
    constraints TEXT,
    handoff_targets TEXT,
    max_turns INTEGER DEFAULT 25,
    read_only INTEGER DEFAULT 0,
    is_active INTEGER DEFAULT 1,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(name, version)
);

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

CREATE TABLE IF NOT EXISTS worktrees (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    base_ref TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'creating',
    merge_commit TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    actor TEXT,
    old_value TEXT,
    new_value TEXT,
    rationale TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    timestamp TEXT NOT NULL,
    severity TEXT NOT NULL,
    category TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT,
    correlation_id TEXT,
    payload TEXT NOT NULL,
    source_process_id TEXT
);

CREATE TABLE IF NOT EXISTS handler_watermarks (
    handler_name TEXT PRIMARY KEY,
    last_sequence INTEGER NOT NULL DEFAULT 0,
    registered_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS scheduled_events (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    schedule_type TEXT NOT NULL,
    schedule_data TEXT NOT NULL,
    payload TEXT NOT NULL,
    category TEXT NOT NULL,
    severity TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    last_fired TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS trigger_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    filter_json TEXT NOT NULL,
    condition_type TEXT NOT NULL,
    condition_data TEXT,
    action_type TEXT NOT NULL,
    action_data TEXT,
    cooldown_secs INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_fired TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

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

CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    secret TEXT,
    filter_json TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1,
    max_failures INTEGER NOT NULL DEFAULT 10,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_delivered_sequence INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS circuit_breaker_state (
    handler_name TEXT PRIMARY KEY,
    failure_count INTEGER NOT NULL DEFAULT 0,
    tripped INTEGER NOT NULL DEFAULT 0,
    tripped_at TEXT,
    last_failure TEXT
);

CREATE TABLE IF NOT EXISTS processed_commands (
    command_id TEXT PRIMARY KEY,
    processed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS trigger_absence_timers (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES trigger_rules(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    deadline_secs INTEGER NOT NULL,
    expected_payload_type TEXT NOT NULL,
    scope_task_id TEXT,
    scope_correlation_id TEXT
);

CREATE TABLE IF NOT EXISTS convergence_trajectories (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    goal_id TEXT,
    phase TEXT NOT NULL DEFAULT 'preparing',
    total_fresh_starts INTEGER NOT NULL DEFAULT 0,
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

-- Indexes
CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);
CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
CREATE INDEX IF NOT EXISTS idx_memories_state ON memories(state);
CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(tier);
CREATE INDEX IF NOT EXISTS idx_memories_expires ON memories(expires_at);
CREATE INDEX IF NOT EXISTS idx_memories_key_ns ON memories(key, namespace);
CREATE INDEX IF NOT EXISTS idx_agent_templates_tier ON agent_templates(tier);
CREATE INDEX IF NOT EXISTS idx_agent_instances_template ON agent_instances(template_name);
CREATE INDEX IF NOT EXISTS idx_agent_instances_status ON agent_instances(status);
CREATE INDEX IF NOT EXISTS idx_agent_instances_task ON agent_instances(current_task_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status);
CREATE INDEX IF NOT EXISTS idx_events_sequence ON events(sequence);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_category ON events(category);
CREATE INDEX IF NOT EXISTS idx_events_goal_id ON events(goal_id) WHERE goal_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_correlation_id ON events(correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_source_process ON events(source_process_id);
CREATE INDEX IF NOT EXISTS idx_trigger_rules_enabled ON trigger_rules(enabled);
CREATE INDEX IF NOT EXISTS idx_trigger_rules_name ON trigger_rules(name);
CREATE INDEX IF NOT EXISTS idx_dle_pending ON dead_letter_events(resolved_at, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_dle_handler ON dead_letter_events(handler_name, resolved_at);
CREATE INDEX IF NOT EXISTS idx_processed_commands_time ON processed_commands(processed_at);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_task_id ON convergence_trajectories(task_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_goal_id ON convergence_trajectories(goal_id);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_updated_at ON convergence_trajectories(updated_at);
CREATE INDEX IF NOT EXISTS idx_convergence_trajectories_phase ON convergence_trajectories(phase);

INSERT OR IGNORE INTO schema_migrations (version, description) VALUES (1, 'Initial schema');
