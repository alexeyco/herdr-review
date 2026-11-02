# Review Scope Selection

## Purpose

When the plugin runs in pane mode it must decide what revdiff is about to
review before the TUI opens. This capability resolves the review directory —
preferring the directory the action exported through `HERDR_REVIEWS_DIR` and
falling back to the pane process working directory — checks that the herdr
environment is usable, then runs a sequence of strictly read-only git probes
in that directory to classify the repository state. From that classification
it selects one of five review modes (default, all-files, last-commit,
staged, untracked), announces the choice on stdout, and maps the mode to the
revdiff flags or positional revision used later by the execution path. The
probes never mutate the git index or working tree, and any unexpected probe
failure degrades silently to the bare default mode rather than aborting the
review.

## Requirements

### Requirement: Review directory resolution

Pane mode SHALL resolve the review directory by preferring a non-blank
`HERDR_REVIEWS_DIR` environment variable (value trimmed; whitespace-only or
empty counts as absent), falling back to the process current directory, and
failing with `cannot read current directory: <err>` only when that fallback
`current_dir()` call itself fails (a non-blank `HERDR_REVIEWS_DIR`, even a
bogus one, never reaches this error)
(src/review.rs:141-149, :333-338).

#### Scenario: Action-exported directory wins

- **WHEN** pane mode starts with `HERDR_REVIEWS_DIR` set to a non-blank path
- **THEN** that trimmed path is used as the review directory and the process
  cwd is not consulted

#### Scenario: Blank variable falls back to cwd

- **WHEN** pane mode starts with `HERDR_REVIEWS_DIR` unset, empty, or
  whitespace-only
- **THEN** the process current directory is used as the review directory

#### Scenario: Unreadable cwd is an error

- **WHEN** `HERDR_REVIEWS_DIR` is absent and reading the current directory
  fails
- **THEN** pane mode returns the error `cannot read current directory: <err>`

### Requirement: Herdr environment pre-check before the reviewer starts

Before spawning the selected review tool or reporting a scope, pane mode
SHALL call `herdr::require_bin()` so a missing or blank `HERDR_BIN_PATH` is
reported as an environment error before the TUI opens (src/herdr.rs:38-42).
The check runs identically whichever catalog tool was selected, and before
any git probe.

#### Scenario: Missing HERDR_BIN_PATH fails early

- **WHEN** pane mode starts without a usable `HERDR_BIN_PATH`
- **THEN** it fails before running any git probe or printing the scope line,
  with the message `herdr did not provide HERDR_BIN_PATH (plugin API v1
contract); cannot call back into herdr`

### Requirement: Read-only git probe

The plugin SHALL use read-only git probes in the resolved directory to
detect repository, HEAD, staged, unstaged, and untracked state. Only a
clean tree proceeds to the attached-branch check and base lookup, in order:
`origin/HEAD`, `origin/main`, then `origin/master`. Stop at the first valid
base; do not use the former `HEAD~1` probe. Probe outcomes SHALL follow the
documented exit-code meanings.

#### Scenario: Probe stops early for a non-repo directory

- **WHEN** `git rev-parse --is-inside-work-tree` in the directory does not
  succeed with trimmed output `true`
- **THEN** the probe returns the non-repo status without running the
  remaining commands

#### Scenario: Probe stops early when HEAD does not exist

- **WHEN** the directory is a work tree but `git rev-parse -q --verify HEAD`
  exits non-zero
- **THEN** the probe returns `repo = true` with `has_head = false` and skips
  the remaining commands

#### Scenario: Full probe on a repository with commits

- **WHEN** the directory is a work tree with a HEAD commit
- **THEN** `unstaged`, `staged`, and `untracked` are each determined by
  their respective read-only commands, and on a clean tree the detached
  check and base chain run to determine `detached` and `base`

#### Scenario: Dirty tree skips the base chain

- **WHEN** any of the staged, unstaged, or untracked probes reports changes
- **THEN** the detached-HEAD and base-chain probes do not run

#### Scenario: Base chain stops at the first hit

- **WHEN** the tree is clean, HEAD is attached, and `origin/HEAD` does not
  verify but `origin/main` does
- **THEN** the probe records `origin/main` as the base and never checks
  `origin/master`

### Requirement: Two-phase scope selection

The plugin SHALL select `NothingToReview` for non-repositories or missing HEAD,
`Uncommitted` for dirty trees, and `CleanCommitted` otherwise. Preserve
attachment and base state. Map clean scope to revdiff `AllFiles`; tuicr and
lumen use `VsBase` only with attached HEAD and a resolved base, otherwise
hold. Announce the scope; `NothingToReview` exits 0 without a tool or output
file.

#### Scenario: Dirty tree reviews uncommitted changes

- **WHEN** the probe reports a repo with HEAD and any staged, unstaged, or
  untracked changes — including a mixed staged+unstaged tree or
  untracked-only files
- **THEN** `Uncommitted` is selected and stdout shows `herdr-review:
reviewing uncommitted changes`

#### Scenario: Clean tree with a base reviews against it

