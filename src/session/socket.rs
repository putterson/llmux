use crate::error::{Error, Result};
use crate::session::pty::PtyHandle;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, Mutex, Notify};
use tracing::{debug, error, info, warn};

// Frame types
pub const FRAME_PTY_OUTPUT: u8 = 0x01;
pub const FRAME_PTY_INPUT: u8 = 0x02;
pub const FRAME_RESIZE: u8 = 0x03;
pub const FRAME_DETACH: u8 = 0x04;
pub const FRAME_SESSION_INFO: u8 = 0x05;
pub const FRAME_SESSION_END: u8 = 0x06;
pub const FRAME_ALERT: u8 = 0x07;

/// Encode a frame: [1B type][4B length BE][payload]
pub fn encode_frame(frame_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(frame_type);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}


type ClientId = u64;

struct ClientHandle {
    tx: mpsc::Sender<Vec<u8>>,
}

#[derive(Clone)]
pub struct SocketServer {
    inner: Arc<SocketServerInner>,
}

struct SocketServerInner {
    session_id: String,
    pty_handle: PtyHandle,
    listener_path: std::path::PathBuf,
    clients: Mutex<HashMap<ClientId, ClientHandle>>,
    next_client_id: Mutex<u64>,
    replay_buffer: Mutex<ReplayBuffer>,
    shutdown_notify: Notify,
    alert_tx: broadcast::Sender<Vec<u8>>,
    input_tx: broadcast::Sender<()>,
}

struct ReplayBuffer {
    data: Vec<u8>,
    max_size: usize,
}

impl ReplayBuffer {
    fn new(max_size: usize) -> Self {
        ReplayBuffer {
            data: Vec::with_capacity(max_size),
            max_size,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
        if self.data.len() > self.max_size {
            let excess = self.data.len() - self.max_size;
            self.data.drain(..excess);
        }
    }

    fn contents(&self) -> &[u8] {
        &self.data
    }
}

impl SocketServer {
    pub async fn new(
        socket_path: &Path,
        session_id: String,
        pty_handle: PtyHandle,
        replay_buffer_size: usize,
    ) -> Result<Self> {
        // Clean up stale socket
        let _ = std::fs::remove_file(socket_path);

        let (alert_tx, _) = broadcast::channel(32);
        let (input_tx, _) = broadcast::channel(32);

        let server = SocketServer {
            inner: Arc::new(SocketServerInner {
                session_id,
                pty_handle,
                listener_path: socket_path.to_path_buf(),
                clients: Mutex::new(HashMap::new()),
                next_client_id: Mutex::new(1),
                replay_buffer: Mutex::new(ReplayBuffer::new(replay_buffer_size)),
                shutdown_notify: Notify::new(),
                alert_tx,
                input_tx,
            }),
        };

        Ok(server)
    }

