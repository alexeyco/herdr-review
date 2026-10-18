# How it works

The `alexeyco.review.open` action is a small Rust program (sources under `src/`) that runs in three modes.

- **Action mode** runs when herdr invokes the `alexeyco.review.open` action. herdr sets `HERDR_PLUGIN_ACTION_ID` and `HERDR_PLUGIN_CONTEXT_JSON`; the binary resolves a repository directory and the caller identity from that JSON, reads `config.toml` once, and decides: a configured tool detected on `PATH` opens the review pane directly (`herdr plugin pane open --plugin alexeyco.review --entrypoint review ...`), otherwise the setup popup opens (`herdr plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`). Then it exits immediately.
- **Setup mode** (`HERDR_PLUGIN_ENTRYPOINT_ID=setup`) runs inside that popup, which the action opens only when no usable tool is configured. herdr spawns the manifest command as a process attached to the pane pty. The dialog resolves the config directory for its path line, shows the one universal layout — the config file path, the three catalog rows with their install commands, and the repository URL — and closes on any key. It never opens a review pane.
- **Review-pane mode** (`HERDR_PLUGIN_ENTRYPOINT_ID=review`) runs inside the review pane. It probes git read-only to pick a scope, runs the chosen review tool as a direct child on the pane tty, collects the tool's annotations (revdiff's output file, tuicr's review session, lumen's stdout export), converts them to the canonical block format, and delivers them to the agent pane that requested the review.

Plugin pane commands are argv-exec, not send-text: the manifest `command` array is spawned directly, so there is no shell layer, no echoed command line, and the review tool's TUI appears the instant the pane opens. Plugin panes also auto-close when the pane process exits, so the error and no-agent paths block on Enter before returning.

All three catalog tools — revdiff, tuicr, lumen — are human diff-review TUIs: you author the annotations inside the tool, and the plugin only collects what the tool exported. None of them needs LLM credentials for this flow.

## Dispatch

`main` picks the mode from the environment, in this order:

| Condition                                      | Mode                                                                 |
| ---------------------------------------------- | -------------------------------------------------------------------- |
| `HERDR_PLUGIN_ENTRYPOINT_ID` = `review`        | Review-pane mode                                                     |
| `HERDR_PLUGIN_ENTRYPOINT_ID` = `setup`         | Setup-dialog mode                                                    |
| `HERDR_PLUGIN_ENTRYPOINT_ID` = anything else   | Error: `unknown plugin entrypoint <value>; expected review or setup` |
| entrypoint unset, `HERDR_PLUGIN_ACTION_ID` set | Action mode                                                          |
| neither                                        | Error: `invoked outside herdr plugin context` (exit 1)               |

The entry point is checked first, so pane modes win when both variables are set, matching herdr's action-spawns-pane nesting. For both variables, a blank or whitespace-only value counts as unset.

Errors in either pane mode print `herdr-review: <message>`, print `herdr-review: press Enter to close`, and block on one stdin line before exiting 1, so the auto-closing pane stays readable. Action-mode errors exit immediately. Exit codes are always 0 or 1 — a review tool's exit code (including revdiff's annotations code 10) is an internal outcome, never the binary's exit status.

## Context

