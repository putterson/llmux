use super::{AgentDef, SessionIdStrategy};

pub fn agent_def() -> AgentDef {
    AgentDef {
        name: "claude".to_string(),
        command: "claude".to_string(),
        default_args: vec![],
        prompt_flag: Some("-p".to_string()),
        resume_flag: Some("--resume".to_string()),
        continue_flag: Some("--continue".to_string()),
        session_id_flag: Some("--session-id".to_string()),
        session_id_strategy: SessionIdStrategy::Flag,
        alert_patterns: vec![
            r"Allow .* tool".to_string(),
            r"\? $".to_string(),
            r"\[Y/n\]".to_string(),
            r"\[y/N\]".to_string(),
            r"approve".to_string(),
            r"Do you want to proceed".to_string(),
        ],
    }
}
