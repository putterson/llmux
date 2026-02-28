use crate::config::AlertConfig;
use crate::session::socket::SocketServer;
use regex::Regex;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tracing::{debug, error, warn};

pub struct InputWatcher {
    session_name: String,
    patterns: Vec<Regex>,
    config: AlertConfig,
    socket_server: SocketServer,
}

impl InputWatcher {
    pub fn new(
        session_name: String,
        pattern_strings: Vec<String>,
        config: AlertConfig,
        socket_server: SocketServer,
    ) -> Self {
        let patterns: Vec<Regex> = pattern_strings
            .iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(r) => Some(r),
                Err(e) => {
                    warn!(pattern = %p, error = %e, "invalid alert pattern");
                    None
                }
            })
            .collect();

        InputWatcher {
            session_name,
            patterns,
            config,
            socket_server,
        }
    }

    pub fn start(
        self,
        mut output_rx: broadcast::Receiver<Vec<u8>>,
        mut input_rx: broadcast::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let idle_timeout = Duration::from_secs(self.config.idle_timeout_secs);
            let mut last_output_time = Instant::now();
            let mut idle_alerted = false;
            // Use a byte buffer to avoid char boundary issues with raw PTY output
            let mut recent_output: Vec<u8> = Vec::new();
            // Track recent activity to avoid false idle alerts on startup
            let mut had_activity = false;
            // Track bytes received per second to distinguish real output from
            // cursor/spinner animations (~20-60 bytes/sec for blinking dots)
            let mut bytes_this_second: usize = 0;

            loop {
                tokio::select! {
                    // User input suppresses idle alerts until new output arrives
                    result = input_rx.recv() => {
                        match result {
                            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                                had_activity = false;
                                idle_alerted = false;
                                last_output_time = Instant::now();
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!("watcher: input channel closed");
                                break;
                            }
                        }
                    }
                    result = output_rx.recv() => {
                        match result {
                            Ok(data) => {
                                bytes_this_second += data.len();

                                // Append to recent output buffer (keep last 4KB for pattern matching)
                                recent_output.extend_from_slice(&data);
                                if recent_output.len() > 4096 {
                                    let excess = recent_output.len() - 4096;
                                    recent_output.drain(..excess);
                                }

                                // Convert to lossy string only for pattern matching
                                let text = String::from_utf8_lossy(&recent_output);

                                // Check for agent-specific patterns on the last few lines
                                let last_lines: String = text
                                    .lines()
                                    .rev()
                                    .take(5)
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect::<Vec<_>>()
                                    .join("\n");

                                for pattern in &self.patterns {
                                    if pattern.is_match(&last_lines) {
                                        let snippet = last_lines.lines().last().unwrap_or("").trim();
                                        let msg = format!("Pattern matched: {}", snippet);
                                        self.fire_alert(&msg).await;
                                        // Clear to avoid re-alerting on same output
                                        recent_output.clear();
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                debug!("watcher lagged by {} messages", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                debug!("watcher: output channel closed");
                                break;
                            }
                        }
                    }

                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        // Only reset idle timer for substantial output (>128 bytes/sec),
                        // ignoring low-bandwidth animations like blinking dots
                        if bytes_this_second > 128 {
                            had_activity = true;
                            last_output_time = Instant::now();
                            idle_alerted = false;
                        }
                        bytes_this_second = 0;

                        // Check for idle timeout
                        if had_activity && !idle_alerted && last_output_time.elapsed() >= idle_timeout {
                            self.fire_alert("Agent appears idle — may be waiting for input").await;
                            idle_alerted = true;
                        }
                    }
                }
            }
        })
    }

    async fn fire_alert(&self, message: &str) {
        debug!(session = %self.session_name, message = %message, "firing alert");

        // Terminal bell to attached clients
        if self.config.terminal_bell {
            self.socket_server.broadcast_alert(message).await;
        }

        // Desktop notification
        if self.config.desktop_notification {
            let title = format!("llmux: {}", self.session_name);
            let msg = message.to_string();
            // Run in blocking task to avoid blocking async runtime
            tokio::task::spawn_blocking(move || {
                if let Err(e) = notify_rust::Notification::new()
                    .summary(&title)
                    .body(&msg)
                    .timeout(notify_rust::Timeout::Milliseconds(5000))
                    .show()
                {
                    debug!("desktop notification failed: {}", e);
                }
            });
        }

        // Custom command
        if !self.config.custom_command.is_empty() {
            let cmd = self
                .config
                .custom_command
                .replace("{name}", &self.session_name)
                .replace("{message}", message);
            let cmd_clone = cmd.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_clone)
                    .status()
                {
                    error!(command = %cmd_clone, error = %e, "custom alert command failed");
                }
            });
        }
    }
}
