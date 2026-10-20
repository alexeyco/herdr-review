# Annotation Delivery

## Purpose

This capability covers what the plugin does after the revdiff child process
finishes inside a review pane: it decides the review outcome from revdiff's
exit code and output (annotations exist when the exit code is 10, or the exit
code is 0 with non-blank output; any other exit code is a failure), builds and
caps the prompt text, and delivers annotations to the caller agent through the
herdr CLI when one is available. It also owns the file-retention rules for the
review output file (kept on failures and no-agent paths, removed on clean
delivery and the no-annotations case), and pane teardown via best-effort
close-on-success with a tab-close fallback. Error and no-agent paths keep the
pane open by holding on stdin until the user presses Enter; the hold
mechanics themselves belong to the mode-dispatch capability.

## Requirements

### Requirement: Review outcome is decided from exit code, output, and agent availability

The pane process SHALL classify the tool's result into exactly one outcome,
per tool. Annotations exist when: revdiff exits 10, or exits 0 with
non-blank `.out` content; tuicr exits 0 and the session adapter yields a
non-empty comment list; lumen exits 0 with non-blank captured stdout. When
annotations exist the outcome is `Deliver` if a caller agent is available
and `NoAgent` otherwise. A clean exit with nothing collected is
`NoAnnotations`. Any other exit code is `Failed(rc)` even when output was
collected. A caller agent is considered available only when both
`HERDR_REVIEWS_CALLER_PANE` and `HERDR_REVIEWS_CALLER_AGENT` are non-blank
in the pane environment (those variables are injected by the action on
the review-pane open).

#### Scenario: Nonzero success code with agent delivers

- **WHEN** revdiff exits with code 10 and both caller env vars are set
- **THEN** the outcome is `Deliver`

#### Scenario: tuicr with comments delivers

- **WHEN** tuicr exits 0, both caller env vars are set, and the adapter
  returns at least one comment
- **THEN** the outcome is `Deliver`

#### Scenario: lumen export delivers

- **WHEN** lumen exits 0, both caller env vars are set, and the captured
  stdout is non-blank and parseable
- **THEN** the outcome is `Deliver`

#### Scenario: Clean exit with blank output is no-annotations

- **WHEN** the tool exits 0 and its collection channel is empty — blank
  revdiff output, an empty tuicr comment list, or lumen stdout that is
  blank or holds only the live-observed terminal-probe preamble after
  escape stripping, because the user closed the TUI without exporting
- **THEN** the outcome is `NoAnnotations`

#### Scenario: Annotations without an agent

- **WHEN** annotations exist for any tool and either caller env var is
  missing or blank
- **THEN** the outcome is `NoAgent`

#### Scenario: Any other exit code fails regardless of output

- **WHEN** the tool exits with a code outside its success set (revdiff:
  neither 0 nor 10; tuicr or lumen: nonzero), even with collected output
- **THEN** the outcome is `Failed(rc)`

### Requirement: The delivered prompt is a header plus trimmed output, capped at 64 KiB on a UTF-8 boundary

The prompt sent to the agent SHALL be the header `Review annotations from
<tool>:\n\n` — where `<tool>` is the selected catalog id `revdiff`,
`tuicr`, or `lumen` — followed by the canonical annotation text with
leading and trailing whitespace trimmed. The prompt SHALL be capped at
`MAX_PROMPT_BYTES = 65536` bytes, cutting on a UTF-8 character boundary and
appending `\n\n(output truncated; full annotations kept at <path>)` only
when the text exceeds the cap; text that already fits is returned unchanged
and multi-byte characters are never split.

#### Scenario: Header names the tool that ran

- **WHEN** the selected tool is tuicr and annotations are delivered
- **THEN** the prompt starts with `Review annotations from tuicr:` followed
  by a blank line

#### Scenario: Prompt within the cap is sent intact

- **WHEN** the header plus trimmed canonical text is at most 65536 bytes
- **THEN** the delivered text equals it unchanged, with no truncation note

#### Scenario: Oversized prompt is cut at a character boundary

- **WHEN** the header plus trimmed canonical text exceeds 65536 bytes
- **THEN** the text is cut at the largest boundary at or below 65536 bytes
  and the note pointing at the kept output file is appended

### Requirement: Annotations are delivered to the caller pane through the herdr CLI with defined success and failure handling

