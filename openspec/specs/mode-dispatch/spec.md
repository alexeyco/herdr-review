# mode-dispatch

## Purpose

The `herdr-revdiff` binary is a single executable serving two herdr plugin
entry points: an action and a pane. At startup it inspects the plugin
environment herdr injects (`HERDR_PLUGIN_ENTRYPOINT_ID`,
`HERDR_PLUGIN_ACTION_ID`) to decide which role to play, and treats a run
outside any herdr context as a hard error. The manifest `herdr-plugin.toml`
declares both entry points, each with the same array-valued command pointing
at the same binary. Around this dispatch the binary defines a
uniform top-level error policy: errors print with a `herdr-revdiff:` prefix
on stderr and exit 1, with pane-mode errors additionally blocking on stdin
so an auto-closing pane stays readable; the binary itself only ever exits
with 0 or 1.

## Requirements

### Requirement: Mode is dispatched purely from the plugin environment

The binary SHALL choose its run mode at startup from environment variables
only, checking `HERDR_PLUGIN_ENTRYPOINT_ID` first: when it is set, its
trimmed value selects the pane role — `review` runs the review pane mode,
`setup` runs the read-only setup-dialog popup mode, and any other
non-blank value fails with `unknown plugin entrypoint <value>; expected
review or setup`.
When the entry-point variable is unset or blank and
`HERDR_PLUGIN_ACTION_ID` is set, the binary runs action mode. When neither
is set it fails as "invoked outside herdr plugin context". Because the
entry-point variable is checked first, pane modes win when both are set,
matching herdr's action-spawns-pane nesting. A variable whose value is
empty or whitespace-only counts as unset, via trimming, so herdr passing
empty strings never silently selects a mode.

#### Scenario: Only the entry-point variable is set

- **WHEN** `HERDR_PLUGIN_ENTRYPOINT_ID` is set to `review` or `setup` and
  `HERDR_PLUGIN_ACTION_ID` is not set
- **THEN** the binary runs that entrypoint's pane mode

#### Scenario: Unknown entrypoint is an error

- **WHEN** `HERDR_PLUGIN_ENTRYPOINT_ID` is `editor`
- **THEN** the binary fails with `unknown plugin entrypoint editor;
expected review or setup`

#### Scenario: Both mode variables are set

- **WHEN** `HERDR_PLUGIN_ENTRYPOINT_ID` and `HERDR_PLUGIN_ACTION_ID` are
  both set
- **THEN** the binary runs the entrypoint's pane mode, not action mode

#### Scenario: Neither mode variable is set

- **WHEN** the binary is invoked with neither `HERDR_PLUGIN_ENTRYPOINT_ID`
  nor `HERDR_PLUGIN_ACTION_ID` set
- **THEN** it writes `herdr-review: invoked outside herdr plugin context`
  to stderr and exits 1

#### Scenario: Whitespace-only entry point with a set action

- **WHEN** `HERDR_PLUGIN_ENTRYPOINT_ID` is set to whitespace and
  `HERDR_PLUGIN_ACTION_ID` is set to a non-blank value
- **THEN** the binary runs action mode

#### Scenario: Both variables set to blank values

- **WHEN** both mode variables are present but blank or whitespace-only
- **THEN** the binary reports the outside-herdr error and exits 1

### Requirement: Every top-level error prints on stderr and exits 1; pane-mode errors hold the pane

When any stage of `run()` returns an error, the binary SHALL print
`herdr-review: <error message>` to stderr and exit 1. Before exiting, if
`HERDR_PLUGIN_ENTRYPOINT_ID` is set (either pane mode, review or setup),
it SHALL call `hold_pane()`, which prints `herdr-review: press Enter to
close` and blocks reading one line from stdin, so the auto-closing plugin
pane stays readable; a closed stdin (EOF) returns immediately. Action-mode
errors exit without holding.

#### Scenario: Pane-mode error is held for reading

