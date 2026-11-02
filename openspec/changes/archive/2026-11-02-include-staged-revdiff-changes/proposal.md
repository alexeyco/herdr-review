# Include staged changes in revdiff reviews

## Why now

The review scope detects staged changes, but the revdiff invocation currently receives only `--untracked`. A repository with staged changes and no working-tree changes therefore opens revdiff without a diff to review. A fully committed repository currently falls through to a base comparison or “nothing to review,” even though revdiff can browse all tracked files.

## What changes

Preserve enough Git status information to choose revdiff's input for each repository state. A staged-only tree reviews staged changes only. When staged changes coexist with unstaged or untracked changes, revdiff compares the working tree against `HEAD`, adding `--untracked` only when untracked files exist; this includes staged and unstaged tracked edits together. Unstaged-only changes use revdiff's default working-tree diff, and untracked-only changes add `--untracked`. A clean committed repository lets revdiff browse all files with `--all-files`, even without an attached branch or detected base. tuicr and lumen retain their current clean-tree behavior and dirty-tree arguments.

## Capabilities

- `review-scope-selection`: refine per-tool argument selection for dirty Git states while preserving existing scope detection.

REJECTED-RISK: None.
