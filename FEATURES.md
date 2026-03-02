# Features

## Backlog

- [ ] The default invocation of llmux (i.e. no arguments) should be to attach to the first session in the current dir, and if none exist, spawn a new one
- [x] Each command should be accessible through its first letter (s for spawn, etc.) except for config (c for clean)
- [x] `--agent` accepts any CLI command, not just predefined agent names
- [x] Auto-detect first available agent when `--agent` is omitted (checks claude, cursor, then config-defined agents in PATH order)
- [x] `llmux temp` / `llmux t` — spawn a session in an empty temporary directory
