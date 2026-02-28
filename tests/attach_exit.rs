//! Regression test for: "when llmux exits, shell prompt requires pressing Enter"
//!
//! Root cause: after a session ends, the tokio Runtime::drop() blocks waiting for
//! a spawn_blocking task stuck on stdin.read(). Fix: use shutdown_background()
//! instead of letting the runtime drop normally.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Demonstrates the bug: a runtime with a blocking reader hangs on drop.
///
/// This test runs the buggy pattern (normal runtime drop) in a thread with a
/// timeout. The runtime drop blocks because spawn_blocking is stuck reading
/// from a pipe that never receives data — exactly like stdin when nobody types.
#[test]
fn bug_runtime_drop_hangs_with_blocking_reader() {
    let completed = Arc::new(AtomicBool::new(false));
    let completed_clone = completed.clone();

    let handle = std::thread::spawn(move || {
        // Create a pipe where the read end will block forever (simulates stdin)
        let (read_end, _write_end) = UnixStream::pair().unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Spawn a task with a blocking reader — mirrors the writer task in attach()
            tokio::spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    let mut pipe = read_end;
                    let mut buf = [0u8; 1024];
                    let _ = pipe.read(&mut buf); // blocks forever
                })
                .await;
            });

            // Give the blocking task time to start
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Simulate: session ends, select! returns, attach() returns
            // block_on returns here...
        });

        // BUG: rt.drop() waits for the spawn_blocking thread to complete.
        // Since nobody writes to the pipe, this hangs forever.
        // (rt drops here at end of scope)
        drop(rt);

        completed_clone.store(true, Ordering::Relaxed);
    });

    // Wait up to 500ms — if the bug is present, it will hang much longer
    std::thread::sleep(Duration::from_millis(500));
    let did_complete = completed.load(Ordering::Relaxed);

    // Don't join (it would block forever if the bug is present).
    // The thread will be cleaned up on process exit.

    assert!(
        !did_complete,
        "BUG REPRODUCED: runtime drop should hang with a pending spawn_blocking reader"
    );

    // Clean up: we can't easily stop the thread, but the test process will exit.
    // Detach the thread so it doesn't block test runner shutdown.
    drop(handle);
}

/// Demonstrates the fix: shutdown_background() returns immediately despite
/// a pending spawn_blocking reader.
#[test]
fn fix_shutdown_background_exits_promptly_with_blocking_reader() {
    // Create a pipe where the read end will block forever (simulates stdin)
    let (read_end, _write_end) = UnixStream::pair().unwrap();

    let start = Instant::now();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Spawn a task with a blocking reader — mirrors the writer task in attach()
        tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let mut pipe = read_end;
                let mut buf = [0u8; 1024];
                let _ = pipe.read(&mut buf); // blocks forever
            })
            .await;
        });

        // Give the blocking task time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Simulate: session ends, select! returns, attach() returns
    });

    // FIX: shutdown_background() does NOT wait for blocking tasks
    rt.shutdown_background();

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "FIX: runtime should shut down in <1s with shutdown_background(), took {:?}",
        elapsed
    );
}
