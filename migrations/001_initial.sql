CREATE TABLE IF NOT EXISTS sessions (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    agent_type       TEXT NOT NULL,
    agent_session_id TEXT,
    pid              INTEGER,
    socket_path      TEXT,
    work_dir         TEXT NOT NULL,
    source_dirs      TEXT,
    is_workspace     INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'running',
    exit_code        INTEGER,
    started_at       TEXT NOT NULL,
    stopped_at       TEXT,
    initial_prompt   TEXT,
    extra            TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at);
