-- Migration 011: Add FK constraints on task_id columns
-- Adds REFERENCES tasks(id) with appropriate ON DELETE behavior to all
-- child tables that reference tasks but lack foreign key constraints.
--
-- SQLite requires table recreation to add FK constraints to existing columns.
-- Uses the 12-step ALTER TABLE process from SQLite documentation.

-- Must be outside transaction for SQLite
PRAGMA foreign_keys=OFF;

BEGIN;

-- ============================================================
-- Step 1: Clean up orphaned rows before adding constraints
-- ============================================================

-- Orphaned tasks (parent_id references non-existent task)
UPDATE tasks SET parent_id = NULL
    WHERE parent_id IS NOT NULL
    AND parent_id NOT IN (SELECT id FROM tasks);

-- Orphaned convergence_trajectories
DELETE FROM convergence_trajectories
    WHERE task_id NOT IN (SELECT id FROM tasks);

-- Orphaned worktrees
DELETE FROM worktrees
    WHERE task_id NOT IN (SELECT id FROM tasks);

-- Orphaned merge_requests
DELETE FROM merge_requests
    WHERE task_id NOT IN (SELECT id FROM tasks);

-- Orphaned agent_instances (SET NULL for nullable column)
UPDATE agent_instances SET current_task_id = NULL
    WHERE current_task_id IS NOT NULL
    AND current_task_id NOT IN (SELECT id FROM tasks);

-- Orphaned events
UPDATE events SET task_id = NULL
    WHERE task_id IS NOT NULL
    AND task_id NOT IN (SELECT id FROM tasks);

-- Orphaned scheduled_events
UPDATE scheduled_events SET task_id = NULL
    WHERE task_id IS NOT NULL
    AND task_id NOT IN (SELECT id FROM tasks);

-- Orphaned task_schedules
UPDATE task_schedules SET last_task_id = NULL
    WHERE last_task_id IS NOT NULL
    AND last_task_id NOT IN (SELECT id FROM tasks);

-- Orphaned trigger_absence_timers
UPDATE trigger_absence_timers SET scope_task_id = NULL
    WHERE scope_task_id IS NOT NULL
    AND scope_task_id NOT IN (SELECT id FROM tasks);

-- ============================================================
-- Step 2: Recreate tasks table (add ON DELETE SET NULL to parent_id)
-- ============================================================

CREATE TABLE tasks_new (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
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
    completed_at TEXT,
    task_type TEXT NOT NULL DEFAULT 'standard'
);

INSERT INTO tasks_new SELECT * FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

-- Recreate tasks indexes
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent_id);
CREATE INDEX idx_tasks_task_type ON tasks(task_type);

-- ============================================================
-- Step 3: Recreate convergence_trajectories (CASCADE)
-- ============================================================