Environment variables set by herdr or injected by this plugin (see the [plugin docs](https://herdr.dev/docs/plugins/)):

| Variable                                                  | Used for                                                                                                                |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `HERDR_PLUGIN_ACTION_ID`                                  | Dispatch: action mode                                                                                                   |
| `HERDR_PLUGIN_ENTRYPOINT_ID`                              | Dispatch: `review` or `setup` pane mode                                                                                 |
| `HERDR_PLUGIN_CONTEXT_JSON`                               | Action mode: source of the repository directory and the caller pane/agent/tab; must be present                          |
| `HERDR_BIN_PATH`                                          | Every callback (`plugin pane open`, `plugin config-dir`, `plugin pane close`, `tab close`, `tab focus`, `agent prompt`) |
| `HERDR_PLUGIN_STATE_DIR`                                  | Base directory for review state (`reviews/`); the system temp directory when unset                                      |
| `HERDR_PLUGIN_CONFIG_DIR`                                 | Config directory for `config.toml`; wins over the `plugin config-dir` CLI call                                          |
| `HERDR_WORKSPACE_ID`, `HERDR_PANE_ID`, `HERDR_TAB_ID`     | Action mode: fallbacks for `workspace_id` / `focused_pane_id` / `tab_id` when the context JSON omits them               |
| `HERDR_REVIEWS_DIR`                                       | Review directory, injected by the action; review-pane mode prefers it over its cwd                                      |
| `HERDR_REVIEWS_CALLER_PANE`, `HERDR_REVIEWS_CALLER_AGENT` | Injected by the action; review-pane mode: delivery target                                                               |
| `HERDR_REVIEWS_CALLER_TAB`                                | Injected by the action; review-pane mode: the focus-return target after a clean ending                                  |
| `HERDR_REVIEW_TOOL`                                       | Review-pane mode: the selected tool id (`revdiff`, `tuicr`, or `lumen`)                                                 |
| `HERDR_TAB_ID`, `HERDR_PANE_ID`                           | Review-pane mode: pane teardown (close pane, then tab)                                                                  |
| `HERDR_PLUGIN_ID`, `HERDR_PLUGIN_ROOT`                    | Set by herdr but unused by this plugin                                                                                  |

Fields read from the context JSON in action mode (all optional; absent when null or blank):

| Field                    | Meaning                                                      |
| ------------------------ | ------------------------------------------------------------ |
| `worktree.checkout_path` | Checkout directory of the worktree the workspace is based on |
| `workspace_cwd`          | Workspace working directory (may be a subdirectory)          |
| `focused_pane_cwd`       | Working directory of the focused pane                        |
| `workspace_id`           | Workspace id, used to open the panes in the right workspace  |
| `focused_pane_id`        | Pane id that receives the annotations on delivery            |
| `focused_pane_agent`     | Agent kind in the focused pane, when any; delivery needs it  |
| `tab_id`                 | Tab of the focused pane; a clean close returns focus to it   |

The caller pane, agent, and tab are captured at action time and travel to the review pane as the `--env HERDR_REVIEWS_CALLER_PANE` / `--env HERDR_REVIEWS_CALLER_AGENT` / `--env HERDR_REVIEWS_CALLER_TAB` pairs of the action's own review-pane open — there is no forwarding hop between panes. Neither pane mode reads the context JSON. The agent has no environment fallback; the pane id falls back to `HERDR_PANE_ID`, the tab to `HERDR_TAB_ID`.

## Directory resolution

```
worktree.checkout_path > workspace_cwd > focused_pane_cwd > error
```

`worktree.checkout_path` wins because `workspace_cwd` may be a subdirectory the user has `cd`-ed into; running the review tool there would review only that subdirectory instead of the repository. A field counts only if it is a present, non-blank string; a non-string or whitespace-only value counts as absent and the chain falls through to the next candidate.

If no field yields a directory, the action exits 1 with `no directory in plugin context`. This happens when the action is invoked outside a workspace context.

## The tool catalog and binary resolution

The catalog is fixed — `lumen`, `revdiff`, `tuicr`, in the setup dialog's alphabetical row order — and not user-configurable:

| Tool    | Minimum | Install                             |
| ------- | ------- | ----------------------------------- |
| lumen   | 2.26.0  | `brew install jnsahaj/lumen/lumen`  |
| revdiff | 1.12    | `brew install umputun/apps/revdiff` |
| tuicr   | 0.27.0  | `brew install tuicr`                |

Tool binaries resolve from `PATH` only, immediately before the spawn: the first `PATH` entry, scanned in order with empty entries skipped, that contains an executable regular file named `<tool>`, canonicalized to an absolute path (the pane process may run with a different `PATH` or cwd than the action process, so no cwd-relative guess is ever made). There is no override variable for any tool; the pre-rename `REVDIFF_BIN` is removed and never read.

Resolution failures are loud:

- `<tool> not found on PATH; install it with <install command>` when no entry yields a binary.
- `found <tool> at <candidate> but cannot resolve it to an absolute path: <err>` when canonicalization fails; the path is shown as given.

The action uses this detection for the direct-open decision on the configured `tool` id; a non-executable file with the tool's name does not count. The setup dialog runs no detection at all — its rows are static.

## The setup dialog

The action opens the setup popup — `herdr plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`, with no `--cwd` and no `--env` pairs (the herdr 0.9.5 daemon also rejects popup opens that target a workspace) — only when no usable tool is configured: the `tool` key unset or blank, an invalid value the config layer already degraded to unset, or a valid id whose binary is not on `PATH`. No review pane opens in that case.

The dialog shows **one universal layout** for every entry case. It runs no PATH detection and reads no config content — no line states which case occurred — and its only environment interaction is resolving the config directory (`HERDR_PLUGIN_CONFIG_DIR`, else the `herdr plugin config-dir alexeyco.review` CLI call) for the path line. The content, as rendered (every content line carries a two-column left padding; the install commands render dim after the tool names):

```
(blank line)
  Config file:
  <config dir>/config.toml — or the `herdr plugin config-dir alexeyco.review` hint when nothing resolves
(blank line)
  Add one of these tools to config:
  lumen    brew install jnsahaj/lumen/lumen
  revdiff  brew install umputun/apps/revdiff
  tuicr    brew install tuicr
(blank line)
  For more information:
  https://github.com/alexeyco/herdr-review
(blank line)
enter/q/esc close
```

- No internal title: the popup window already shows the manifest pane title. One blank line sits above the content, the help footer is pinned to the bottom of the drawable area with blank filler lines (the terminal size is re-read from `stty size` on every redraw, 80×24 fallback), and the frame ends without a trailing newline.
- Every content line starts with the two-column left padding, and the full-width panel-background fill acts as the right padding — top, left, and right all get breathing room. The rows are static and alphabetical (`lumen`, `revdiff`, `tuicr`), each with its install command dim on the same line after a two-column gap; the repository URL line closes the content. The manifest declares the popup size (`width = "60%"`, `height = 15`), which fits the thirteen-line content budget — blank top, eleven content lines, footer — plus a two-row wrap margin with no scrolling; a machine-dependent config path longer than the popup width wraps onto extra visual lines and the margin keeps up to two of them visible with the footer still pinned last (the path stays copy-typable from the command the hint names).
- Colors come from the herdr theme palette: `[theme] name` in the herdr config (`$HERDR_CONFIG_PATH` else `~/.config/herdr/config.toml`), resolved once per dialog run, with the ANSI-16 palette on any resolution failure (unset, unreadable, invalid, or unknown name).
- The dialog is read-only and closes on any key: `Enter`, `q`, and `Esc` each close it — exit 0, the popup auto-closes, no `plugin pane open` call is made, and no file is written anywhere. There are no movement keys and no highlighted row.
- The key reader uses a ~0.1 s read timeout (`stty min 0 time 1`), so a lone Escape is told apart from a closed terminal.

## Placement and tool config

The action reads `config.toml` from the plugin config directory exactly once per launch, after the context parse and before any pane open: a non-blank `HERDR_PLUGIN_CONFIG_DIR` wins outright, otherwise `herdr plugin config-dir alexeyco.review` resolves the directory. Flat keys:

| Key         | Values                              | Default |
| ----------- | ----------------------------------- | ------- |
| `tool`      | `revdiff`, `tuicr`, `lumen`         | unset   |
| `placement` | `tab`, `split`, `overlay`, `zoomed` | `tab`   |
| `direction` | `right`, `down`, `auto`             | `right` |

`tool` decides the launch: a valid id selects the review tool the action checks against `PATH` for the direct open; an unset or blank value is silent and routes the launch to the setup dialog; any other value prints one `herdr-review: warning: invalid tool in config.toml: "<value>"; expected one of revdiff, tuicr, lumen; treating it as unset` warning and is treated as unset. `direction` is passed as `--direction <value>` only when the placement is `split` (`right`/`down` only — the herdr CLI accepts exactly those two); `auto`, an unset key, or a non-split placement omit the flag. `placement = "popup"` is refused — the review pane is a workspace pane and `popup` is not among the placements the herdr CLI documents — printing `herdr-review: warning: placement "popup" is not supported for the review pane; using "tab"` and using `tab`. Every other failure (no config directory, missing or unreadable file, corrupt TOML, an invalid value) prints one `herdr-review: warning:` line and degrades to defaults; the launch never fails on config. Unknown keys are ignored.

## Launch and lifecycle

### Action mode

1. Parse the context JSON, resolve the repository directory and the caller identity (see [Context](#context)).
2. Read `config.toml` once (see [Placement and tool config](#placement-and-tool-config)).
3. Decide: the configured `tool` id, checked against `PATH` with the catalog's detection rules.

   **Usable** — the direct open, the main path:

   ```
   herdr plugin pane open --plugin alexeyco.review --entrypoint review --placement <placement> \
     [--direction <direction>] [--target-pane <caller-pane>] \
     [--workspace <workspace_id>] --cwd <dir> \
     --env HERDR_REVIEWS_DIR=<dir> --env HERDR_REVIEW_TOOL=<tool> \
     [--env HERDR_REVIEWS_CALLER_PANE=<pane>] \
     [--env HERDR_REVIEWS_CALLER_AGENT=<agent>] \
     [--env HERDR_REVIEWS_CALLER_TAB=<tab>] --focus
   ```

   `--direction` appears only for `split` with a right/down direction. `--target-pane <caller-pane>` (the resolved caller pane) appears only for `split` and `zoomed` — the daemon rejects those placements without a target — and when no caller pane resolved the action warns `herdr-review: warning: placement "<placement>" needs a caller pane; using "tab"` and degrades to `tab` before the open, so no `--direction` and no `--target-pane` reach the argv. `--workspace <workspace_id>` appears only for `tab` (the daemon rejects it on every pane-anchored placement); `overlay`, `split`, and `zoomed` opens carry no workspace targeting at all. The reply is parsed best-effort: exit status 0 already means the pane exists, so a non-blank `result.plugin_pane.pane.pane_id` just enables the success log line (`herdr-review: opened review pane <pane>` or `... in tab <tab>` followed by a best-effort `herdr tab focus <tab>`), while an unparsable reply logs `herdr-review: warning: cannot parse review pane open reply: <snippet>` (200 characters) and still succeeds.

   **Not usable** (unset, blank, invalid and degraded to unset, or valid but missing from `PATH`) — the setup popup:

   ```
   herdr plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus
   ```

   No `--workspace`, no `--cwd`, no `--env` pairs: the dialog resolves the config path itself and needs nothing else forwarded, and the herdr 0.9.5 daemon rejects popup opens that target a workspace. Success is the herdr exit status alone — a popup reply carries no pane id, and the reply body is not parsed. Log `herdr-review: opened review tool setup` to stderr (visible via `herdr plugin log list`) and exit 0. No focus calls, no hold.

### Setup-dialog mode

The pane resolves the config directory for its path line (see [The setup dialog](#the-setup-dialog)), draws the one universal dialog, and waits for one key. Any of `Enter`, `q`, and `Esc` closes it: the process exits 0, the popup auto-closes, no herdr call is made, and nothing is written. A terminal failure (raw mode, drawing, restore) is an error, which the pane-mode error policy holds.

### Review-pane mode

1. `herdr::require_bin()` fails early when `HERDR_BIN_PATH` is missing, before any git probe.
2. Resolve the review directory: `HERDR_REVIEWS_DIR` (injected by the action), falling back to the process cwd when absent — herdr runtime commands may default to the plugin directory as cwd.
3. Read the tool from `HERDR_REVIEW_TOOL`. An unset or blank value fails with `HERDR_REVIEW_TOOL is not set; open the review pane through the review action`; an unknown value with `unknown review tool <value>; expected revdiff, tuicr or lumen`. Both precede any git probe or spawn.
4. Probe git read-only and select the scope (see [Scope selection](#scope-selection)). On `NothingToReview` print `herdr-review: nothing to review`, hold the pane, and exit 0 — no tool is spawned, no state directory is touched. Otherwise print `herdr-review: reviewing uncommitted changes` or `herdr-review: reviewing vs <base>`.
5. Prepare the state: create `<state_dir>/reviews` on demand (`state_dir` is the trimmed `HERDR_PLUGIN_STATE_DIR`, or the system temp directory when unset), prune kept `*.out` files older than 7 days, and generate the output file name `<pid>-<unix millis>.out` (the pid keeps two reviews started in the same millisecond from clobbering each other). revdiff writes the file itself; for tuicr and lumen the plugin writes the collected text, so an empty file is created before the spawn and failure paths always have evidence on disk.
6. Resolve the binary PATH-only (see [The tool catalog and binary resolution](#the-tool-catalog-and-binary-resolution)) and spawn per the matrix below. A spawn failure is the hard error `failed to run <bin> in <dir>: <err>`.
7. Collect per tool, decide the outcome, deliver or follow the retention rules.

## Scope selection

Before the review tool starts, the pane runs a set of **read-only** git probes in the review directory (`git` via `Command::new("git").current_dir(dir)`). The index and the working tree are never mutated: no `git add`, no staging, nothing that writes. The catalog tools must not stage or mutate the branch either.

| Probe            | Command in `dir`                           | Read as                            | When                 |
| ---------------- | ------------------------------------------ | ---------------------------------- | -------------------- |
| Repository       | `git rev-parse --is-inside-work-tree`      | exit 0 and stdout `true`           | first                |
| HEAD exists      | `git rev-parse -q --verify HEAD`           | exit 0                             | second               |
| Unstaged changes | `git diff --quiet`                         | exit 1 means present, 0 means none | third                |
| Staged changes   | `git diff --cached --quiet`                | exit 1 means present, 0 means none | fourth               |
| Untracked files  | `git ls-files --others --exclude-standard` | non-empty stdout                   | fifth                |
| Detached HEAD    | `git symbolic-ref -q HEAD`                 | nonzero exit means detached        | only on a clean tree |
| Base chain       | `git rev-parse -q --verify <base>`         | exit 0 means the ref exists        | only on a clean tree |

The base chain probes `origin/HEAD`, then `origin/main`, then `origin/master`, stopping at the first that verifies. A dirty tree skips the detached check and the base chain entirely.

The pure `select_scope` maps the probed state to exactly one scope:

| State                                      | Scope             | stdout announcement                           |
| ------------------------------------------ | ----------------- | --------------------------------------------- |
| Not a repo, or a repo without HEAD         | `NothingToReview` | `herdr-review: nothing to review`             |
| Any staged, unstaged, or untracked changes | `Uncommitted`     | `herdr-review: reviewing uncommitted changes` |
| Clean tree, attached HEAD, base found      | `VsBase(<base>)`  | `herdr-review: reviewing vs <base>`           |
| Clean tree, detached HEAD or no base       | `NothingToReview` | `herdr-review: nothing to review`             |

`NothingToReview` holds the pane (press Enter) and exits 0 without spawning a tool or creating the output file.

On any unexpected git failure — a quiet diff exiting with a code other than 0 or 1, a spawn error, an undocumented exit code — the pane prints one warning to stderr, `herdr-review: git probe failed (<cmd>): <err>; nothing to review`, and takes the nothing-to-review hold path. The old silent fallback to a bare revdiff launch is gone.

Rationale: this mirrors the pattern the official [`umputun/revdiff`](https://github.com/umputun/revdiff) integrations use — choose the diff source from the repository state, never write to the index. Some third-party integrations run `git add -A` to force a full staged diff; that is deliberately rejected because it mutates the user's index and can stage unintended files.

### Per-tool scope fragments

Each scope maps to a static argv fragment per tool:

| Scope            | revdiff       | tuicr             | lumen           |
| ---------------- | ------------- | ----------------- | --------------- |
| `Uncommitted`    | `--untracked` | `-w`              | (none)          |
| `VsBase(<base>)` | `<base> HEAD` | `-r <base>..HEAD` | `<base>...HEAD` |

revdiff's vs-base form is deliberately the two-positional form (`revdiff <base> HEAD`), matching the installed binary's documented interface (`against: second git ref for two-ref diff`); the upstream README documents `<base>..HEAD` as an equivalent single-argument alternative. lumen's three dots review the branch side since the merge base; with no fragment, `lumen diff` reviews HEAD vs the working tree including untracked files. A staged-only tree reviews as `Uncommitted` — each tool runs once, so the staged-only portion cannot be reviewed separately.

## The spawn matrix

The pane process spawns the resolved binary as a direct child with `.current_dir(<review dir>)`:

| Tool    | stdio                                                            | argv                                                                                  | collection after exit                                      |
| ------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| revdiff | stdout+stderr inherit the pane tty                               | `<bin> [scope flags…] --output <out> --exit-code-on-annotations [scope positionals…]` | exit code from the child status, then read `<out>` (lossy) |
| tuicr   | stdout+stderr inherit the pane tty                               | `<bin> -w` or `<bin> -r <base>..HEAD`                                                 | session adapter (below)                                    |
| lumen   | **stdout piped** and captured in memory; stderr inherits the tty | `<bin> diff` or `<bin> diff <base>...HEAD`                                            | captured stdout, converted after exit                      |

revdiff's scope flag goes before the fixed trailing `--output`/`--exit-code-on-annotations` pair and the scope positionals go after it, so both strict and lenient parsers accept the argv.

lumen is the single exception to "argv-exec child, no piped stdio": its annotation export is only written to stdout, and lumen itself renders its TUI to `/dev/tty` when stdout is not a terminal. The pane therefore pipes lumen's stdout with a Rust `Stdio::piped()` on the direct child (no shell involved) and drains the pipe to completion while the child runs; its stderr still inherits the pane tty.

A child killed without an exit code fails with `<tool> terminated without an exit code (killed?); output kept at <path>`; lumen's partial capture is kept, and the other tools' files are already on disk.

## Per-tool collection

### revdiff: the file channel

revdiff writes inline annotations to `<pid>-<unix millis>.out` in the canonical form and exits 10 when there are annotations (`--exit-code-on-annotations`). The pane reads the child's exit code directly from the process status and the file lossily (`String::from_utf8_lossy`; invalid sequences become U+FFFD). A missing or unreadable file is only harmless when revdiff exited 0 — `herdr-review: warning: cannot read <path>: <err>` and empty output; with any other exit code it is the hard error `cannot read revdiff output <path>: <err>` and the file is kept. revdiff output is already canonical and passes through unconverted.

### tuicr: the session adapter

tuicr has no `--output` and no annotations exit code; its review sessions persist on disk. The pane records a launch timestamp before spawning the TUI, then collects this run's comments in order:

1. Predict the session slug `[<owner>/]<repo>@<sanitized-branch>/worktree/<7-char-head>` from read-only git probes: the `origin` remote URL for `[<owner>/]<repo>` (the owner segment is omitted when no remote exists; the repo name falls back to the directory basename), `git branch --show-current` for the branch, and `git rev-parse --short=7 HEAD` for the head.
2. Call `tuicr review comments --session <slug>`. JSON is tuicr's only output format — a `--json` flag does not exist in 0.27.0 and none is passed. A valid JSON comment list answers directly.
3. On failure or an empty answer, fall back to `tuicr review list --repo .` (JSON, newest first), select the newest session updated after the launch timestamp, and fetch its comments with the same `review comments` call. No session from this run means the user quit without a session: a clean empty result.
4. Both paths failing (spawn error or invalid JSON twice) is the hard error `tuicr session lookup failed: <detail>; output kept at <path>`, with the pane held.

A nonzero tuicr exit never reaches the adapter: the (empty) output file is kept and the run fails with the code. The converted canonical text is written to the `.out` file; a write failure is the fatal `cannot write review output <path>: <err>`.

### lumen: the captured stdout export

lumen writes its formatted annotation export to stdout only through its `s`→Enter export action; closing the TUI with `q`/Esc leaves stdout blank. On exit 0 the pane strips terminal-probe escape sequences (CSI/OSC) from the captured stdout — the live spike observed `ESC]11;?BEL` + `ESC[5n` before the payload — then converts the remainder and writes the canonical text to the `.out` file.

- Blank stdout at exit 0 is the clean no-annotations case; the file is removed.
- Non-blank stdout that converts to zero blocks is the hard error `cannot parse lumen annotation export; raw output kept at <path>`, with the raw capture written and kept.
- A nonzero exit writes whatever was captured best-effort and fails with the code.

## The canonical format and conversion

Annotations reach the delivered prompt in a canonical block format: each block is a header line `## <path>:<line> (<side>)` — side `+` or `-` — followed by the annotation body lines, with blocks separated by blank lines. revdiff's native output is already this grammar and passes through unconverted.

tuicr JSON comment → canonical block:

| tuicr field                               | canonical use                                                |
| ----------------------------------------- | ------------------------------------------------------------ |
| `path`                                    | header `<path>`                                              |
| `start_line`                              | header `<line>` — the anchor; `end_line` is never rendered   |
| `side` RIGHT (or `new`) / LEFT (or `old`) | `(+)` / `(-)`; absent or unrecognized sides default to `(+)` |
| `content`                                 | body lines                                                   |

Comments without a usable `path` are skipped: the canonical grammar has no slot for a review-level comment.

lumen export → canonical block:

| lumen pattern                     | canonical header             |
| --------------------------------- | ---------------------------- |
| `**p** line N (RIGHT)`            | `## p:N (+)`                 |
| `**p** line N (LEFT)`             | `## p:N (-)`                 |
| `**p** lines A-B (RIGHT or LEFT)` | `## p:A (+)` or `## p:A (-)` |
| `**p**` (file-level)              | `## p:0 (+)`                 |
| `# <ref>` header line             | dropped                      |
| text lines up to `---`            | body                         |

A range anchors on its first line. LEFT (the removed side in lumen's viewer) maps to `(-)` and RIGHT to `(+)`; a file-level annotation becomes line 0. The mapping is lossy by design: one side marker and one anchor line per block is what the delivered prompt expects.

## Outcome and delivery

The pane classifies the tool's result into exactly one outcome:

| Tool condition                                         | Outcome         |
| ------------------------------------------------------ | --------------- |
| Annotations exist and both caller values are present   | `Deliver`       |
| Annotations exist, a caller value is missing or blank  | `NoAgent`       |
| Clean exit (0, or revdiff's 10) with nothing collected | `NoAnnotations` |
| Any other exit code, even with collected output        | `Failed(<rc>)`  |

Annotations exist when: revdiff exits 10, or exits 0 with non-blank `.out` content; tuicr exits 0 and the session adapter yields a non-empty comment list; lumen exits 0 with non-blank captured stdout. A caller agent is available only when both `HERDR_REVIEWS_CALLER_PANE` and `HERDR_REVIEWS_CALLER_AGENT` are non-blank in the pane environment.

Delivery calls, identically for all three tools:

```
herdr agent prompt <HERDR_REVIEWS_CALLER_PANE> <text>
```

via `HERDR_BIN_PATH`. The prompt is the header `Review annotations from <tool>:\n\n` — `<tool>` is the catalog id — followed by the trimmed canonical text, capped at 64 KiB (65536 bytes) on a UTF-8 character boundary; when truncation happens the plugin appends `\n\n(output truncated; full annotations kept at <path>)`, so the full review stays available on disk. The plugin is agent-agnostic: it does not inspect the agent kind.

- **`Deliver`** — on success, print `herdr-review: sent review annotations to agent pane <target>` to stderr, remove the output file, return focus to the caller tab, attempt a best-effort pane close, and exit 0. On failure, report `failed to send annotations to agent pane <target>: <err>; output kept at <path>`, keep the file, hold the pane, exit 1.
- **`NoAnnotations`** — print `herdr-review: no annotations` to stderr, remove the output file, return focus to the caller tab, attempt the pane close, exit 0.
- **`NoAgent`** — print `herdr-review: no agent in focused pane; annotations kept at <path>` to stderr, keep the file, hold the pane.
- **`Failed(<rc>)`** — report `<tool> failed with exit code <rc>; output kept at <path>`, keep the file, hold the pane, exit 1.

The pane close is best-effort: `herdr plugin pane close <HERDR_PANE_ID>` (skipped when the variable is unset), falling back to `herdr tab close <HERDR_TAB_ID>`; failures are logged (`herdr-review: failed to close pane <pane>: <err>`) and never propagated. Success-path closes are best-effort because plugin panes are expected to auto-close when the pane process exits.

After both clean endings — `Deliver` and `NoAnnotations` — the pane also returns focus to the caller's tab, before the pane close is attempted: `herdr tab focus <HERDR_REVIEWS_CALLER_TAB>` runs best-effort while the pane process is still alive, because closing its own pane kills the process at that call, so a post-close focus would never run (live-probed on herdr 0.9.5). The caller tab is the one the action forwarded from the context's `tab_id`. A missing or blank variable skips the call silently; a failed or unspawnable call prints one `herdr-review: warning: failed to focus caller tab <tab>: <err>` line and leaves the exit code at 0. Hold and error paths never refocus. For `split`, `overlay`, and `zoomed` placements the plugin pane lives inside the caller's tab anyway, so the call is a semantic no-op there; for `tab` it is the point — without it, focus stays wherever the closed review pane left it.

### Retention

The `<pid>-<unix millis>.out` file is:

- **removed** on delivery success and on `NoAnnotations`;
- **kept** on `Failed`, `NoAgent`, a delivery failure, a revdiff read failure with a nonzero exit code, a tuicr adapter failure, a `.out` write failure, a lumen parse failure, and a killed child.

The rules are identical for all three tools. Kept files are pruned best-effort at pane start: `*.out` files directly under `<state_dir>/reviews/` whose modification time is at least 7 days old are removed; removal failures log `herdr-review: warning: cannot remove <path>: <err>` and never abort the review; non-`.out` entries and files whose mtime cannot be read or lies in the future are skipped. `HERDR_PLUGIN_STATE_DIR` is used as given; when it is unset or blank, the system temp directory is used.

If you close the review pane yourself while the tool is open, the pane process is killed: nothing is delivered, and whatever the tool had already flushed stays on disk (pruned after 7 days on a later pane start). This is the intended cancel path.

## JSON parsing

The context is parsed by [`serde_json`](https://crates.io/crates/serde_json) into a generic `Value`. The plugin then takes the first present, non-blank string along the resolution chain (`worktree.checkout_path` → `workspace_cwd` → `focused_pane_cwd`). Standard JSON escaping, including `\uXXXX` with surrogate pairs, survives, so non-ASCII repository paths are preserved. tuicr's `review comments` and `review list` replies are parsed with the same crate.

## Troubleshooting

- **`no directory in plugin context`** — the action ran without workspace context. Invoke it from inside a herdr workspace (e.g. via the keybinding), not from a bare shell.
- **`invoked outside herdr plugin context`** — the binary was run directly, without `HERDR_PLUGIN_ACTION_ID` or `HERDR_PLUGIN_ENTRYPOINT_ID`. Invoke it through herdr.
- **`unknown plugin entrypoint <value>`** — the manifest or the pane-open call named an entrypoint other than `review` or `setup`.
- **`<tool> not found on PATH`** — the selected tool is not on the `PATH` of the pane process (the herdr daemon's environment). Install it (the error and the setup dialog's rows both show the install command) and restart herdr. There is no override variable; `REVDIFF_BIN` no longer exists.
- **`herdr-review: nothing to review`** — the scope resolved to nothing: not a repo, no commits, a detached HEAD on a clean tree, or a clean tree with no `origin/HEAD` / `origin/main` / `origin/master`. Commit or fetch a base, or dirty the tree.
- **Empty review** — the reviewed diff follows the current git state (see [Scope selection](#scope-selection)); when that diff is empty, the tool exits cleanly with no annotations, the plugin logs `no annotations`, and nothing is delivered. A mixed staged+unstaged tree is reviewed as uncommitted changes, so the staged-only portion is not shown separately.
- **Annotations not delivered** — the caller pane had no agent. The review output is kept under `<state_dir>/reviews/`, and the pane waits for Enter before auto-closing. Focus an agent pane and re-run the action.
- **`tuicr session lookup failed: <detail>`** — both the predicted-slug comments call and the list fallback failed. The detail names what each attempt returned; the (empty) output file is kept.
- **`cannot parse lumen annotation export; raw output kept at <path>`** — lumen exited 0 but its capture held no recognizable annotation blocks. The raw capture is in the kept file.
- **`cannot read revdiff output <path>: <err>`** — revdiff exited with an error and its output file could not be read (for example a permissions problem). The file is kept; fix the permissions or the state directory. When revdiff exited 0, an unreadable file is only a warning and the review is treated as having no annotations.
- **A tool fails to start** — the pane prints `herdr-review: reviewing <description>` and the error (for example `failed to run <bin> in <dir>: <err>`), then waits for Enter. Fix the cause and re-run the action; check the action side with `herdr plugin log list --plugin alexeyco.review`.
- **`herdr did not provide HERDR_BIN_PATH (plugin API v1 contract)`** — the process was not given the herdr binary path it needs for callbacks. Run the plugin through herdr rather than directly, and check the herdr install.
- **`placement "popup" is not supported for the review pane`** — a `config.toml` warning, not a failure: the review pane is a workspace pane and `popup` is not a documented placement for it, so the pane opened as a tab instead.
- **The setup dialog appears instead of a review pane** — the `tool` key is unset, blank, invalid, or names a tool missing from the daemon's `PATH`. Add `tool = "<id>"` to the config file the dialog prints, install the named tool, and re-run the action. The dialog itself is the same for every one of these cases — it names no state.
- **The action appears hung** — herdr CLI calls (`plugin pane`, `plugin config-dir`, `tab`, `agent`) have no timeout by design in plugin API v1, so a hung herdr daemon blocks the action; check the daemon first.
- Action logs: `herdr plugin log list --plugin alexeyco.review`.