- **WHEN** review or setup pane mode fails at any stage
- **THEN** stderr shows the `herdr-review:` error and the prompt
  `herdr-review: press Enter to close`, the process waits for Enter, then
  exits 1

#### Scenario: Error with closed stdin does not hang

- **WHEN** a pane mode errors and stdin is already at EOF
- **THEN** `hold_pane()` returns at once and the process exits 1

#### Scenario: Action-mode error exits immediately

- **WHEN** action mode fails
- **THEN** the error is printed on stderr and the process exits 1 without
  blocking on stdin

### Requirement: The binary exits with 0 or 1 only

Successful paths SHALL exit 0 by returning normally (no explicit exit
call); all error paths exit 1. Exit codes from review-tool children —
including revdiff's annotations code 10 — are internal outcomes consumed by
pane mode and SHALL NOT become the binary's exit code. Closing the setup
dialog (`Enter`/`q`/Esc) is a success path and exits 0.

#### Scenario: Success in either mode

- **WHEN** any mode — action, setup dialog (including one closed by any
  key), or review pane — completes without error
- **THEN** the process exits 0

#### Scenario: revdiff child failure does not leak its code

- **WHEN** a review tool child (for example revdiff exiting 10, or tuicr
  exiting 1) terminates with any code
- **THEN** that code is mapped to an internal outcome, not propagated as
  the binary's exit status

### Requirement: Output convention distinguishes errors and best-effort warnings

Top-level errors SHALL use the prefix `herdr-review: <msg>` on stderr and
best-effort warnings SHALL use `herdr-review: warning: <msg>` on stderr
(for example unreadable review output at exit 0, prune or removal
failures, unparsable review-pane open replies, and config degradations
including an invalid `tool` key). Pane progress lines follow the same prefix; the
stdout progress lines pane mode prints (`herdr-review: reviewing
<description>`, `herdr-review: nothing to review`) are owned by the
review-scope-selection capability, not restated here.

#### Scenario: Best-effort cleanup failure warns rather than fails

- **WHEN** a prune or similar best-effort step cannot complete (for example,
  removing an old review output)
- **THEN** a `herdr-review: warning: ...` line is written to stderr and the
  run continues

### Requirement: The manifest declares paired action and pane entry points for one binary

`herdr-plugin.toml` SHALL declare plugin id `alexeyco.review` — dots are
legal in herdr ids, precedent `ray.plugin-manager` — with the display
`name = "Review"`, `min_herdr_version = "0.9.0"`, and `platforms =
["linux", "macos"]`, one action with id `open` (`contexts =
["workspace"]`), so the keybinding/invoke command is
`alexeyco.review.open`, and two panes whose entrypoint ids are unchanged:
id `review` with `placement = "tab"` and id `setup` with
`placement = "popup"`, title "Review tool setup", and the popup sizing
keys `width = "60%"` and `height = 15`. All three entries SHALL declare
the identical argv-form command `["./target/release/herdr-review"]` — a
JSON/TOML array, not a shell string. The manifest `version` SHALL stay equal to the
`Cargo.toml` version, enforced by the CI version-sync job and the
manifest unit test; the version value itself is the maintainer's call
(this change releases 0.2.0 per task 12).
Evidence for popup panes being declarable: herdr-open-in
herdr-plugin.toml:12-17 ships `placement = "popup"` for its picker pane.
UNVERIFIED: that herdr executes array commands as argv without a shell
layer is attested only in AGENTS.md and herdr's own documentation, not by
any code in this repository.

#### Scenario: Both entry points resolve to the same binary

- **WHEN** the manifest's action and pane entries are compared
- **THEN** all of them declare the identical command array
  `["./target/release/herdr-review"]`, and the binary's environment
  dispatch selects which role the launched process plays

#### Scenario: Manifest and Cargo versions diverge

- **WHEN** `version` in `herdr-plugin.toml` differs from the version in
  `Cargo.toml`
- **THEN** the CI version-sync check fails the build
