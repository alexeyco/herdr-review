# Reviewer Execution

## Purpose

The reviewer-execution capability covers how the review pane process
selects, locates, and runs the chosen review tool and reads its result
back: the tool comes from the injected `HERDR_REVIEW_TOOL` environment,
binaries resolve from `PATH` only, the collision-safe `.out` file model and
7-day pruning are shared by all three tools, and each tool has its own
spawn/stdio strategy and post-exit collection path. All three tools are
human diff-review TUIs; annotations are authored by the user inside the
tool.

## Requirements

### Requirement: The selected tool comes from the pane environment

The review pane process SHALL read the tool id from `HERDR_REVIEW_TOOL`
(trimmed). An unset or blank value SHALL fail with `HERDR_REVIEW_TOOL is
not set; open the review pane through the review action`; any other unknown
value SHALL fail with `unknown review tool <value>; expected revdiff, tuicr
or lumen`. Both errors follow the pane-mode error policy (held, exit 1) and
precede any git probe or spawn.

#### Scenario: Valid tool id selects the tool

- **WHEN** `HERDR_REVIEW_TOOL` is `lumen`
- **THEN** the pane runs the lumen strategy and no other tool is considered

#### Scenario: Missing tool id is an error

- **WHEN** `HERDR_REVIEW_TOOL` is unset or whitespace-only in the review
  pane
- **THEN** the pane fails with `HERDR_REVIEW_TOOL is not set; open the
review pane through the review action`

#### Scenario: Unknown tool id is an error

- **WHEN** `HERDR_REVIEW_TOOL` is `diffy`
- **THEN** the pane fails with `unknown review tool diffy; expected
revdiff, tuicr or lumen`

### Requirement: Tool binaries resolve from PATH only

The pane process SHALL resolve the selected tool's binary immediately
before spawning it: the first `PATH` entry, scanned in order with empty
entries skipped, that contains an executable regular file named `<tool>`
(on unix at least one execute bit, `mode & 0o111`), canonicalized to an
absolute path. No environment-variable override exists for any tool:
`REVDIFF_BIN` is removed and SHALL NOT be read.

#### Scenario: First executable hit wins

- **WHEN** two `PATH` entries contain an executable `revdiff`
- **THEN** resolution returns the canonicalized path from the earlier entry

#### Scenario: No binary anywhere

- **WHEN** no `PATH` entry contains an executable `tuicr`
- **THEN** resolution fails with `tuicr not found on PATH; install it with
brew install tuicr`, using the catalog's install hint for that tool

#### Scenario: Candidate cannot be canonicalized

- **WHEN** an executable candidate is found but `canonicalize` fails
- **THEN** resolution fails with `found <tool> at <candidate> but cannot
resolve it to an absolute path: <error>` and no cwd-relative guess is
  used

#### Scenario: REVDIFF_BIN is ignored

- **WHEN** `REVDIFF_BIN` points at an executable file and `PATH` also
  contains an executable `revdiff`
- **THEN** resolution returns the `PATH` hit; the variable is never read

### Requirement: Create the annotation output file under the state directory

The pane process SHALL keep review annotations in
`<state_dir>/reviews/<pid>-<unix millis>.out`, the pid prefix keeping
same-millisecond reviews collision-safe and the filename generated after
pruning; `state_dir` is the trimmed value of `HERDR_PLUGIN_STATE_DIR` or
the system temporary directory when that is unset or blank; the `reviews`
directory is created on demand. revdiff writes the file itself; for tuicr
and lumen the plugin writes the collected canonical text into it.

#### Scenario: Output path layout

- **WHEN** `HERDR_PLUGIN_STATE_DIR` is set to `/state` and the pane process
  has pid 4242 at unix millisecond 1700000000000
- **THEN** the output file path is `/state/reviews/4242-1700000000000.out`

#### Scenario: State directory created on demand

- **WHEN** `<state_dir>/reviews` does not exist at pane start
- **THEN** the pane process creates it before spawning the tool

#### Scenario: State directory creation failure is fatal

- **WHEN** creating `<state_dir>/reviews` fails
- **THEN** the run fails with `failed to create state dir <path>: <error>`

#### Scenario: System clock before the epoch is fatal

