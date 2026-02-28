# Attach Exit Behavior

## Expected behavior

When a session ends or the user detaches, the `llmux` process must exit
promptly and return control to the user's shell. The user should see their
shell prompt immediately without needing to press Enter or any other key.

## Invariants

1. **Prompt exit on session end**: When the agent process exits and the server
   sends `FRAME_SESSION_END`, the llmux client process must terminate within
   ~100ms — not block waiting for user input.

2. **Prompt exit on detach**: When the user presses the detach escape sequence
   (Ctrl+] then q), the process must exit promptly.

3. **Terminal restoration**: The terminal must be restored to its original
   (cooked) mode before the process exits, regardless of whether the session
   ended or the user detached.

4. **Accurate exit message**: The exit message should reflect what happened:
   - Session end: "Session ended (exit code: N)" (already printed by reader)
   - Detach: "Detached from session 'name'."

## Edge cases

- **User typing during session end**: If the user is mid-keystroke when the
  session ends, the process should still exit promptly. The pending
  `spawn_blocking` stdin read must not delay shutdown.

- **Multiple rapid session ends**: If multiple FRAME_SESSION_END frames arrive
  (shouldn't happen but could in edge cases), only the first should be
  processed.

- **Signal during exit**: If the user sends SIGINT during the exit sequence,
  the process should terminate without hanging.

## Constraints

- The fix must not change the behavior of `spawn_blocking` for stdin reading
  during normal operation — only the runtime shutdown path is affected.
- The tokio runtime must be shut down using `shutdown_background()` to avoid
  waiting for orphaned blocking threads.
