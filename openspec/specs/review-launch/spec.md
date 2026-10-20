# Review Launch

## Purpose

The review-launch capability is the action mode of the herdr-revdiff binary:
when herdr invokes the plugin's `review` action (signaled by
`HERDR_PLUGIN_ACTION_ID`), the process reads the herdr plugin context from
`HERDR_PLUGIN_CONTEXT_JSON`, resolves the repository directory to review, and
opens a plugin-managed review pane through the herdr CLI (`plugin pane open
--plugin revdiff --entrypoint review --placement tab`). It passes the resolved
directory to the pane both as `--cwd` and as an injected
`HERDR_REVIEWS_DIR` environment variable, forwards optional workspace and
caller identity, reports success or failure on stderr, and exits. The action
does not run revdiff itself — that happens in the pane (a separate
capability).

## Requirements

### Requirement: Action mode reads and validates the herdr plugin context

The action SHALL obtain the plugin context from the environment variable
`HERDR_PLUGIN_CONTEXT_JSON`, and SHALL fail with a clear error when it is
missing, blank, or not valid JSON. (`src/review.rs:103-109`,
`src/context.rs:20-21`)

#### Scenario: Context variable absent or blank

- **WHEN** the action runs and `HERDR_PLUGIN_CONTEXT_JSON` is unset or
  contains only whitespace
- **THEN** the action fails with the message "HERDR_PLUGIN_CONTEXT_JSON is not
  set; invoke this action from herdr" (`src/review.rs:103-108`)

#### Scenario: Context is invalid JSON

- **WHEN** `HERDR_PLUGIN_CONTEXT_JSON` contains JSON that cannot be parsed
- **THEN** the action fails with a message beginning "invalid plugin context
  JSON: " followed by the parser error (`src/context.rs:20-21`)

### Requirement: The review directory is resolved along a fixed fallback chain

The action SHALL pick the first present, non-blank string among
`worktree.checkout_path`, `workspace_cwd`, and `focused_pane_cwd` in the
context JSON; a non-string or whitespace-only value counts as absent; when all
three are absent the action SHALL fail. (`src/context.rs:36-52`,
`src/context.rs:22-24`, `src/context.rs:55-65`)

#### Scenario: Worktree checkout path wins

- **WHEN** the context JSON has a non-blank `worktree.checkout_path`
- **THEN** the action uses that value as the review directory, regardless of
  `workspace_cwd` or `focused_pane_cwd` (`src/context.rs:37-40`)

#### Scenario: Fallback chain and blank handling

- **WHEN** `worktree.checkout_path` is absent, non-string, or whitespace-only,
  and a later candidate in the chain (`workspace_cwd`, then
  `focused_pane_cwd`) is a non-blank string
- **THEN** the action uses the first such later candidate
  (`src/context.rs:41-50`, `src/context.rs:62-65`)

#### Scenario: No directory anywhere in the context

- **WHEN** none of the three candidates yields a non-blank string
- **THEN** the action fails with "no directory in plugin context; invoke this
  action inside a workspace" (`src/context.rs:22-24`)

### Requirement: Caller identity fields are read from context with defined env fallbacks

The action SHALL read `workspace_id`, `focused_pane_id`, and
`focused_pane_agent` from the context JSON (blank or non-string counts as
absent); `workspace_id` and `focused_pane_id` SHALL fall back to the
environment variables `HERDR_WORKSPACE_ID` and `HERDR_PANE_ID` respectively,
and `focused_pane_agent` SHALL have no environment fallback.
(`src/context.rs:25-30`, `src/review.rs:111-117`) The resolved values feed
the action's own review-pane open directly — `--workspace <id>` for `tab`
placement, `--target-pane <pane>` for `split`/`zoomed`, and the
`HERDR_REVIEWS_CALLER_PANE` / `HERDR_REVIEWS_CALLER_AGENT` env pairs — with
no popup forwarding hop; the action-time context is the only trusted source
for caller identity.

#### Scenario: Pane id taken from the environment

- **WHEN** the context JSON has no non-blank `focused_pane_id` and
  `HERDR_PANE_ID` is set to a non-blank value
- **THEN** the action uses the `HERDR_PANE_ID` value as the caller pane
  identifier for the `--target-pane` flag and the
  `--env HERDR_REVIEWS_CALLER_PANE=<value>` pair of the review-pane open

#### Scenario: Agent without context entry is simply absent

- **WHEN** the context JSON has no non-blank `focused_pane_agent`
- **THEN** the action treats the caller agent as absent even if any
  environment variable names an agent, and omits the corresponding `--env`
  argument from the review-pane open

### Requirement: The review pane is opened through the herdr CLI with a fixed, ordered argv

