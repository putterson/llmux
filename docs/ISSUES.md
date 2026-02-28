# Issues

## No shell prompt after llmux exits (must press Enter)

**Status**: Fixed

**Symptoms**:
When a session ends (the agent exits), the llmux process does not return
control to the shell immediately. The user must press Enter to get their
shell prompt back.

**Root cause**: In `src/session/attach.rs`, the `attach()` function uses
`tokio::task::spawn_blocking` to read from stdin. When a session ends, the
reader task finishes and `tokio::select!` returns, but the writer task's
`spawn_blocking` thread remains blocked on `stdin.read()`. Back in `main.rs`,
each caller creates a `tokio::runtime::Runtime` and calls `rt.block_on()`.
When `block_on` returns and the `Runtime` drops, `Runtime::drop()` waits for
**all** tasks including the stuck `spawn_blocking` thread. The process hangs
until the user presses a key, which unblocks the read.

**Fix**: Two changes:
1. `src/main.rs` — Added `run_attach()` helper that calls
   `rt.shutdown_background()` after `block_on()` returns instead of letting
   the runtime drop normally. `shutdown_background()` abandons pending
   blocking tasks. Updated all 4 callers (`cmd_default`, `cmd_spawn`,
   `cmd_attach`, `cmd_resume`).
2. `src/session/attach.rs` — Added `session_ended` flag so the "Detached from
   session" message is only shown on actual detach, not when the session ended
   (which already prints its own message).

**Files changed**:
- `src/main.rs` — added `run_attach()`, updated 4 callers
- `src/session/attach.rs` — added `session_ended` flag, conditional detach message

**Test files**:
- `tests/attach_exit.rs` — 2 tests: bug reproduction (runtime drop hangs with
  blocking reader) and fix verification (shutdown_background exits promptly)

**Specification**:
- `docs/attach-exit.md` — invariants and edge cases for attach exit behavior

---

## Terminal bell false triggers on focus/defocus and after user input

**Status**: Fixed

**Symptoms**:
1. Focusing or defocusing a terminal window containing an llmux session
   triggers a terminal bell (idle alert).
2. After user input, the bell triggers shortly afterward even though the agent
   may still be processing.

**Root cause**: In `src/session/watcher.rs`, the `had_activity` flag was set to
`true` on ANY PTY output regardless of size, but `last_output_time` was only
reset for substantial output (>128 bytes/sec). This mismatch meant trivial
output — resize-triggered screen redraws (~20 bytes) and input echo (~few
bytes) — enabled idle monitoring while the timer was stale, causing immediate
false alerts. Additionally, user input did not reset `last_output_time`, so
after typing, the stale timer would fire an alert as soon as the echo set
`had_activity`.

**Fix**: Two changes in `src/session/watcher.rs`:
1. Moved `had_activity = true` from the output receive handler into the
   per-second timer block, guarded by `bytes_this_second > 128`. Only
   substantial output now counts as "activity" for idle monitoring.
2. Added `last_output_time = Instant::now()` to the input receive handler,
   so user input resets the idle timer and gives the agent a fresh window
   to respond.

**Files changed**:
- `src/session/watcher.rs` — core fix (3 lines changed)

**Test files**:
- `tests/watcher_bell.rs` — 7 tests covering both bug reproduction and
  correct behavior

**Specification**:
- `docs/idle-alerts.md` — invariants and edge cases for the idle alert system