Delivery SHALL run `herdr agent prompt <caller_pane> <text>` invoked via
`HERDR_BIN_PATH` (src/herdr.rs:147-149), identically for all three tools.
On delivery success the process SHALL, in order, print the full line
`herdr-review: sent review annotations to agent pane <target>` to stderr,
remove the output file (tolerating `NotFound`, logging other removal
failures as warnings), perform a best-effort pane close, and exit 0. On
delivery failure the process SHALL report the hard error `failed to send
annotations to agent pane <target>: <e>; output kept at <path>`, keep the
output file, print the error, hold the pane, and exit 1.

#### Scenario: Successful delivery tears the pane down and removes the file

- **WHEN** the herdr agent prompt command succeeds
- **THEN** the `herdr-review: sent review annotations to agent pane
<target>` line is printed to stderr, the output file is removed, a
  best-effort pane close is attempted, and the process exits 0

#### Scenario: Failed delivery keeps the file and holds the pane

- **WHEN** the herdr agent prompt command fails
- **THEN** the error message names the target, the error, and the kept
  output path; the file is left in place; the pane is held; the process
  exits 1

### Requirement: Non-delivery outcomes follow fixed message and retention rules

On `NoAnnotations` the process SHALL print `herdr-review: no annotations`
to stderr, remove the output file, attempt a best-effort pane close, and
finish successfully. On `NoAgent` it SHALL print `herdr-review: no agent in
focused pane; annotations kept at <path>` to stderr, keep the output file,
and hold the pane. On `Failed(rc)` it SHALL report the error `<tool> failed
with exit code <rc>; output kept at <path>` — naming the selected tool —
keep the file, and the pane-mode error path in main SHALL print the error,
hold the pane, and exit 1. The output file is therefore retained on
`Failed`, `NoAgent`, delivery failure, a revdiff read failure with a
nonzero exit code, a tuicr adapter failure, a `.out` write failure, a lumen
parse failure, and a killed child, and removed only on delivery success and
`NoAnnotations`. The rules are identical for all three tools.

#### Scenario: No annotations cleans up silently

- **WHEN** the outcome is `NoAnnotations` for any tool
- **THEN** `herdr-review: no annotations` is printed to stderr, the output
  file is removed, and the pane close is attempted

#### Scenario: Unreadable output at a clean exit counts as no annotations

- **WHEN** revdiff exits 0 but its output file cannot be read
- **THEN** a `herdr-review: warning: cannot read <path>: <err>` line is
  written to stderr and the run proceeds as `NoAnnotations`

#### Scenario: Failed run surfaces the code and keeps evidence

- **WHEN** the outcome is `Failed(2)` with lumen selected
- **THEN** the error is `lumen failed with exit code 2; output kept at
<path>`, the file stays on disk, and the pane is held before exiting 1

### Requirement: Attention outcomes hold the pane, and pane close is best-effort with a tab-close fallback

The `NoAgent` outcome SHALL call `hold_pane()` after printing its message,
and delivery failures, `Failed(rc)` runs, adapter, parse and write
failures, and the nothing-to-review scope (owned by
review-scope-selection) reach the same hold through the pane-mode error
path or their own hold call; `hold_pane`'s mechanics — prompt line
`herdr-review: press Enter to close`, stdin block, EOF behavior — belong to
the mode-dispatch capability and are not restated here. `close_pane` SHALL
first try `herdr plugin pane close <HERDR_PANE_ID>`, skipping that attempt
when the variable is unset; on failure it SHALL log the full line
`herdr-review: failed to close pane <pane>: <err>` and fall through to
`herdr tab close <HERDR_TAB_ID>`; all close failures are logged and never
propagated. Success-path closes are best-effort because plugin panes are
expected to auto-close when the pane process exits.
UNVERIFIED: this auto-close mechanism (herdr-side
`RuntimeExitAction::ClosePane`) is attested only in AGENTS.md:15; the
plugin manifest declares no such key, and the herdr-side behavior was not
verified from herdr sources.

#### Scenario: Pane close falls back to tab close

- **WHEN** `herdr plugin pane close` fails for a pane with `HERDR_PANE_ID`
  set
- **THEN** the failure is logged and `herdr tab close <HERDR_TAB_ID>` is
  attempted; if that also fails, the error is logged only

### Requirement: An internally inconsistent Deliver outcome is rejected as an error