    /// Run the socket server. This spawns tasks for:
    /// 1. Accepting new client connections
    /// 2. Broadcasting PTY output to all clients and the replay buffer
    pub async fn run(&self) -> Result<()> {
        let listener =
            UnixListener::bind(&self.inner.listener_path).map_err(|e| Error::Socket(e.to_string()))?;

        info!(path = %self.inner.listener_path.display(), "socket server listening");

        // Task: read PTY output and broadcast to clients + replay buffer
        let server = self.clone();
        let pty_broadcast_handle = tokio::spawn(async move {
            let mut rx = server.inner.pty_handle.subscribe_output();
            loop {
                match rx.recv().await {
                    Ok(data) => {
                        // Append to replay buffer
                        {
                            let mut buf = server.inner.replay_buffer.lock().await;
                            buf.append(&data);
                        }
                        // Broadcast to all connected clients
                        let frame = encode_frame(FRAME_PTY_OUTPUT, &data);
                        server.broadcast_frame(&frame).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("PTY broadcast lagged by {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("PTY broadcast channel closed");
                        break;
                    }
                }
            }
        });

        // Accept loop
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let server = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = server.handle_client(stream).await {
                                    debug!(error = %e, "client handler error");
                                }
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "accept error");
                        }
                    }
                }
                _ = self.inner.shutdown_notify.notified() => {
                    info!("socket server shutting down");
                    break;
                }
            }
        }

        pty_broadcast_handle.abort();
        Ok(())
    }

    async fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
        let client_id = {
            let mut id = self.inner.next_client_id.lock().await;
            let cid = *id;
            *id += 1;
            cid
        };

        info!(client_id = client_id, "client connected");

        // Send session info
        let info = serde_json::json!({
            "session_id": self.inner.session_id,
        });
        let info_frame = encode_frame(FRAME_SESSION_INFO, info.to_string().as_bytes());
        stream
            .write_all(&info_frame)
            .await
            .map_err(|e| Error::Socket(e.to_string()))?;

        // Send replay buffer
        {
            let buf = self.inner.replay_buffer.lock().await;
            let contents = buf.contents();
            if !contents.is_empty() {
                let replay_frame = encode_frame(FRAME_PTY_OUTPUT, contents);
                stream
                    .write_all(&replay_frame)
                    .await
                    .map_err(|e| Error::Socket(e.to_string()))?;
            }
        }

        // Register client for broadcasts
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);
        {
            let mut clients = self.inner.clients.lock().await;
            clients.insert(client_id, ClientHandle { tx });
        }

        // Split the stream for concurrent read/write
        let (read_half, write_half) = stream.into_split();
        let read_half = Arc::new(Mutex::new(read_half));
        let write_half = Arc::new(Mutex::new(write_half));

        // Task: forward broadcast frames to this client
        let write_half_clone = write_half.clone();
        let writer_task = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                let mut wh = write_half_clone.lock().await;
                if wh.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });

        // Read frames from client
        let pty_handle = self.inner.pty_handle.clone();
        let input_tx = self.inner.input_tx.clone();
        let reader_task = tokio::spawn(async move {
            let mut rh = read_half.lock().await;
            #[allow(clippy::while_let_loop)]
            loop {
                // Read frame header
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
                    FRAME_PTY_INPUT => {
                        if let Err(e) = pty_handle.write_input(&payload) {
                            debug!(error = %e, "failed to write to PTY");
                            break;
                        }
                        let _ = input_tx.send(());
                    }
                    FRAME_RESIZE => {
                        if payload.len() >= 4 {
                            let rows = u16::from_be_bytes([payload[0], payload[1]]);
                            let cols = u16::from_be_bytes([payload[2], payload[3]]);
                            let _ = pty_handle.resize(rows, cols);
                        }
                    }
                    FRAME_DETACH => {
                        debug!(client_id = client_id, "client detached");
                        break;
                    }
                    _ => {
                        warn!(frame_type = frame_type, "unknown frame type from client");
                    }
                }
            }
        });

        // Wait for either task to finish
        tokio::select! {
            _ = writer_task => {}
            _ = reader_task => {}
        }

        // Unregister client
        {
            let mut clients = self.inner.clients.lock().await;
            clients.remove(&client_id);
        }

        info!(client_id = client_id, "client disconnected");
        Ok(())
    }

    /// Broadcast a pre-encoded frame to all connected clients
    async fn broadcast_frame(&self, frame: &[u8]) {
        let clients = self.inner.clients.lock().await;
        for (_id, client) in clients.iter() {
            let _ = client.tx.send(frame.to_vec()).await;
        }
    }

    /// Broadcast session end to all clients
    pub async fn broadcast_session_end(&self, exit_code: Option<i32>) {
        let payload = serde_json::json!({ "exit_code": exit_code }).to_string();
        let frame = encode_frame(FRAME_SESSION_END, payload.as_bytes());
        self.broadcast_frame(&frame).await;
    }

    /// Broadcast an alert to all clients
    pub async fn broadcast_alert(&self, message: &str) {
        let frame = encode_frame(FRAME_ALERT, message.as_bytes());
        self.broadcast_frame(&frame).await;
        // Also send via the alert channel for the watcher
        let _ = self.inner.alert_tx.send(frame);
    }

    /// Subscribe to input events (notified when a client sends PTY input)
    pub fn subscribe_input(&self) -> broadcast::Receiver<()> {
        self.inner.input_tx.subscribe()
    }

    /// Shutdown the server
    pub async fn shutdown(&self) {
        self.inner.shutdown_notify.notify_one();
    }
}
