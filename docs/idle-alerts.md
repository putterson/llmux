# Idle Alert Specification

## Purpose

The idle alert system notifies the user when an agent appears to have stopped
producing output and may be waiting for input. It fires via terminal bell,
desktop notification, and/or a custom command.

## Definitions

- **Substantial output**: PTY output exceeding 128 bytes within a single tick
  interval (~1 second). This threshold filters out low-bandwidth animations
  like blinking cursors and spinner dots (~20-60 bytes/sec).
- **Trivial output**: PTY output of 128 bytes or fewer per tick. Includes
  resize-triggered screen redraws, input echo, and cursor animations.
- **User input**: Any keystrokes sent from an attached client to the PTY
  (FRAME_PTY_INPUT). Resize events (FRAME_RESIZE) are NOT user input.
- **Idle timeout**: Configurable duration (default 5 seconds) after the last
  substantial output before an idle alert fires.

## Invariants

1. **No alert without substantial output**: An idle alert MUST NOT fire unless
   the agent has previously produced substantial output (>128 bytes in a tick).
   This prevents false alerts on session startup, after resize events, and from
   input echo.

2. **Idle timer resets on substantial output**: When substantial output is
   detected, the idle timer resets. The agent is considered "active".

3. **Idle timer resets on user input**: When the user sends input, the idle
   timer resets. This gives the agent time to process the input and produce a
   response before being considered idle.

4. **Single alert per idle period**: Once an idle alert fires, no further
   alerts fire until either (a) new substantial output is detected, or (b) the
   user sends input followed by new substantial output.

5. **No alert on session startup**: The watcher starts with `had_activity =
   false`. An alert cannot fire until substantial output has been observed.

## Edge Cases

| Scenario | Expected behavior |
|---|---|
| Session starts, no output | No alert |
| Resize with no prior output | No alert (trivial output, had_activity stays false) |
| Resize after agent was active | No alert (trivial output doesn't flip had_activity) |
| User types, agent echoes input only | No alert (echo is trivial output) |
| User types, agent responds with >128B then stops | Alert after idle_timeout |
| Agent shows spinner (< 128B/tick) | No alert (treated as trivial) |
| Agent produces >128B/tick then stops | Alert after idle_timeout |
| Two rapid resizes | No alert (each produces trivial output) |
