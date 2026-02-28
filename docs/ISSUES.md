# Issues

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
