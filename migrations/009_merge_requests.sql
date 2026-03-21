-- Merge request persistence for the two-stage merge queue.
-- Ensures conflict records survive across ephemeral MergeQueue instances.

CREATE TABLE IF NOT EXISTS merge_requests (
    id TEXT PRIMARY KEY,
    stage TEXT NOT NULL,
    task_id TEXT NOT NULL,
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

CREATE INDEX IF NOT EXISTS idx_merge_requests_status ON merge_requests (status);
CREATE INDEX IF NOT EXISTS idx_merge_requests_task_id ON merge_requests (task_id);
