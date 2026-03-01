use crate::error::{Error, Result};
use crate::session::chord::ChordAction;
use crate::session::socket::{
    encode_frame, FRAME_ALERT, FRAME_DETACH, FRAME_PTY_INPUT, FRAME_PTY_OUTPUT, FRAME_RESIZE,
    FRAME_SESSION_END, FRAME_SESSION_INFO,
};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::debug;

/// Attach to a session's Unix domain socket.
/// This puts the terminal into raw mode, relays I/O, and handles detach.
pub async fn attach(socket_path: &Path, session_name: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| {
            Error::Socket(format!(
                "failed to connect to {}: {}",
                socket_path.display(),
                e
            ))
        })?;

    eprintln!(
        "Attached to session '{}'. Press Ctrl+] then q to detach.",
        session_name
    );

    // Check for debug input logging
    let debug_input = std::env::var("LLMUX_DEBUG_INPUT").map_or(false, |v| v == "1");

    // Send initial resize
    send_resize(&mut stream).await?;

    // Read session info frame
    let frame_type = stream
        .read_u8()
        .await
        .map_err(|e| Error::Socket(e.to_string()))?;
    let length = stream
        .read_u32()
        .await
        .map_err(|e| Error::Socket(e.to_string()))?;
    let mut payload = vec![0u8; length as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| Error::Socket(e.to_string()))?;

    if frame_type == FRAME_SESSION_INFO {
        debug!(
            "received session info: {}",
            String::from_utf8_lossy(&payload)
        );
    }

    // Put terminal into raw mode
    let stdin_handle = io::stdin();
    let stdin_fd = stdin_handle.as_fd();
    let original_termios = nix::sys::termios::tcgetattr(stdin_fd)?;
    let mut raw = original_termios.clone();
    nix::sys::termios::cfmakeraw(&mut raw);
    nix::sys::termios::tcsetattr(stdin_fd, nix::sys::termios::SetArg::TCSANOW, &raw)?;

    let running = Arc::new(AtomicBool::new(true));
    let session_ended = Arc::new(AtomicBool::new(false));

    // Split stream
    let (read_half, write_half) = stream.into_split();
    let read_half = Arc::new(tokio::sync::Mutex::new(read_half));
    let write_half = Arc::new(tokio::sync::Mutex::new(write_half));

    // Set up SIGWINCH handler
    let write_half_resize = write_half.clone();
    let running_resize = running.clone();
    let _sigwinch_task = tokio::spawn(async move {
        let mut sig =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
                .expect("failed to register SIGWINCH handler");
        while running_resize.load(Ordering::Relaxed) {
            sig.recv().await;
            let mut wh = write_half_resize.lock().await;
            if let Ok(resize_data) = get_resize_payload() {
                let frame = encode_frame(FRAME_RESIZE, &resize_data);
                let _ = wh.write_all(&frame).await;
            }
        }
    });

    // Task: read from socket, write to stdout
    let running_reader = running.clone();
    let session_ended_reader = session_ended.clone();
    let read_half_clone = read_half.clone();
    let original_termios_clone = original_termios.clone();
    let reader_task = tokio::spawn(async move {
        let mut rh = read_half_clone.lock().await;
        let mut stdout = io::stdout();
        loop {
            if !running_reader.load(Ordering::Relaxed) {
                break;
            }
            let frame_type = match rh.read_u8().await {
                Ok(t) => t,
                Err(_) => break,
            };
            let length = match rh.read_u32().await {
                Ok(l) => l,
                Err(_) => break,
            };
            let mut payload = vec![0u8; length as usize];
            if rh.read_exact(&mut payload).await.is_err() {
                break;
            }

            match frame_type {
                FRAME_PTY_OUTPUT => {
                    let _ = stdout.write_all(&payload);
                    let _ = stdout.flush();
                }
                FRAME_SESSION_END => {
                    let info: serde_json::Value =
                        serde_json::from_slice(&payload).unwrap_or_default();
                    let exit_code = info.get("exit_code").and_then(|v| v.as_i64());
                    // Restore terminal before printing
                    restore_terminal(&original_termios_clone);
                    match exit_code {
                        Some(code) => eprintln!("\nSession ended (exit code: {})", code),
                        None => eprintln!("\nSession ended"),
                    }
                    session_ended_reader.store(true, Ordering::Relaxed);
                    running_reader.store(false, Ordering::Relaxed);
                    break;
                }
                FRAME_ALERT => {
                    // Send BEL to terminal
                    let _ = stdout.write_all(b"\x07");
                    let _ = stdout.flush();
                }
                _ => {}
            }
        }
    });

    // Task: read from stdin, write to socket (with detach escape detection)
    let running_writer = running.clone();
    let write_half_input = write_half.clone();
    let session_name_owned = session_name.to_string();
    let original_termios_diag = original_termios.clone();
    let writer_task = tokio::spawn(async move {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 1024];
        let mut chord = super::chord::ChordDetector::new();

        loop {
            if !running_writer.load(Ordering::Relaxed) {
                break;
            }

            // Read from stdin (blocking, in a blocking task)
            let data = tokio::task::spawn_blocking(move || {
                let n = stdin.read(&mut buf)?;
                Ok::<(io::Stdin, [u8; 1024], usize), io::Error>((stdin, buf, n))
            })
            .await;

            match data {
                Ok(Ok((stdin_ret, buf_ret, n))) => {
                    stdin = stdin_ret;
                    buf = buf_ret;

                    if n == 0 {
                        break;
                    }

                    if debug_input {
                        let hex: Vec<String> =
                            buf[..n].iter().map(|b| format!("{:02x}", b)).collect();
                        eprintln!("[LLMUX_DEBUG_INPUT] stdin {} bytes: {}", n, hex.join(" "));
                    }

                    let result = chord.process(&buf[..n]);

                    if !result.forward.is_empty() {
                        let frame = encode_frame(FRAME_PTY_INPUT, &result.forward);
                        let mut wh = write_half_input.lock().await;
                        let _ = wh.write_all(&frame).await;
                    }

                    match result.action {
                        ChordAction::Detach => {
                            let frame = encode_frame(FRAME_DETACH, &[]);
                            let mut wh = write_half_input.lock().await;
                            let _ = wh.write_all(&frame).await;
                            running_writer.store(false, Ordering::Relaxed);
                            break;
                        }
                        ChordAction::Diagnostic => {
                            // Temporarily restore terminal for diagnostic output
                            restore_terminal(&original_termios_diag);
                            print_diagnostic(
                                &session_name_owned,
                                &chord,
                                debug_input,
                            );
                            // Re-enter raw mode
                            let stdin_handle = io::stdin();
                            let stdin_fd = stdin_handle.as_fd();
                            let mut raw = original_termios_diag.clone();
                            nix::sys::termios::cfmakeraw(&mut raw);
                            let _ = nix::sys::termios::tcsetattr(
                                stdin_fd,
                                nix::sys::termios::SetArg::TCSANOW,
                                &raw,
                            );
                        }
                        ChordAction::None => {}
                    }
                }
                _ => break,
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = reader_task => {}
        _ = writer_task => {}
    }

    running.store(false, Ordering::Relaxed);

    // Restore terminal
    restore_terminal(&original_termios);

    if !session_ended.load(Ordering::Relaxed) {
        eprintln!("Detached from session '{}'.", session_name);
    }
    Ok(())
}

/// Print diagnostic information about the session and chord detector state.
fn print_diagnostic(
    session_name: &str,
    chord: &super::chord::ChordDetector,
    debug_input: bool,
) {
    let mut stderr = io::stderr();
    let _ = writeln!(stderr, "\n--- llmux diagnostic (Ctrl+] d) ---");
    let _ = writeln!(stderr, "Session:          {}", session_name);
    let _ = writeln!(stderr, "Binary version:   {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(stderr, "Chord state:      {}", chord.state_description());

    // Last input bytes hex dump
    let last = chord.last_input_bytes();
    if last.is_empty() {
        let _ = writeln!(stderr, "Last input bytes: (none)");
    } else {
        let hex: Vec<String> = last.iter().map(|b| format!("{:02x}", b)).collect();
        let _ = writeln!(stderr, "Last input bytes: {}", hex.join(" "));
    }

    // Terminal info
    let term = std::env::var("TERM").unwrap_or_else(|_| "(unset)".to_string());
    let _ = writeln!(stderr, "TERM:             {}", term);

    if let Ok(payload) = get_resize_payload() {
        if payload.len() >= 4 {
            let rows = u16::from_be_bytes([payload[0], payload[1]]);
            let cols = u16::from_be_bytes([payload[2], payload[3]]);
            let _ = writeln!(stderr, "Terminal size:    {}x{}", cols, rows);
        }
    }

    let _ = writeln!(
        stderr,
        "LLMUX_DEBUG_INPUT: {}",
        if debug_input { "enabled" } else { "disabled" }
    );
    let _ = writeln!(stderr, "---");
}

fn restore_terminal(termios: &nix::sys::termios::Termios) {
    let stdin = io::stdin();
    let _ = nix::sys::termios::tcsetattr(
        stdin.as_fd(),
        nix::sys::termios::SetArg::TCSANOW,
        termios,
    );
}

async fn send_resize(stream: &mut UnixStream) -> Result<()> {
    if let Ok(data) = get_resize_payload() {
        let frame = encode_frame(FRAME_RESIZE, &data);
        stream
            .write_all(&frame)
            .await
            .map_err(|e| Error::Socket(e.to_string()))?;
    }
    Ok(())
}

fn get_resize_payload() -> Result<Vec<u8>> {
    use nix::libc;
    let mut winsize: libc::winsize = unsafe { std::mem::zeroed() };
    let result =
        unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut winsize) };
    if result != 0 {
        return Err(Error::Pty("ioctl TIOCGWINSZ failed".to_string()));
    }
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&winsize.ws_row.to_be_bytes());
    data.extend_from_slice(&winsize.ws_col.to_be_bytes());
    Ok(data)
}
