-- Migration 013: Template stats persistence for evolution loop
-- Persists TemplateStats, TaskExecution records, and version change history
-- so the evolution loop survives process restarts.

CREATE TABLE IF NOT EXISTS template_stats (
    template_name TEXT PRIMARY KEY,
    template_version INTEGER NOT NULL,
    total_tasks INTEGER NOT NULL DEFAULT 0,
    successful_tasks INTEGER NOT NULL DEFAULT 0,
    failed_tasks INTEGER NOT NULL DEFAULT 0,
    goal_violations INTEGER NOT NULL DEFAULT 0,
    success_rate REAL NOT NULL DEFAULT 0.0,
    avg_turns REAL NOT NULL DEFAULT 0.0,
    avg_tokens REAL NOT NULL DEFAULT 0.0,
    first_execution TEXT,
    last_execution TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS template_executions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    template_name TEXT NOT NULL,
    template_version INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    executed_at TEXT NOT NULL,
    turns_used INTEGER NOT NULL,
    tokens_used INTEGER NOT NULL,
    downstream_tasks_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_template_executions_template
    ON template_executions(template_name, template_version);

CREATE TABLE IF NOT EXISTS template_version_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_name TEXT NOT NULL,
    from_version INTEGER NOT NULL,
    to_version INTEGER NOT NULL,
    previous_stats_json TEXT NOT NULL,
    changed_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_template_version_changes_template
    ON template_version_changes(template_name);
