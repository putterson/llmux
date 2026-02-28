# Single-Letter Command Aliases

## What It Does

Every llmux subcommand becomes accessible via its first letter, allowing faster invocation:

| Command   | Alias |
|-----------|-------|
| `spawn`   | `s`   |
| `ls`      | `l`   |
| `attach`  | `a`   |
| `history` | `H`   |
| `resume`  | `r`   |
| `kill`    | `k`   |
| `clean`   | `c`   |
| `config`  | *(none — `c` is reserved for `clean`)* |

`h` is left free so that `llmux h` falls through to clap's built-in `help` subcommand.

`ls` already has the alias `list`; it keeps that and gains `l`.
`_serve` is hidden/internal and gets no alias.

## Scope

**In scope:**
- Add a `#[command(alias = "X")]` for each command listed above in `src/cli.rs`

**Out of scope:**
- Changing default behavior when no subcommand is provided (separate backlog item)
- Adding multi-letter abbreviations beyond the existing `list` alias
- Tab completion (handled by shell integration, not this change)

## Data Model Changes

None. This is purely a CLI parsing change.

## UI Changes

- `llmux s "fix the bug"` works the same as `llmux spawn "fix the bug"`
- `llmux --help` will show aliases in the help text automatically (clap behavior)

## Edge Cases and Constraints

- **No first-letter conflicts**: All commands have unique first letters except `config`/`clean` and `history`/`help` — resolved by giving `c` to `clean`, `H` (uppercase) to `history`, and leaving `h` for clap's built-in `help`.
- **`ls` already lowercased**: The alias `l` is unambiguous and doesn't conflict.
- **Existing `list` alias on `ls`**: Preserved alongside the new `l` alias.