- **WHEN** the system clock is before the Unix epoch when the millisecond
  stamp is taken
- **THEN** the run fails with `system clock error: <error>`

#### Scenario: Missing state-dir variable uses the temp directory

- **WHEN** `HERDR_PLUGIN_STATE_DIR` is unset, empty, or whitespace-only
- **THEN** the output file lives under `reviews/` in the system temporary
  directory

### Requirement: Prune kept review outputs older than seven days at pane start

Before generating the output file name, the pane process SHALL prune
top-level `*.out` files in `<state_dir>/reviews` whose modification time is
at least `REVIEW_MAX_AGE` (7 days) old, for all tools alike. Pruning is
best-effort: an unreadable directory, entries without a `.out` extension,
and files whose mtime cannot be read or lies in the future SHALL be
skipped silently, and a failed removal SHALL log `herdr-review: warning:
cannot remove <path>: <error>` without aborting the review.

#### Scenario: Old output removed, fresh output kept

- **WHEN** `reviews/` contains an 8-day-old `old.out`, a current
  `fresh.out`, and a non-`.out` file, and a pane starts
- **THEN** `old.out` is removed, `fresh.out` and the non-`.out` file remain

#### Scenario: Pruning never blocks the review

- **WHEN** removal of an old `.out` file fails (e.g. permissions)
- **THEN** a warning is logged and the pane continues to spawn the tool

### Requirement: Each tool spawns as a direct child with its own stdio strategy

The pane process SHALL spawn the resolved binary with
`.current_dir(<review dir>)` and stay the child's direct parent. revdiff
and tuicr inherit the pane tty for stdout and stderr; lumen's stdout is
piped and captured in memory while its stderr inherits the pane tty,
because lumen routes its TUI to `/dev/tty` itself when stdout is not a
terminal. Scope argv fragments come from review-scope-selection; revdiff
adds `--output <out> --exit-code-on-annotations` with scope positionals
last.

#### Scenario: revdiff argv keeps the fixed trailing flags

- **WHEN** the revdiff child is spawned for any scope
- **THEN** its argv is `<bin> [scope flags…] --output <out>
--exit-code-on-annotations [scope positionals…]` with positionals last,
  and stdout/stderr inherit the pane tty

#### Scenario: tuicr runs on the inherited tty

- **WHEN** the tuicr child is spawned
- **THEN** its argv is `<bin>` plus the scope fragment (`-w` or
  `-r <base>..HEAD`) and stdout/stderr inherit the pane tty

#### Scenario: lumen stdout is piped while stderr is inherited

- **WHEN** the lumen child is spawned
- **THEN** its stdout is a pipe the pane process reads to completion and
  its stderr is the pane tty

#### Scenario: Spawn failure

- **WHEN** spawning the resolved binary fails (e.g. it was removed between
  resolution and spawn)
- **THEN** the pane reports `failed to run <bin> in <dir>: <error>`

#### Scenario: Killed child keeps the output

- **WHEN** the tool child was killed and reports no exit code
- **THEN** the run fails with `<tool> terminated without an exit code
(killed?); output kept at <path>` and the output file is kept

### Requirement: revdiff keeps its file-based annotation channel

For revdiff the pane process SHALL take the exit code directly from the
child status and read the `.out` file lossily (`String::from_utf8_lossy`).
A missing or unreadable file with exit code 0 SHALL log `herdr-review:
warning: cannot read <path>: <error>` and count as empty output; with any
other exit code it SHALL be a hard error `cannot read revdiff output
<path>: <error>` and the file SHALL be kept.

#### Scenario: Output file unreadable after a clean exit

- **WHEN** revdiff exits 0 but its output file cannot be read
- **THEN** a `herdr-review: warning: cannot read <path>: <error>` line is
  logged and empty output is used, not an error

#### Scenario: Missing output at an annotations exit code

- **WHEN** revdiff exits 10 but the output file is missing
- **THEN** the run fails with `cannot read revdiff output <path>: <error>`
  and the pane is held

#### Scenario: Invalid UTF-8 in the output file

- **WHEN** the output file contains bytes that are not valid UTF-8
- **THEN** they are replaced with U+FFFD and the read still succeeds

