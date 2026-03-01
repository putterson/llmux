# Chord Detach (Ctrl+] q)

## Expected behavior

When the user presses Ctrl+] followed by q while attached to a session, the
client must detect the chord and send `FRAME_DETACH` to the server. This must
work regardless of whether the two keystrokes arrive in the same `read()` call
or in separate calls.

## Invariants

1. **Split reads**: Ctrl+] (`0x1D`) arrives alone in one read, then `q` arrives
   in the next read → detach.

2. **Batched reads**: `[0x1D, b'q']` arrives in a single read → detach.

3. **Mid-buffer chord**: `[..., 0x1D, b'q']` arrives in one read → forward
   bytes before the chord, then detach.

4. **Ctrl+] at end of buffer**: `[..., 0x1D]` → forward bytes before it, enter
   pending state, wait for next read.

5. **False alarm (split)**: Ctrl+] alone, then a non-q byte → forward `[0x1D]`
   plus the non-q input as normal PTY data.

6. **False alarm (batched)**: `[0x1D, non-q]` in one read → forward all bytes
   as normal PTY data.

7. **No chord byte**: Normal input with no `0x1D` → forward as-is.

8. **State resets after false alarm**: A false alarm fully resets the detector.
   A subsequent Ctrl+] q chord must still work.

## Chord actions

The `ChordDetector` supports multiple chord actions via `ChordAction`:

- **Ctrl+] q** → `ChordAction::Detach` — detach from the session
- **Ctrl+] d** → `ChordAction::Diagnostic` — print diagnostic information

## Diagnostic chord (Ctrl+] d)

Pressing Ctrl+] then d while attached to a session prints diagnostic info to
stderr without detaching:

- Session name and binary version
- Chord detector state (idle/pending)
- Last 64 raw input bytes as hex dump
- Terminal info (TERM env var, terminal size)
- Whether `LLMUX_DEBUG_INPUT` is enabled

This is useful for debugging chord detection issues — if Ctrl+] d works but
Ctrl+] q doesn't, the issue is in detach handling, not byte detection.

## Debug tools

### `llmux debug-input` (alias: `di`)

Standalone diagnostic that puts the terminal in raw mode and prints the hex
value of every byte received from stdin. Use this to verify what byte your
terminal emulator sends for Ctrl+]:

```
$ llmux debug-input
llmux debug-input — press keys to see hex values. Press Ctrl+C or q to exit.
Looking for Ctrl+] = 0x1d

  0x1d  GS  (Ctrl+]) *** DETACH TRIGGER ***
```

If you see a different value (e.g., an escape sequence), your terminal emulator
is remapping Ctrl+] to something else.

### `LLMUX_DEBUG_INPUT=1`

Set this environment variable before attaching to log every stdin read as hex
to stderr:

```
$ LLMUX_DEBUG_INPUT=1 llmux attach my-session
[LLMUX_DEBUG_INPUT] stdin 1 bytes: 1d
[LLMUX_DEBUG_INPUT] stdin 1 bytes: 71
```

## Edge cases

- **Empty input**: No action, no state change.
- **Pending state has no timeout**: If Ctrl+] is pressed and the user waits
  arbitrarily long before pressing q, the chord still fires. This is intentional
  — the pending state persists until the next keystroke.
- **Multiple `0x1D` in one buffer**: Only the first occurrence is significant.
  If it's a false alarm (followed by non-q), the remaining bytes (including any
  later `0x1D`) are forwarded as-is.

## Constraints

- The chord detector must be a pure state machine with no I/O, timers, or
  async dependencies — only input bytes and internal state.
- The detector is used inside the writer task's `spawn_blocking` loop in
  `attach.rs`, so it must be `Send`.
