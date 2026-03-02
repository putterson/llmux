# Command Aliases

## What It Does

Every llmux subcommand has a short alias for faster invocation:

| Command       | Alias |
|---------------|-------|
| `spawn`       | `s`   |
| `temp`        | `t`   |
| `ls`          | `l`   |
| `attach`      | `a`   |
| `history`     | `H`   |
| `resume`      | `r`   |
| `kill`        | `k`   |
| `clean`       | `c`   |
| `debug-input` | `di`  |
| `config`      | *(none — `c` is reserved for `clean`)* |

`h` is left free so that `llmux h` falls through to clap's built-in `help` subcommand.

`ls` also has the hidden alias `list`.
`_serve` is hidden/internal and gets no alias.

All aliases are visible in `llmux --help` output.

## Data Model Changes

None. This is purely a CLI parsing change.

## UI Changes

- `llmux s "fix the bug"` works the same as `llmux spawn "fix the bug"`
- `llmux t` works the same as `llmux temp`
- `llmux --help` shows aliases next to each command (e.g. `spawn [aliases: s]`)

## Edge Cases and Constraints

- **No first-letter conflicts**: All commands have unique first letters except `config`/`clean`, `history`/`help`, and `temp`/`_serve` (hidden) — resolved by giving `c` to `clean`, `H` (uppercase) to `history`, and leaving `h` for clap's built-in `help`.
- **`ls` already lowercased**: The alias `l` is unambiguous and doesn't conflict.
- **Existing `list` alias on `ls`**: Preserved as a hidden alias alongside the visible `l` alias.
- **`debug-input`**: Uses `di` since `d` could conflict with future commands.
