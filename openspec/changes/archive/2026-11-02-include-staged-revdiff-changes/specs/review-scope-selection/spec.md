## MODIFIED Requirements

### Requirement: Two-phase scope selection

From the probed Git state the plugin SHALL select a tool-aware review mode. A non-repo or repo without HEAD SHALL select `NothingToReview`. Any staged, unstaged, or untracked changes SHALL select `Uncommitted`. A clean repository with HEAD SHALL select a clean committed scope regardless of attachment or base availability. In that scope, revdiff SHALL browse all files; tuicr and lumen SHALL retain the existing clean-tree behavior: compare against the resolved base only when HEAD is attached and a base resolves, otherwise hold with `NothingToReview`. Pane mode SHALL announce the selected scope and SHALL hold without spawning a tool when that tool has no reviewable scope.

#### Scenario: Dirty tree reviews uncommitted changes

- **WHEN** the probe reports a repo with HEAD and any staged, unstaged, or untracked changes — including a mixed staged+unstaged tree or untracked-only files
- **THEN** `Uncommitted` is selected and stdout shows `herdr-review: reviewing uncommitted changes`

#### Scenario: Clean tree with a base reviews against it

- **WHEN** the probe reports a clean tree with HEAD, attached HEAD, and `origin/main` as the first base that verifies
- **THEN** the clean committed scope is selected; revdiff receives `--all-files`, while tuicr and lumen retain their base comparison against `origin/main`

#### Scenario: Clean tree without any base is tool-specific

- **WHEN** the tree is clean with HEAD and none of `origin/HEAD`, `origin/main`, `origin/master` verifies
- **THEN** revdiff receives `--all-files`; tuicr and lumen show `herdr-review: nothing to review`, hold, and do not spawn

#### Scenario: Detached HEAD on a clean tree is tool-specific

- **WHEN** the tree is clean with HEAD and `git symbolic-ref -q HEAD` reports a detached HEAD
- **THEN** revdiff receives `--all-files`; tuicr and lumen show `herdr-review: nothing to review`, hold, and do not spawn

#### Scenario: Non-repo directory holds

- **WHEN** the resolved directory is not inside a git work tree
- **THEN** the scope is `NothingToReview` with no user-facing "not a git repo" error beyond the hold path

#### Scenario: Repo without commits holds

- **WHEN** the directory is a work tree but `git rev-parse -q --verify HEAD` fails, even with untracked files present
- **THEN** the scope is `NothingToReview` for all tools

### Requirement: Scope maps to per-tool argv fragments

The review scope SHALL preserve the repository state needed to choose argv per tool. For revdiff, staged-only changes SHALL map to `--staged`. When staged changes coexist with unstaged or untracked changes, revdiff SHALL compare the working tree against `HEAD` using the single positional ref `HEAD`; it SHALL add `--untracked` only when untracked files exist. Unstaged-only changes SHALL use revdiff's default working-tree diff, and untracked-only changes SHALL add `--untracked`. When the repository has a HEAD commit and no staged, unstaged, or untracked changes, revdiff SHALL receive `--all-files`, regardless of whether HEAD is attached or a base ref resolves. A repository without a HEAD commit SHALL remain `NothingToReview`. tuicr and lumen SHALL retain their existing dirty-tree arguments and clean-tree base comparison/hold behavior.

#### Scenario: Uncommitted fragments per tool

- **WHEN** the scope is `Uncommitted` with no staged or untracked changes
- **THEN** revdiff uses its default working-tree diff, tuicr receives `-w`, and lumen receives no scope flags

#### Scenario: Vs-base fragments per tool

- **WHEN** an explicit `VsBase(origin/main)` comparison is selected
- **THEN** revdiff retains the `origin/main HEAD` positional pair, tuicr receives `-r origin/main..HEAD`, and lumen receives `origin/main...HEAD`; clean committed default selection uses a separate scope

#### Scenario: revdiff uses two positional refs, not a range argument

- **WHEN** an explicit `VsBase(<base>)` revdiff comparison is selected
- **THEN** it remains `<base> HEAD` as two positionals, not a single range argument; the plugin's clean committed default uses `--all-files` instead

#### Scenario: Staged-only changes map to the staged flag

- **WHEN** the repository has staged changes and no unstaged or untracked changes
- **THEN** revdiff receives `--staged` and does not receive `--untracked`

#### Scenario: Mixed staged and working-tree changes map to both categories

- **WHEN** the repository has staged and unstaged changes
- **THEN** revdiff receives the positional base `HEAD`, which includes staged and unstaged tracked changes

#### Scenario: Mixed staged and untracked changes map to HEAD and untracked

- **WHEN** the repository has staged changes and untracked files
- **THEN** revdiff receives positional base `HEAD` and `--untracked`

#### Scenario: Mixed staged, unstaged, and untracked changes map to all categories

- **WHEN** the repository has staged changes, unstaged changes, and untracked files
- **THEN** revdiff receives positional base `HEAD` and `--untracked`

#### Scenario: Unstaged-only changes use the default working-tree diff

- **WHEN** the repository has unstaged changes but no staged or untracked changes
- **THEN** revdiff uses its default working-tree diff and does not receive `--staged` or `--untracked`

#### Scenario: Untracked-only changes include untracked files

- **WHEN** the repository has untracked files but no staged or unstaged changes
- **THEN** revdiff receives `--untracked` and does not receive `--staged`

#### Scenario: Other tool mappings remain unchanged

- **WHEN** an `Uncommitted` scope is built for tuicr or lumen
- **THEN** tuicr receives `-w` and lumen receives no scope flags

#### Scenario: Clean committed repository lets revdiff browse all files

- **WHEN** the repository has a HEAD commit and no staged, unstaged, or untracked changes
- **THEN** revdiff receives `--all-files` whether HEAD is attached or detached and whether or not a base ref resolves
