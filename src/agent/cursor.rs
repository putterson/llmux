use super::{AgentDef, SessionIdStrategy};

pub fn agent_def() -> AgentDef {
    AgentDef {
        name: "cursor".to_string(),
        command: "cursor".to_string(),
        default_args: vec!["agent".to_string()],
        prompt_flag: None,
        resume_flag: Some("--resume".to_string()),
        continue_flag: None,
        session_id_flag: None,
        session_id_strategy: SessionIdStrategy::Manual,
        alert_patterns: vec![
            r"\[y/N\]".to_string(),
            r"\? $".to_string(),
            r"approve".to_string(),
            r"> $".to_string(),
        ],
    }
}
