-- Migration 007: Refinement requests persistence for evolution loop
--
-- Stores RefinementRequest records so InProgress refinements can be
-- recovered after a process restart via startup reconciliation.

CREATE TABLE IF NOT EXISTS refinement_requests (
    id TEXT PRIMARY KEY NOT NULL,
    template_name TEXT NOT NULL,
    template_version INTEGER NOT NULL,
    severity TEXT NOT NULL,
    trigger TEXT NOT NULL,
    stats_json TEXT NOT NULL,
    failed_task_ids_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'Pending',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_refinement_requests_status
    ON refinement_requests(status);

CREATE INDEX IF NOT EXISTS idx_refinement_requests_template_name
    ON refinement_requests(template_name);
