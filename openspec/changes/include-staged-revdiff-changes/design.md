# Design: Include staged changes in revdiff reviews

## Approach

Keep the Git probe intact, but select a tool-aware clean committed scope whenever HEAD exists and the tree is clean. In that scope revdiff receives `--all-files` whether or not the branch is attached or a base exists. tuicr and lumen use the base comparison when attached HEAD and a base are available; otherwise they take their existing hold path. Keep `VsBase(<base>)` available for explicit two-ref revdiff comparisons, even though the clean-tree plugin flow uses the clean committed scope. For dirty repositories, retain staged, unstaged, and untracked booleans. Staged-only selects `--staged`. If staged changes coexist with unstaged or untracked changes, use `HEAD` as a single base ref so revdiff compares all tracked working-tree content against the commit; add `--untracked` only when untracked files exist. Unstaged-only uses revdiff's default working-tree diff, and untracked-only adds `--untracked`. Continue using the existing `-w` and empty fragments for tuicr and lumen.

## Files

- `src/adapters/git.rs`: carry the three dirty-state booleans in the uncommitted scope and map them to per-tool fragments; update unit coverage for every specified state.
- `src/features/review/process.rs`: place revdiff scope flags before fixed output flags and a mixed-state `HEAD` ref after them.
- `src/features/review/tests.rs`: update argv expectations and cover staged-only, mixed-state, unstaged-only, and untracked-only revdiff argv.
- `tests/e2e.rs`: verify mixed staged/unstaged and staged/untracked states reach revdiff with the `HEAD` base and optional `--untracked` flag.
- `openspec/specs/review-scope-selection/spec.md`: update the baseline mapping after implementation.
- `openspec/changes/include-staged-revdiff-changes/specs/review-scope-selection/spec.md`: define the changed mapping and acceptance scenarios.

## Data shape and trade-offs

The scope needs to retain the three dirty-state dimensions and distinguish a clean committed repository with no base from a repository that has no commits. This avoids extra Git probes. Tool-aware handling of the clean committed scope allows revdiff to browse all files while preserving tuicr and lumen's existing base comparison or hold behavior. `VsBase(<base>)` remains the explicit two-ref comparison mode. For mixed staged and working-tree changes, a `HEAD` base includes staged and unstaged tracked edits in one comparison; `--untracked` adds untracked files when present. Local `revdiff --help` describes `base` as the git ref to diff against, and `--untracked` as showing untracked files in the tree.
