-- Workflow orchestration schema
-- Supports workflow definitions, instances, and phase instances for the phase orchestrator.

-- Workflow definitions (immutable blueprints)
CREATE TABLE IF NOT EXISTS workflow_definitions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    goal_id TEXT NOT NULL REFERENCES goals(id),
    definition_json TEXT NOT NULL,  -- serialized WorkflowDefinition (phases, edges, config)
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_workflow_definitions_goal_id ON workflow_definitions(goal_id);

-- Workflow instances (mutable runtime state)
CREATE TABLE IF NOT EXISTS workflow_instances (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflow_definitions(id),
    goal_id TEXT NOT NULL REFERENCES goals(id),
    status TEXT NOT NULL DEFAULT 'pending',
    tokens_consumed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_workflow_instances_workflow_id ON workflow_instances(workflow_id);
CREATE INDEX IF NOT EXISTS idx_workflow_instances_goal_id ON workflow_instances(goal_id);
CREATE INDEX IF NOT EXISTS idx_workflow_instances_status ON workflow_instances(status);

-- Phase instances (mutable runtime state per phase)
CREATE TABLE IF NOT EXISTS phase_instances (
    id TEXT PRIMARY KEY,  -- composite: workflow_instance_id + phase_id
    workflow_instance_id TEXT NOT NULL REFERENCES workflow_instances(id),
    phase_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    task_ids_json TEXT NOT NULL DEFAULT '[]',  -- serialized Vec<Uuid>
    retry_count INTEGER NOT NULL DEFAULT 0,
    verification_result INTEGER,  -- NULL = not verified, 0 = failed, 1 = passed
    iteration_count INTEGER NOT NULL DEFAULT 0,
    started_at TEXT,
    completed_at TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_phase_instances_workflow ON phase_instances(workflow_instance_id);
CREATE INDEX IF NOT EXISTS idx_phase_instances_status ON phase_instances(status);
