use crate::error::{Error, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error};

/// Handle for interacting with the PTY
#[derive(Clone)]
pub struct PtyHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master_pty: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    output_tx: broadcast::Sender<Vec<u8>>,
}

impl PtyHandle {
    /// Write input to the PTY (from attached client)
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        self.writer
            .lock()
            .map_err(|e| Error::Pty(format!("writer lock poisoned: {}", e)))?
            .write_all(data)
            .map_err(|e| Error::Pty(format!("write failed: {}", e)))?;
        Ok(())
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master_pty
            .lock()
            .map_err(|e| Error::Pty(format!("pty lock poisoned: {}", e)))?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("resize failed: {}", e)))?;
        Ok(())
    }

    /// Subscribe to PTY output
    pub fn subscribe_output(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }
}

/// Spawn an agent in a PTY. Returns the PtyHandle and a channel that receives
/// the exit code when the agent exits.
pub fn spawn_agent(
    command: &str,
    args: &[String],
    work_dir: &Path,
) -> Result<(PtyHandle, mpsc::Receiver<Option<i32>>)> {
    let pty_system = native_pty_system();

    // Get terminal size from current terminal, or use defaults
    let size = get_terminal_size().unwrap_or(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    });

    let pair = pty_system
        .openpty(size)
        .map_err(|e| Error::Pty(format!("openpty failed: {}", e)))?;

    let mut cmd = CommandBuilder::new(command);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.cwd(work_dir);

    // Remove environment variables that prevent agent nesting.
    // CommandBuilder inherits the parent env; we must explicitly remove unwanted vars.
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");
    // Set TERM
    cmd.env(
        "TERM",
        std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()),
    );

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| Error::Pty(format!("spawn failed: {}", e)))?;

    // Drop slave — we only need the master side
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| Error::Pty(format!("take_writer failed: {}", e)))?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| Error::Pty(format!("clone_reader failed: {}", e)))?;

    let (output_tx, _) = broadcast::channel::<Vec<u8>>(256);
    let (exit_tx, exit_rx) = mpsc::channel::<Option<i32>>(1);

    let handle = PtyHandle {
        writer: Arc::new(Mutex::new(writer)),
        master_pty: Arc::new(Mutex::new(pair.master)),
        output_tx: output_tx.clone(),
    };

    // Dedicated OS thread for reading PTY output (portable-pty is sync)
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    debug!("PTY reader: EOF");
                    break;
                }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    // Broadcast to all subscribers; ignore error if no receivers
                    let _ = output_tx.send(data);
                }
                Err(e) => {
                    debug!(error = %e, "PTY reader error");
                    break;
                }
            }
        }
    });

    // Dedicated OS thread for waiting on child exit
    std::thread::spawn(move || {
        let status = child.wait();
        let exit_code = match status {
            Ok(status) => {
                if status.success() {
                    Some(0)
                } else {
                    // portable-pty ExitStatus doesn't directly expose code on all platforms,
                    // but we can check success/failure
                    Some(1)
                }
            }
            Err(e) => {
                error!(error = %e, "failed to wait for child");
                None
            }
        };
        let _ = exit_tx.blocking_send(exit_code);
    });

    Ok((handle, exit_rx))
}

fn get_terminal_size() -> Option<PtySize> {
    use nix::libc;
    let mut winsize: libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut winsize) };
    if result == 0 && winsize.ws_row > 0 && winsize.ws_col > 0 {
        Some(PtySize {
            rows: winsize.ws_row,
            cols: winsize.ws_col,
            pixel_width: winsize.ws_xpixel,
            pixel_height: winsize.ws_ypixel,
        })
    } else {
        None
    }
}