The review pane SHALL be opened by the action itself after selecting an
installed catalog tool, invoking the herdr binary at `HERDR_BIN_PATH` with the
argument vector `plugin pane open --plugin alexeyco.review --entrypoint
review --placement <placement>`, then a conditional `--direction <direction>` (only
when placement is `split` and direction resolves to `right` or `down`),
then a conditional `--target-pane <pane>` (only for `split` and `zoomed`,
from the resolved caller pane; without one those placements degrade to
`tab` with a warning before the open, because the daemon rejects them
targetless), then a conditional `--workspace <id>` (only when placement is
`tab`; overlay takes no targeting flags — live-probed `invalid_params` on
targeted non-tab opens), then `--cwd <dir>`, then always
`--env HERDR_REVIEWS_DIR=<dir>` and `--env HERDR_REVIEW_TOOL=<tool>`, then
conditional `--env HERDR_REVIEWS_CALLER_PANE=<pane>`,
`--env HERDR_REVIEWS_CALLER_AGENT=<agent>`, and
`--env HERDR_REVIEWS_CALLER_TAB=<tab>` (each only when that value is
present), then `--focus`. `<placement>` and `<direction>` come from the
placement-config capability; `<tool>` is the selected installed catalog id
(`lumen`, `revdiff`, or `tuicr`).

#### Scenario: Resolved directory is passed twice

- **WHEN** the action opens the review pane for the resolved directory
- **THEN** the argv contains both `--cwd <dir>` and
  `--env HERDR_REVIEWS_DIR=<dir>`, because runtime commands may default the
  pane's cwd to the plugin directory

#### Scenario: The tool env pair is always present

- **WHEN** the review pane is opened for the selected installed tool
- **THEN** the argv contains `--env HERDR_REVIEW_TOOL=<tool>` with the
  catalog id selected from the installed tools on `PATH`

#### Scenario: Direction is passed only for split placement

- **WHEN** the configured placement is `split` with direction `right` or
  `down`
- **THEN** the argv contains `--direction <direction>` immediately after the
  `--placement split` pair; for any other placement no `--direction` flag is
  passed

#### Scenario: Split and zoomed pass the caller pane as target

- **WHEN** the resolved placement is `split` or `zoomed` and a caller pane
  was resolved from the context
- **THEN** the argv contains `--target-pane <caller-pane>` after the
  placement-and-direction pairs and before `--cwd`; `tab` and `overlay`
  pass no `--target-pane`

#### Scenario: Split or zoomed without a caller pane falls back to tab

- **WHEN** the resolved placement is `split` or `zoomed` but no caller pane
  value is present
- **THEN** the action warns
  `herdr-review: warning: placement "<p>" needs a caller pane; using "tab"`
  and opens the review pane with `--placement tab` and no targeting flags

#### Scenario: The workspace flag is passed only for tab placement

- **WHEN** the review pane is opened with any placement and a workspace id
  was resolved
- **THEN** `--workspace <id>` appears only for `tab` placement; `overlay`,
  `split`, and `zoomed` opens carry no `--workspace`

#### Scenario: The pane-open argv always carries its own placement

- **WHEN** the action builds the `plugin pane open` argv
- **THEN** it always includes `--placement <placement>` as the argument it
  actually passes, resolved by the placement-config capability, while the
  manifest's `placement = "tab"` for the `review` pane is a separate
  declaration that can drift from the argv

#### Scenario: HERDR_BIN_PATH is missing or blank

- **WHEN** any herdr CLI call is attempted and `HERDR_BIN_PATH` is unset or
  whitespace-only
- **THEN** the call fails with "herdr did not provide HERDR_BIN_PATH (plugin
  API v1 contract); cannot call back into herdr" (`src/herdr.rs:28-36`)

### Requirement: Success is reported on stderr and the action exits without holding

On a successful direct open the action SHALL log `herdr-review: opened
review pane {pane_id} in tab {tab}` followed by a best-effort
`herdr tab focus <tab>` whose errors are ignored when a `tab_id` parsed, or
`herdr-review: opened review pane {pane_id}` with no focus call when it is
absent; an unparsable reply already warned and skips the log line. On a
successful setup-popup open the action SHALL log `herdr-review: opened
review tool setup` with no focus call. Either way the action exits 0
without holding; failures follow the mode-dispatch capability's top-level
error policy (printed with the `herdr-review:` prefix, exit 1, no hold in
action mode).

#### Scenario: Action success opens the popup and exits

- **WHEN** the setup-popup open exits 0
- **THEN** stderr shows `herdr-review: opened review tool setup` and the
  action exits 0 without any focus call or hold

#### Scenario: Pane opens with a tab

- **WHEN** the action's direct review-pane open returns a parsed pane id
  and tab id
- **THEN** stderr shows `herdr-review: opened review pane {pane_id} in tab
{tab}` and a `tab focus {tab}` call runs best-effort against a rendering
  race; its failure does not affect the exit code, because herdr already
  applied `--focus` at open