### Requirement: tuicr annotations are collected after exit through the session adapter

The pane process SHALL record a launch timestamp before spawning tuicr. On
tuicr exit 0 it SHALL predict the session slug
`[<owner>/]<repo>@<sanitized-branch>/worktree/<7-char-head>` from read-only
git probes, call `tuicr review comments --session <slug>` (JSON is its
only output format), and —
when that fails or names no session — fall back to `tuicr review list
--repo .`, select the newest session updated after the launch timestamp,
and fetch its comments with the same `review comments` call.

#### Scenario: Predicted slug answers directly

- **WHEN** tuicr exits 0 and `review comments --session <predicted slug>`
  returns a JSON comment list
- **THEN** no `review list` call is made; the slug is predicted from the
  `origin` remote URL for `[<owner>/]<repo>` (segment omitted when no
  remote exists), `git branch --show-current` for the branch, and
  `git rev-parse --short=7 HEAD` for the head

#### Scenario: Fallback list picks the session created by this run

- **WHEN** the predicted-slug call fails and `review list --repo .` returns
  sessions of which exactly one was updated after the launch timestamp
- **THEN** the pane fetches comments for that session's slug

#### Scenario: No comments is a clean no-annotations

- **WHEN** the adapter yields an empty comment list with tuicr exit code 0
- **THEN** the outcome is no-annotations and the `.out` file is removed

#### Scenario: Adapter failure keeps evidence

- **WHEN** both the predicted-slug call and the list fallback fail (spawn
  error or invalid JSON)
- **THEN** the run fails with `tuicr session lookup failed: <detail>;
output kept at <path>` and the pane is held

#### Scenario: Nonzero tuicr exit skips the adapter

- **WHEN** tuicr exits with a nonzero code
- **THEN** no adapter command runs, the outcome is `Failed(rc)`, and the
  (empty) output file is kept

#### Scenario: Tuicr canonical text write failure is fatal

- **WHEN** writing the converted comments to the `.out` file fails
- **THEN** the run fails with `cannot write review output <path>: <error>`
  and the pane is held

### Requirement: lumen annotations are the captured stdout export, converted after exit

Lumen writes its formatted annotation export to stdout only via its
`s`→Enter action. On exit 0 the pane process SHALL strip terminal-probe
escape sequences (CSI/OSC) from the captured stdout, convert the
remainder to the canonical format, and write it to the `.out` file; a
stdout whose stripped remainder is blank is the clean no-annotations
case — raw bytes alone never are, since lumen probes the terminal at
startup even without exporting (live-probed); a nonzero exit keeps the
raw capture.

#### Scenario: Export on stdout delivers

- **WHEN** lumen exits 0 and its captured stdout holds one or more parsed
  annotation blocks after escape-sequence stripping
- **THEN** the canonical conversion is written to the `.out` file and the
  run proceeds to delivery decisions

#### Scenario: Terminal-probe preamble is stripped before parsing

- **WHEN** the captured stdout begins with terminal-probe escape
  sequences (as observed live: `ESC]11;?BEL` and `ESC[5n`) followed by
  annotation blocks
- **THEN** the sequences are discarded and the blocks parse normally

#### Scenario: TUI closed without exporting

- **WHEN** lumen exits 0 with captured stdout that is blank or, after
  escape stripping, holds no annotation content (the live-observed probe
  preamble alone)
- **THEN** the outcome is no-annotations, nothing is delivered to the
  agent, and the `.out` file is removed

#### Scenario: Unparseable export keeps raw

- **WHEN** lumen exits 0 with non-blank stdout that converts to zero
  canonical blocks
- **THEN** the raw capture is written to the file and kept, and the run
  fails with `cannot parse lumen annotation export; raw output kept at
<path>`

#### Scenario: Nonzero exit keeps raw capture

- **WHEN** lumen exits nonzero
- **THEN** whatever was captured is written to the `.out` file
  best-effort, the file is kept, and the outcome is `Failed(rc)`

#### Scenario: Lumen canonical text write failure is fatal

- **WHEN** writing the converted export to the `.out` file fails after a
  clean parse
- **THEN** the run fails with `cannot write review output <path>: <error>`
  and the pane is held