CREATE TABLE convergence_trajectories_new (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
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

INSERT INTO convergence_trajectories_new SELECT * FROM convergence_trajectories;
DROP TABLE convergence_trajectories;
ALTER TABLE convergence_trajectories_new RENAME TO convergence_trajectories;

-- Recreate convergence_trajectories indexes
CREATE INDEX idx_convergence_trajectories_task_id ON convergence_trajectories(task_id);
CREATE INDEX idx_convergence_trajectories_goal_id ON convergence_trajectories(goal_id);
CREATE INDEX idx_convergence_trajectories_updated_at ON convergence_trajectories(updated_at);
CREATE INDEX idx_convergence_trajectories_phase ON convergence_trajectories(phase);

-- ============================================================
-- Step 4: Recreate worktrees (CASCADE)
-- ============================================================

CREATE TABLE worktrees_new (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
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

INSERT INTO worktrees_new SELECT * FROM worktrees;
DROP TABLE worktrees;
ALTER TABLE worktrees_new RENAME TO worktrees;

-- Recreate worktrees indexes
CREATE INDEX idx_worktrees_task ON worktrees(task_id);
CREATE INDEX idx_worktrees_status ON worktrees(status);

-- ============================================================
-- Step 5: Recreate merge_requests (CASCADE)
-- ============================================================

CREATE TABLE merge_requests_new (
    id TEXT PRIMARY KEY,
    stage TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    source_branch TEXT NOT NULL,
    target_branch TEXT NOT NULL,
    workdir TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'Queued',
    error TEXT,
    commit_sha TEXT,
    verification_json TEXT,
    conflict_files_json TEXT NOT NULL DEFAULT '[]',
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO merge_requests_new SELECT * FROM merge_requests;
DROP TABLE merge_requests;
ALTER TABLE merge_requests_new RENAME TO merge_requests;

-- Recreate merge_requests indexes
CREATE INDEX idx_merge_requests_status ON merge_requests(status);
CREATE INDEX idx_merge_requests_task_id ON merge_requests(task_id);

-- ============================================================
-- Step 6: Recreate agent_instances (SET NULL)
-- ============================================================

CREATE TABLE agent_instances_new (
    id TEXT PRIMARY KEY,
    template_id TEXT NOT NULL,
    template_name TEXT NOT NULL,
    current_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    turn_count INTEGER DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'idle',
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

INSERT INTO agent_instances_new SELECT * FROM agent_instances;
DROP TABLE agent_instances;
ALTER TABLE agent_instances_new RENAME TO agent_instances;

-- Recreate agent_instances indexes
CREATE INDEX idx_agent_instances_template ON agent_instances(template_name);
CREATE INDEX idx_agent_instances_status ON agent_instances(status);
CREATE INDEX idx_agent_instances_task ON agent_instances(current_task_id);

-- ============================================================
-- Step 7: Recreate events (SET NULL)
-- ============================================================

CREATE TABLE events_new (
    id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    timestamp TEXT NOT NULL,
    severity TEXT NOT NULL,
    category TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    correlation_id TEXT,
    payload TEXT NOT NULL,
    source_process_id TEXT
);

INSERT INTO events_new SELECT * FROM events;
DROP TABLE events;
ALTER TABLE events_new RENAME TO events;

-- Recreate events indexes
CREATE INDEX idx_events_sequence ON events(sequence);
CREATE INDEX idx_events_timestamp ON events(timestamp);
CREATE INDEX idx_events_category ON events(category);
CREATE INDEX idx_events_goal_id ON events(goal_id) WHERE goal_id IS NOT NULL;
CREATE INDEX idx_events_task_id ON events(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX idx_events_correlation_id ON events(correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX idx_events_source_process ON events(source_process_id);

-- ============================================================
-- Step 8: Recreate scheduled_events (SET NULL)
-- ============================================================

CREATE TABLE scheduled_events_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    schedule_type TEXT NOT NULL,
    schedule_data TEXT NOT NULL,
    payload TEXT NOT NULL,
    category TEXT NOT NULL,
    severity TEXT NOT NULL,
    goal_id TEXT,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    last_fired TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0
);

INSERT INTO scheduled_events_new SELECT * FROM scheduled_events;
DROP TABLE scheduled_events;
ALTER TABLE scheduled_events_new RENAME TO scheduled_events;

-- ============================================================
-- Step 9: Recreate task_schedules (SET NULL on last_task_id)
-- ============================================================

CREATE TABLE task_schedules_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    schedule_type TEXT NOT NULL,
    schedule_data TEXT NOT NULL,
    task_title TEXT NOT NULL,
    task_description TEXT NOT NULL,
    task_priority TEXT NOT NULL DEFAULT 'normal',
    task_agent_type TEXT,
    overlap_policy TEXT NOT NULL DEFAULT 'skip',
    status TEXT NOT NULL DEFAULT 'active',
    scheduled_event_id TEXT,
    fire_count INTEGER NOT NULL DEFAULT 0,
    last_fired_at TEXT,
    last_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO task_schedules_new SELECT * FROM task_schedules;
DROP TABLE task_schedules;
ALTER TABLE task_schedules_new RENAME TO task_schedules;

-- Recreate task_schedules indexes
CREATE INDEX idx_task_schedules_status ON task_schedules(status);
CREATE INDEX idx_task_schedules_name ON task_schedules(name);

-- ============================================================
-- Step 10: Recreate trigger_absence_timers (SET NULL on scope_task_id)
-- ============================================================

CREATE TABLE trigger_absence_timers_new (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES trigger_rules(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    deadline_secs INTEGER NOT NULL,
    expected_payload_type TEXT NOT NULL,
    scope_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    scope_correlation_id TEXT
);

INSERT INTO trigger_absence_timers_new SELECT * FROM trigger_absence_timers;
DROP TABLE trigger_absence_timers;
ALTER TABLE trigger_absence_timers_new RENAME TO trigger_absence_timers;

COMMIT;

-- Must be outside transaction for SQLite
PRAGMA foreign_keys=ON;