- **WHEN** the probe reports a clean tree, an attached HEAD, and
  `origin/main` as the first base that verifies
- **THEN** `CleanCommitted` is selected; revdiff announces that it is
  reviewing all files, while tuicr and lumen announce
  `herdr-review: reviewing vs origin/main`

#### Scenario: Clean tree without any base is tool-specific

- **WHEN** the tree is clean and none of `origin/HEAD`, `origin/main`,
  `origin/master` verifies
- **THEN** revdiff announces that it is reviewing all files; tuicr and
  lumen show `herdr-review: nothing to review`, hold, and do not spawn

#### Scenario: Detached HEAD on a clean tree is tool-specific

- **WHEN** the tree is clean and `git symbolic-ref -q HEAD` reports a
  detached HEAD
- **THEN** revdiff announces that it is reviewing all files; tuicr and
  lumen select `NothingToReview` even when a base branch exists

#### Scenario: Non-repo directory holds

- **WHEN** the resolved directory is not inside a git work tree
- **THEN** the scope is `NothingToReview` with no user-facing "not a git
  repo" error beyond the hold path

#### Scenario: Repo without commits holds

- **WHEN** the directory is a work tree but `git rev-parse -q --verify HEAD`
  fails, even with untracked files present
- **THEN** the scope is `NothingToReview`

### Requirement: Scope maps to per-tool argv fragments

Each scope SHALL expose per-tool argv fragments. For revdiff, map
staged-only to `--staged`, mixed staged and working-tree changes to
positional `HEAD` with optional `--untracked`, unstaged-only to the default
diff, untracked-only to `--untracked`, and clean committed scope to
`--all-files`. tuicr and lumen retain their existing arguments. Explicit
`VsBase` uses each tool's documented refs or range; order revdiff scope
flags before fixed flags and refs after them.

#### Scenario: Uncommitted fragments per tool

- **WHEN** the scope is `Uncommitted` with no staged or untracked changes
- **THEN** revdiff uses its default working-tree diff, tuicr gets `["-w"]`, and
  lumen gets `[]`

#### Scenario: Staged-only changes map to the staged flag

- **WHEN** the scope has staged changes and no unstaged or untracked changes
- **THEN** revdiff gets `["--staged"]` without `--untracked`

#### Scenario: Mixed staged and working-tree changes map to both categories

- **WHEN** the scope has staged changes and also unstaged changes or
  untracked files
- **THEN** revdiff gets the positional base `HEAD`, which includes staged
  and unstaged tracked changes

#### Scenario: Mixed staged and untracked changes map to HEAD and untracked

- **WHEN** the scope has staged changes and untracked files
- **THEN** revdiff gets positional base `HEAD` and `--untracked`

#### Scenario: Mixed staged, unstaged, and untracked changes map to all categories

- **WHEN** the scope has staged changes, unstaged changes, and untracked files
- **THEN** revdiff gets positional base `HEAD` and `--untracked`

#### Scenario: Unstaged-only changes use the default working-tree diff

- **WHEN** the scope has unstaged changes and no staged or untracked changes
- **THEN** revdiff uses its default working-tree diff without `--staged` or
  `--untracked`

#### Scenario: Untracked-only changes include untracked files

- **WHEN** the scope has untracked files and no staged or unstaged changes
- **THEN** revdiff gets `--untracked` without `--staged`

#### Scenario: Other tool mappings remain unchanged

- **WHEN** any `Uncommitted` scope is mapped for tuicr or lumen
- **THEN** tuicr gets `["-w"]` and lumen gets `[]`

#### Scenario: Clean committed repository lets revdiff browse all files

- **WHEN** the scope is `CleanCommitted`
- **THEN** revdiff gets `["--all-files"]`, regardless of branch attachment
  or whether a base resolved

#### Scenario: Vs-base fragments per tool

- **WHEN** the scope is `VsBase(origin/main)`
- **THEN** the fragments are the positionals `["origin/main", "HEAD"]` for
  revdiff, `["-r", "origin/main..HEAD"]` for tuicr, and
  `["origin/main...HEAD"]` for lumen

#### Scenario: revdiff uses two positional refs, not a range argument

- **WHEN** the revdiff fragment for `VsBase(<base>)` is built
- **THEN** it is `<base> HEAD` as two positionals, not a single range
  argument

### Requirement: Unexpected probe failures degrade to the nothing-to-review hold

Any unexpected git failure during probing (spawn error, or an exit code
outside the documented meanings) SHALL print one warning to stderr in the
form `herdr-review: git probe failed (<cmd>): <err>; nothing to review`,
select the `NothingToReview` scope, print `herdr-review: nothing to
review`, hold the pane, and exit 0. No tool is spawned on this path.

#### Scenario: Unexpected diff exit code holds

- **WHEN** `git diff --quiet` exits with a code other than 0 or 1
- **THEN** a single probe-failure warning naming the command is written to
  stderr and the pane takes the nothing-to-review hold path

#### Scenario: Probe spawn failure holds

- **WHEN** a probe command cannot be spawned at all
- **THEN** the same warning and hold path apply instead of aborting with a
  hard error
