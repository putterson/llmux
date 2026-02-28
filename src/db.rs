use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SCHEMA: &str = r#"
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
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    pub agent_session_id: Option<String>,
    pub pid: Option<i64>,
    pub socket_path: Option<String>,
    pub work_dir: String,
    pub source_dirs: Option<Vec<String>>,
    pub is_workspace: bool,
    pub status: SessionStatus,
    pub exit_code: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub initial_prompt: Option<String>,
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Stopped,
    Crashed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Stopped => write!(f, "stopped"),
            SessionStatus::Crashed => write!(f, "crashed"),
        }
    }
}

impl SessionStatus {
    fn from_str(s: &str) -> Self {
        match s {
            "running" => SessionStatus::Running,
            "stopped" => SessionStatus::Stopped,
            "crashed" => SessionStatus::Crashed,
            _ => SessionStatus::Crashed,
        }
    }
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open() -> Result<Self> {
        let db_path = db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Database { conn })
    }

    pub fn insert_session(&self, session: &Session) -> Result<()> {
        let source_dirs_json = session
            .source_dirs
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let extra_json = session
            .extra
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        self.conn.execute(
            "INSERT INTO sessions (id, name, agent_type, agent_session_id, pid, socket_path, work_dir, source_dirs, is_workspace, status, exit_code, started_at, stopped_at, initial_prompt, extra)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                session.id,
                session.name,
                session.agent_type,
                session.agent_session_id,
                session.pid,
                session.socket_path,
                session.work_dir,
                source_dirs_json,
                session.is_workspace as i32,
                session.status.to_string(),
                session.exit_code,
                session.started_at.to_rfc3339(),
                session.stopped_at.map(|t| t.to_rfc3339()),
                session.initial_prompt,
                extra_json,
            ],
        )?;
        Ok(())
    }

    pub fn update_session_status(
        &self,
        id: &str,
        status: SessionStatus,
        exit_code: Option<i32>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET status = ?1, exit_code = ?2, stopped_at = ?3 WHERE id = ?4",
            params![status.to_string(), exit_code, now, id],
        )?;
        Ok(())
    }

    pub fn update_session_pid(&self, id: &str, pid: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET pid = ?1 WHERE id = ?2",
            params![pid, id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn update_session_socket(&self, id: &str, socket_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET socket_path = ?1 WHERE id = ?2",
            params![socket_path, id],
        )?;
        Ok(())
    }

    pub fn update_session_agent_session_id(
        &self,
        id: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET agent_session_id = ?1 WHERE id = ?2",
            params![agent_session_id, id],
        )?;
        Ok(())
    }

    pub fn get_session_by_id(&self, id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row("SELECT * FROM sessions WHERE id = ?1", params![id], |row| {
                row_to_session(row)
            })
            .optional()
            .map_err(Error::from)
    }

    pub fn get_session_by_name(&self, name: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT * FROM sessions WHERE name = ?1",
                params![name],
                row_to_session,
            )
            .optional()
            .map_err(Error::from)
    }

    /// Find session by prefix match on name or ID
    pub fn find_session(&self, query: &str) -> Result<Session> {
        // Try exact name match first
        if let Some(session) = self.get_session_by_name(query)? {
            return Ok(session);
        }
        // Try exact ID match
        if let Some(session) = self.get_session_by_id(query)? {
            return Ok(session);
        }
        // Try prefix match on name
        let mut stmt = self.conn.prepare(
            "SELECT * FROM sessions WHERE name LIKE ?1 ORDER BY started_at DESC",
        )?;
        let pattern = format!("{}%", query);
        let sessions: Vec<Session> = stmt
            .query_map(params![pattern], row_to_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        match sessions.len() {
            0 => {
                // Try prefix match on ID
                let mut stmt = self.conn.prepare(
                    "SELECT * FROM sessions WHERE id LIKE ?1 ORDER BY started_at DESC",
                )?;
                let sessions: Vec<Session> = stmt
                    .query_map(params![pattern], row_to_session)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                match sessions.len() {
                    0 => Err(Error::SessionNotFound(query.to_string())),
                    1 => Ok(sessions.into_iter().next().unwrap()),
                    _ => Err(Error::AmbiguousSession(query.to_string())),
                }
            }
            1 => Ok(sessions.into_iter().next().unwrap()),
            _ => Err(Error::AmbiguousSession(query.to_string())),
        }
    }

    pub fn list_sessions(&self, include_all: bool) -> Result<Vec<Session>> {
        let sql = if include_all {
            "SELECT * FROM sessions ORDER BY started_at DESC"
        } else {
            "SELECT * FROM sessions WHERE status = 'running' ORDER BY started_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let sessions = stmt
            .query_map([], row_to_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    pub fn list_running_sessions_in_dir(&self, work_dir: &str) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM sessions WHERE status = 'running' AND work_dir = ?1 ORDER BY started_at DESC",
        )?;
        let sessions = stmt
            .query_map(params![work_dir], row_to_session)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    pub fn history(&self, limit: usize, agent_type: Option<&str>) -> Result<Vec<Session>> {
        let sql = match agent_type {
            Some(_) => {
                "SELECT * FROM sessions WHERE agent_type = ?1 ORDER BY started_at DESC LIMIT ?2"
            }
            None => "SELECT * FROM sessions ORDER BY started_at DESC LIMIT ?1",
        };
        let mut stmt = self.conn.prepare(sql)?;
        let sessions = match agent_type {
            Some(agent) => stmt
                .query_map(params![agent, limit as i64], row_to_session)?
                .collect::<std::result::Result<Vec<_>, _>>()?,
            None => stmt
                .query_map(params![limit as i64], row_to_session)?
                .collect::<std::result::Result<Vec<_>, _>>()?,
        };
        Ok(sessions)
    }

    /// Mark any sessions whose PIDs are no longer alive as crashed
    pub fn reap_dead_sessions(&self) -> Result<usize> {
        let running = self.list_sessions(false)?;
        let mut reaped = 0;
        for session in running {
            if let Some(pid) = session.pid {
                if !is_process_alive(pid as u32) {
                    self.update_session_status(&session.id, SessionStatus::Crashed, None)?;
                    // Clean up socket file
                    if let Some(ref sock) = session.socket_path {
                        let _ = std::fs::remove_file(sock);
                    }
                    reaped += 1;
                }
            }
        }
        Ok(reaped)
    }

    /// Get the single running session, if exactly one exists
    pub fn get_sole_running_session(&self) -> Result<Option<Session>> {
        let running = self.list_sessions(false)?;
        if running.len() == 1 {
            Ok(Some(running.into_iter().next().unwrap()))
        } else {
            Ok(None)
        }
    }

    /// Check if a name is already in use
    pub fn name_exists(&self, name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Delete sessions (for clean command)
    pub fn delete_stopped_sessions(&self) -> Result<usize> {
        let count = self.conn.execute(
            "DELETE FROM sessions WHERE status IN ('stopped', 'crashed')",
            [],
        )?;
        Ok(count)
    }
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    let source_dirs_json: Option<String> = row.get("source_dirs")?;
    let source_dirs: Option<Vec<String>> = source_dirs_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let extra_json: Option<String> = row.get("extra")?;
    let extra: Option<serde_json::Value> =
        extra_json.as_deref().and_then(|s| serde_json::from_str(s).ok());

    let status_str: String = row.get("status")?;
    let started_at_str: String = row.get("started_at")?;
    let stopped_at_str: Option<String> = row.get("stopped_at")?;

    Ok(Session {
        id: row.get("id")?,
        name: row.get("name")?,
        agent_type: row.get("agent_type")?,
        agent_session_id: row.get("agent_session_id")?,
        pid: row.get("pid")?,
        socket_path: row.get("socket_path")?,
        work_dir: row.get("work_dir")?,
        source_dirs,
        is_workspace: row.get::<_, i32>("is_workspace")? != 0,
        status: SessionStatus::from_str(&status_str),
        exit_code: row.get("exit_code")?,
        started_at: DateTime::parse_from_rfc3339(&started_at_str)
            .unwrap_or_default()
            .with_timezone(&Utc),
        stopped_at: stopped_at_str.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        initial_prompt: row.get("initial_prompt")?,
        extra,
    })
}

fn is_process_alive(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

fn db_path() -> Result<PathBuf> {
    let data_dir = dirs::data_dir().ok_or_else(|| Error::Config("cannot determine data directory".to_string()))?;
    Ok(data_dir.join("llmux").join("llmux.db"))
}