A `Deliver` outcome that reaches execution without a caller pane SHALL produce
the internal error
`internal: deliver outcome without a caller pane`; this state is unreachable
given `decide`'s rules, so it guards against future inconsistency
(src/review.rs:225-230).

#### Scenario: Deliver without a caller pane

- **WHEN** the outcome is `Deliver` but the caller pane value is absent at
  execution time
- **THEN** the internal error is reported instead of attempting delivery

### Requirement: Tool-native annotations are converted to the canonical block format

Annotations SHALL reach the prompt in a canonical block format: each block
is a header line `## <path>:<line> (<side>)` — side `+` or `-` — followed by
comment body lines, with blocks separated by blank lines. revdiff output is
already canonical and SHALL pass through unconverted; tuicr and lumen
exports SHALL be converted as the scenarios pin.

#### Scenario: tuicr comment converts with side mapping

- **WHEN** a tuicr JSON comment has path `src/a.rs`, start_line 12,
  end_line 15, side LEFT, and content `move this`
- **THEN** the canonical block is `## src/a.rs:12 (-)` followed by
  `move this`; side maps RIGHT→`+` and LEFT→`-`, and `end_line` never
  appears in the header — the block anchors on the start line

#### Scenario: tuicr comment without side defaults to plus

- **WHEN** a tuicr comment carries no side field or an unrecognized one
- **THEN** its header side marker is `(+)`

#### Scenario: lumen RIGHT line converts

- **WHEN** the lumen export holds `**src/a.rs** line 7 (RIGHT)` followed by
  `rename this` and a `---` separator
- **THEN** the canonical block is `## src/a.rs:7 (+)` followed by
  `rename this`; a `(LEFT)` marker maps to `(-)` the same way

#### Scenario: lumen line range anchors at the first line

- **WHEN** the export holds `**src/a.rs** lines 4-9 (LEFT)`
- **THEN** the canonical header is `## src/a.rs:4 (-)`

#### Scenario: lumen file-level annotation converts to line 0

- **WHEN** the export holds a file-level `**src/a.rs**` block with no line
  information
- **THEN** the canonical header is `## src/a.rs:0 (+)`

#### Scenario: lumen ref header is dropped

- **WHEN** the export starts with a `# origin/main` line
- **THEN** that line contributes no canonical block and no body text, and
  unrecognized lines before the first block are dropped

#### Scenario: revdiff output passes through unchanged

- **WHEN** the tool is revdiff
- **THEN** its `.out` content is used verbatim as the canonical text with
  no parsing

### Requirement: A clean review-pane ending returns focus to the caller tab

On the two clean outcomes — a successful delivery and the no-annotations
cleanup — the pane process SHALL best-effort run `herdr tab focus <tab>`
with the non-blank `HERDR_REVIEWS_CALLER_TAB` value BEFORE the pane
close: closing its own pane kills the process, so a post-close call
never runs (live-probed). Hold and error paths (`NoAgent`, `Failed(rc)`,
delivery failure) SHALL never call it. A failed call prints one
`herdr-review: warning:` line, exit stays 0; missing/blank env skips
silently.

#### Scenario: Delivered run refocuses the caller tab

- **WHEN** delivery succeeded and `HERDR_REVIEWS_CALLER_TAB` is non-blank
- **THEN** the process runs `herdr tab focus <caller tab>` best-effort
  before the `plugin pane close` attempt and exits 0

#### Scenario: No-annotations run refocuses the caller tab

- **WHEN** the outcome is `NoAnnotations` and `HERDR_REVIEWS_CALLER_TAB`
  is non-blank
- **THEN** the same close-then-focus sequence runs and the process exits 0

#### Scenario: Hold and error paths never refocus

- **WHEN** the outcome is `NoAgent`, `Failed(rc)`, or a delivery failure
- **THEN** no `tab focus` call is made and the pane holds as specced

#### Scenario: Focus failure warns without changing the outcome

- **WHEN** the `tab focus` call fails or cannot be spawned after a clean
  ending
- **THEN** exactly one `herdr-review: warning:` line is printed, the exit
  code stays 0, and the output-file handling is already complete

#### Scenario: Missing caller tab is silently skipped

- **WHEN** a clean ending runs with `HERDR_REVIEWS_CALLER_TAB` unset or
  whitespace-only
- **THEN** no focus call is made and no warning is printed
