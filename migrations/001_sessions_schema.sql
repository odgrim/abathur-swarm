-- Create sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    app_name TEXT NOT NULL,
    user_id TEXT NOT NULL,
    project_id TEXT,
    state TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Create indexes for sessions
CREATE INDEX IF NOT EXISTS idx_sessions_user_id
    ON sessions(user_id);

CREATE INDEX IF NOT EXISTS idx_sessions_project_id
    ON sessions(project_id)
    WHERE project_id IS NOT NULL;

-- Create session_events table
CREATE TABLE IF NOT EXISTS session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- Create indexes for session_events
CREATE INDEX IF NOT EXISTS idx_session_events_session_timestamp
    ON session_events(session_id, timestamp);
