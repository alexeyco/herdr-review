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
