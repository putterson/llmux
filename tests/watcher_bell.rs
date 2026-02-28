//! Tests for InputWatcher idle-alert logic.
//!
//! These tests exercise the core watcher loop through broadcast channels,
//! using short real-time timeouts to verify behavior.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;

/// Atomic alert counter — simple and thread-safe.
#[derive(Clone)]
struct AlertCounter(Arc<AtomicUsize>);

impl AlertCounter {
    fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
    fn increment(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// Reproduce the CURRENT (buggy) watcher logic.
/// This mirrors `InputWatcher::start` from watcher.rs.
fn spawn_buggy_watcher(
    idle_timeout: Duration,
    mut output_rx: broadcast::Receiver<Vec<u8>>,
    mut input_rx: broadcast::Receiver<()>,
    counter: AlertCounter,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_output_time = Instant::now();
        let mut idle_alerted = false;
        let mut had_activity = false;
        let mut bytes_this_second: usize = 0;

        loop {
            tokio::select! {
                result = input_rx.recv() => {
                    match result {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            had_activity = false;
                            idle_alerted = false;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                result = output_rx.recv() => {
                    match result {
                        Ok(data) => {
                            // BUG: sets had_activity on ANY output
                            had_activity = true;
                            bytes_this_second += data.len();
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if bytes_this_second > 128 {
                        last_output_time = Instant::now();
                        idle_alerted = false;
                    }
                    bytes_this_second = 0;

                    if had_activity && !idle_alerted && last_output_time.elapsed() >= idle_timeout {
                        counter.increment();
                        idle_alerted = true;
                    }
                }
            }
        }
    })
}

/// The FIXED watcher logic with the two key changes:
/// 1. Only set had_activity for substantial output (>128 bytes in a tick)
/// 2. Reset last_output_time on user input
fn spawn_fixed_watcher(
    idle_timeout: Duration,
    mut output_rx: broadcast::Receiver<Vec<u8>>,
    mut input_rx: broadcast::Receiver<()>,
    counter: AlertCounter,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_output_time = Instant::now();
        let mut idle_alerted = false;
        let mut had_activity = false;
        let mut bytes_this_second: usize = 0;

        loop {
            tokio::select! {
                result = input_rx.recv() => {
                    match result {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            had_activity = false;
                            idle_alerted = false;
                            // FIX: reset idle timer on user input
                            last_output_time = Instant::now();
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                result = output_rx.recv() => {
                    match result {
                        Ok(data) => {
                            // FIX: do NOT set had_activity here
                            bytes_this_second += data.len();
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if bytes_this_second > 128 {
                        had_activity = true;
                        last_output_time = Instant::now();
                        idle_alerted = false;
                    }
                    bytes_this_second = 0;

                    if had_activity && !idle_alerted && last_output_time.elapsed() >= idle_timeout {
                        counter.increment();
                        idle_alerted = true;
                    }
                }
            }
        }
    })
}

// Use short timeouts: 200ms idle timeout, 50ms tick interval

const IDLE_TIMEOUT: Duration = Duration::from_millis(200);
/// Wait long enough for the watcher to tick and detect idle state.
const SETTLE: Duration = Duration::from_millis(400);

// ---------------------------------------------------------------------------
// Bug reproduction tests
// ---------------------------------------------------------------------------

/// BUG: A resize (focus/defocus) causes a false idle alert.
///
/// The PTY emits a few bytes of screen-redraw output in response to a resize.
/// The watcher treats this as "activity" and fires an idle alert because
/// last_output_time is stale.
#[tokio::test]
async fn bug_resize_output_triggers_false_idle_alert() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (_input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_buggy_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Let last_output_time become stale
    tokio::time::sleep(IDLE_TIMEOUT + Duration::from_millis(100)).await;

    // Simulate resize-triggered PTY redraw (small output, ~20 bytes)
    output_tx.send(b"\x1b[1;1H\x1b[2J".to_vec()).unwrap();

    // Wait for the watcher to detect "idle"
    tokio::time::sleep(SETTLE).await;

    // BUG: alert fires even though this was just a resize redraw
    assert!(
        counter.count() > 0,
        "BUG REPRODUCED: expected false alert from resize output"
    );

    handle.abort();
}

/// BUG: After user input, the PTY echo triggers a false idle alert.
///
/// User input resets had_activity and idle_alerted. But the PTY echoes the
/// input back (~few bytes), which sets had_activity = true again. Since
/// last_output_time is stale, the idle alert fires shortly after.
#[tokio::test]
async fn bug_input_echo_triggers_false_idle_alert() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_buggy_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Let last_output_time become stale
    tokio::time::sleep(IDLE_TIMEOUT + Duration::from_millis(100)).await;

    // User types something
    input_tx.send(()).unwrap();
    // Small delay then PTY echoes (happens in real life within ms)
    tokio::time::sleep(Duration::from_millis(10)).await;
    output_tx.send(b"hello\r\n".to_vec()).unwrap();

    // Wait for watcher
    tokio::time::sleep(SETTLE).await;

    // BUG: alert fires because echo set had_activity and last_output_time is stale
    assert!(
        counter.count() > 0,
        "BUG REPRODUCED: expected false alert from input echo"
    );

    handle.abort();
}

// ---------------------------------------------------------------------------
// Fixed-behavior tests
// ---------------------------------------------------------------------------

/// FIXED: Resize output should NOT trigger an idle alert.
#[tokio::test]
async fn fixed_resize_output_does_not_trigger_idle_alert() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (_input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_fixed_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Let last_output_time become stale
    tokio::time::sleep(IDLE_TIMEOUT + Duration::from_millis(100)).await;

    // Simulate resize-triggered PTY redraw (small output)
    output_tx.send(b"\x1b[1;1H\x1b[2J".to_vec()).unwrap();

    // Wait for watcher
    tokio::time::sleep(SETTLE).await;

    assert_eq!(
        counter.count(),
        0,
        "FIXED: resize output should not trigger idle alert"
    );

    handle.abort();
}

/// FIXED: Input echo should NOT trigger an idle alert.
#[tokio::test]
async fn fixed_input_echo_does_not_trigger_idle_alert() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_fixed_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Let last_output_time become stale
    tokio::time::sleep(IDLE_TIMEOUT + Duration::from_millis(100)).await;

    // User types something
    input_tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    // PTY echoes the input back (small output)
    output_tx.send(b"hello\r\n".to_vec()).unwrap();

    // Wait for watcher
    tokio::time::sleep(SETTLE).await;

    assert_eq!(
        counter.count(),
        0,
        "FIXED: input echo should not trigger idle alert"
    );

    handle.abort();
}

/// FIXED: Substantial output followed by silence SHOULD still trigger idle alert.
#[tokio::test]
async fn fixed_substantial_output_then_idle_triggers_alert() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (_input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_fixed_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Send substantial output (>128 bytes)
    output_tx.send(vec![b'x'; 256]).unwrap();

    // Wait for the tick to register it + idle timeout + extra settle time
    tokio::time::sleep(Duration::from_millis(100) + IDLE_TIMEOUT + SETTLE).await;

    assert!(
        counter.count() > 0,
        "FIXED: substantial output then silence should trigger idle alert"
    );

    handle.abort();
}

/// FIXED: User input should reset the idle timer.
#[tokio::test]
async fn fixed_user_input_resets_idle_timer() {
    let (output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_fixed_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Substantial output → idle alert
    output_tx.send(vec![b'x'; 256]).unwrap();
    tokio::time::sleep(Duration::from_millis(100) + IDLE_TIMEOUT + SETTLE).await;
    assert!(counter.count() > 0, "should have initial idle alert");
    let initial_count = counter.count();

    // User sends input → resets timer and clears flags
    input_tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Agent responds with substantial output
    output_tx.send(vec![b'y'; 256]).unwrap();
    // Wait for tick + idle timeout
    tokio::time::sleep(Duration::from_millis(100) + IDLE_TIMEOUT + SETTLE).await;

    assert!(
        counter.count() > initial_count,
        "should get another idle alert after agent responds and goes idle again"
    );

    handle.abort();
}

/// FIXED: No false alert on startup (no activity has occurred).
#[tokio::test]
async fn fixed_no_alert_without_activity() {
    let (_output_tx, output_rx) = broadcast::channel::<Vec<u8>>(16);
    let (_input_tx, input_rx) = broadcast::channel::<()>(16);
    let counter = AlertCounter::new();

    let handle = spawn_fixed_watcher(IDLE_TIMEOUT, output_rx, input_rx, counter.clone());

    // Wait well past idle timeout with no output at all
    tokio::time::sleep(IDLE_TIMEOUT * 3).await;

    assert_eq!(
        counter.count(),
        0,
        "no alert should fire without any activity"
    );

    handle.abort();
}