#### Scenario: A CLI call exits nonzero

- **WHEN** a herdr CLI invocation returns a nonzero status
- **THEN** the error is "{bin} {args} failed ({status}): {stderr snippet}"
  (`src/herdr.rs:16-25`), which propagates as a failure under the
  mode-dispatch capability's error policy

#### Scenario: The herdr binary cannot be spawned

- **WHEN** spawning the binary at `HERDR_BIN_PATH` fails
- **THEN** the run fails with "failed to run {bin} {args}: {io error}" and
  exits 1 (`src/herdr.rs:10-13`)

### Requirement: The action chooses a review tool from the installed catalog

The action SHALL inspect supported tool executables on `PATH`. It SHALL open the review pane directly with the sole installed tool, open the picker popup when multiple tools are installed, and open no review pane before the picker selection.

#### Scenario: No catalog tools are installed

- **WHEN** no supported tool executable is found on `PATH`
- **THEN** the action routes to the setup popup and does not open a review pane

#### Scenario: Exactly one catalog tool is installed

- **WHEN** exactly one supported tool executable is found on `PATH`
- **THEN** the action opens the review pane directly with that tool id and opens no picker

#### Scenario: Multiple catalog tools are installed

- **WHEN** multiple supported tool executables are found on `PATH`
- **THEN** the action opens the picker popup and waits for selection before opening a review pane

### Requirement: The action opens the setup popup when no supported tool is installed

When no supported review tool is installed, the action SHALL open the existing setup popup using `HERDR_BIN_PATH` with `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`, without `--workspace`, `--cwd`, or `--env` arguments. The existing no-tool guidance is retained, and success is determined by the herdr exit status.

#### Scenario: No supported tool is installed

- **WHEN** the PATH check finds none of the supported catalog tools
- **THEN** the setup popup opens with the existing no-tool guidance and the action does not open a review pane

#### Scenario: Setup-open argv carries no forwarding

- **WHEN** the action opens the setup popup
- **THEN** the argv is exactly `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`

#### Scenario: Popup open succeeds without a pane id in the reply

- **WHEN** the herdr CLI exits 0 for the setup popup open and the reply JSON carries no pane id
- **THEN** the action succeeds and does not treat the missing pane id as an error

### Requirement: The action forwards the caller tab id to the review pane

On the direct open the action SHALL include
`--env HERDR_REVIEWS_CALLER_TAB=<tab>` with the caller tab id it resolved
from the plugin context, alongside the other conditional caller pairs, so
the review pane can return focus after a clean ending (the
annotation-delivery capability owns the focus call itself). When no tab id
resolves, the pair is omitted and the open proceeds. The setup-popup open
SHALL never carry the pair — it carries no `--env` at all.

#### Scenario: Caller tab pair is present on the direct open

- **WHEN** the action resolved a caller tab id from the plugin context and
  opens the review pane directly
- **THEN** the argv contains `--env HERDR_REVIEWS_CALLER_TAB=<tab>` with
  the resolved id, after the caller pane and agent pairs and before
  `--focus`

#### Scenario: Caller tab pair is omitted when no tab resolves

- **WHEN** no caller tab id resolves from the context
- **THEN** the pair is absent from the argv and the review-pane open
  proceeds unchanged

#### Scenario: Setup popup never carries the caller tab

- **WHEN** the action opens the setup popup
- **THEN** the argv contains no `--env` pairs at all, so
  `HERDR_REVIEWS_CALLER_TAB` never appears on that open

### Requirement: The review-pane open reply is parsed best-effort

For the direct review-pane open, the action SHALL parse the reply
best-effort: a non-blank `result.plugin_pane.pane.pane_id` enables the
success log line, and `tab_id` is optional with a blank value treated as
absent. A reply that is not valid JSON or lacks a usable pane id produces a
`herdr-review: warning:` line — not an error, because exit status 0 already
means the pane exists. The setup-popup reply body is not parsed at all.

#### Scenario: Reply without a usable pane id warns but succeeds

- **WHEN** the review-pane open exits 0 but the reply is not valid JSON or
  lacks a non-blank `result.plugin_pane.pane.pane_id`
- **THEN** the action logs a warning quoting a raw reply snippet truncated
  to 200 characters, still treats the launch as successful, and exits 0

#### Scenario: Overlay-style reply without a tab

- **WHEN** the reply carries a valid `pane_id` and an absent or blank
  `tab_id`
- **THEN** the action proceeds with tab_id treated as absent and makes no
  tab-focus call

#### Scenario: Popup reply body is ignored

- **WHEN** the action's setup-popup open exits 0 with any reply body
- **THEN** the body is not parsed and cannot cause a failure
