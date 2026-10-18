# Spec Delta

## ADDED Requirements

### Requirement: Two-phase scope selection

From the probed git state the plugin SHALL select exactly one scope with a
pure function: non-repo or repo without HEAD → `NothingToReview`; any
staged, unstaged, or untracked changes → `Uncommitted`; otherwise (clean
tree) → `VsBase(<base>)` when a base from the chain resolved and HEAD is
attached, else `NothingToReview`. Pane mode SHALL announce the selected
scope on stdout and, for `NothingToReview`, hold the pane and exit 0
without spawning a tool or creating an output file.

#### Scenario: Dirty tree reviews uncommitted changes

- **WHEN** the probe reports a repo with HEAD and any staged, unstaged, or
  untracked changes — including a mixed staged+unstaged tree or
  untracked-only files
- **THEN** `Uncommitted` is selected and stdout shows `herdr-review:
reviewing uncommitted changes`

#### Scenario: Clean tree with a base reviews against it

- **WHEN** the probe reports a clean tree, an attached HEAD, and
  `origin/main` as the first base that verifies
- **THEN** `VsBase(origin/main)` is selected and stdout shows
  `herdr-review: reviewing vs origin/main`

#### Scenario: Clean tree without any base holds

- **WHEN** the tree is clean and none of `origin/HEAD`, `origin/main`,
  `origin/master` verifies
- **THEN** stdout shows `herdr-review: nothing to review`, the pane is
  held, the process exits 0, and no tool is spawned

#### Scenario: Detached HEAD on a clean tree holds

- **WHEN** the tree is clean and `git symbolic-ref -q HEAD` reports a
  detached HEAD
- **THEN** the scope is `NothingToReview` even when a base branch exists

#### Scenario: Non-repo directory holds

- **WHEN** the resolved directory is not inside a git work tree
- **THEN** the scope is `NothingToReview` with no user-facing "not a git
  repo" error beyond the hold path

#### Scenario: Repo without commits holds

- **WHEN** the directory is a work tree but `git rev-parse -q --verify HEAD`
  fails, even with untracked files present
- **THEN** the scope is `NothingToReview`

### Requirement: Scope maps to per-tool argv fragments

Each scope SHALL expose a static argv fragment per catalog tool, consumed
by reviewer-execution: `Uncommitted` → revdiff `--untracked`, tuicr `-w`,
lumen nothing; `VsBase(<base>)` → revdiff positionals `<base> HEAD` (two
refs, no flags), tuicr `-r <base>..HEAD`, lumen positional
`<base>...HEAD`. revdiff positionals are placed after its fixed trailing
flags.
Evidence: `revdiff --help → against: second git ref for two-ref diff`; the
README documents `<base>..HEAD` as equivalent.

#### Scenario: Uncommitted fragments per tool

- **WHEN** the scope is `Uncommitted`
- **THEN** the fragments are `["--untracked"]` for revdiff, `["-w"]` for
  tuicr, and `[]` for lumen

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

## MODIFIED Requirements

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

The plugin SHALL probe the resolved directory by running, in order,
`git rev-parse --is-inside-work-tree`, `git rev-parse -q --verify HEAD`,
`git diff --quiet`, `git diff --cached --quiet`, and `git ls-files --others
--exclude-standard`, each with `current_dir(dir)`, and every command SHALL
be read-only so the index and working tree are never mutated — no catalog
tool may stage or mutate the branch either. Only when the tree is clean (no
staged, unstaged, or untracked changes) the probe SHALL continue with
`git symbolic-ref -q HEAD` (attached-branch check) and the base chain
`git rev-parse -q --verify origin/HEAD`, then `origin/main`, then
`origin/master`, stopping at the first base that verifies. The former
`HEAD~1` probe is gone. Exit-code meanings are fixed:
`--is-inside-work-tree` other than success with output `true` means
non-repo (not an error); `rev-parse -q --verify` exit 0 means the revision
exists; quiet diffs exit 0 for clean and 1 for changes; non-empty trimmed
stdout from `ls-files --others` means untracked files exist;
`symbolic-ref -q HEAD` nonzero means detached HEAD.

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

## REMOVED Requirements

### Requirement: Review mode selection

**Reason**: The five revdiff-specific modes (default, all-files,
last-commit, staged, untracked) are replaced by the two-phase dirty/clean
scope selection shared by all three catalog tools; a staged-only tree now
reviews as uncommitted changes, an accepted limitation of running each tool
once.
**Migration**: review-scope-selection → "Two-phase scope selection".

### Requirement: Silent fallback to the default mode

**Reason**: With three tools and base-derived scopes there is no safe
default review left to fall back to; a non-repo directory or failed probe
now holds the pane instead of launching a bare revdiff.
**Migration**: review-scope-selection → "Unexpected probe failures degrade
to the nothing-to-review hold" and the `NothingToReview` branch of
"Two-phase scope selection".

### Requirement: Mode maps to revdiff flags or a positional, never both

**Reason**: The revdiff-only flag/positional map is replaced by the
per-tool argv fragment table covering revdiff, tuicr, and lumen.
**Migration**: review-scope-selection → "Scope maps to per-tool argv
fragments".

## RENAMED Requirements

- FROM: `### Requirement: Herdr environment pre-check before revdiff starts`
- TO: `### Requirement: Herdr environment pre-check before the reviewer starts`
