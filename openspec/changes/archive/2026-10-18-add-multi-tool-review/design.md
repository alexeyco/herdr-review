# Design

## Context

`herdr-revdiff` is a dual-mode binary (action + pane) wrapping revdiff.
This change renames it to `herdr-review` and turns it into a three-tool
review launcher: the tool is fixed by hand in the plugin's `config.toml`
(pattern of sibling plugin herdr-gitui at `../herdr-gitui`), the action
opens the review pane directly when the configured tool is usable and a
read-only setup popup otherwise (frame/theme machinery patterned on sibling
plugin herdr-open-in at `../herdr-open-in`), and all three tools' annotation
exports funnel into the existing delivery pipeline.

**Superseded decision (revision after groups 1–6 landed).** The original
confirmed playback had the action open an interactive tool picker
("the choice is remembered but always asked again", `last-tool` state file,
✓ preselect). The user retired it mid-apply: the tool is fixed by hand in
config; the choice dialog must never stand between the user and a review.
Everything picker-selection-related (`src/state.rs`, preselect,
persist-on-Enter, `HERDR_REVIEWS_WORKSPACE` forwarding) is replaced as
described below; the dialog machinery that survives is reused by the
read-only `setup` popup. All three catalog tools are
human diff-review TUIs: the user authors annotations inside the tool, and
the plugin only collects each tool's export. None of them needs LLM
credentials for this flow; lumen's optional AI helpers (draft/explain) are
out of scope.

Version floors: herdr >= 0.9 (0.9.5 probed locally), revdiff >= 1.12
(local: `revdiff --version → version: v1.13.0-2cbdee1-20260905T044325`),
tuicr >= 0.27.0 (latest release v0.27.0, 2026-09-23, confirmed on
crates.io and GitHub releases; `cargo search tuicr → tuicr = "0.27.0"`),
lumen >= 2.26.0 (floor pinned by upstream release archaeology: the `s`
export keybind exists since v2.25.0, commit 8a9d0f9, and its current
shape — "drop agent integrations in favor of shell-escape + `s`",
exporting annotations to stdout at exit — shipped in v2.26.0, commit
8b02357; the behavior was additionally verified against v2.32.0/main
sources during research). The spike and manual e2e may run on the
installed brew 2.31.0, which satisfies the floor: tap jnsahaj/lumen is
pinned at stable 2.31.0, so `brew upgrade lumen` cannot go higher; newer
builds via `cargo install lumen` (crates.io 2.32.0) are fine but not
required for the gate.

## Module layout after the change

| Path                                                                                                                                                                       | Fate   | Role                                                                                                             |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------- |
| `src/main.rs`                                                                                                                                                              | modify | dispatch on entrypoint value (`review`/`setup`), error policy, `herdr-review:` prefix                            |
| `src/context.rs`                                                                                                                                                           | modify | action-side plugin-context JSON parsing, now incl. `tab_id` (focus return)                                       |
| `src/herdr.rs`                                                                                                                                                             | modify | CLI client: setup-popup argv, review-pane argv, `plugin config-dir alexeyco.review`, reply parsing               |
| `src/config.rs`                                                                                                                                                            | new    | `config.toml` load (tool/placement/direction), warning-and-default degradation                                   |
| `src/tool.rs`                                                                                                                                                              | new    | catalog (`revdiff`/`tuicr`/`lumen`), install hints, PATH-only detection + resolution                             |
| `src/setup.rs`                                                                                                                                                             | new    | read-only setup-dialog TUI (stty + restore guard, injected seams; was `src/picker.rs` in the intermediate build) |
| `src/theme.rs`                                                                                                                                                             | new    | herdr theme palette resolution for the dialog (shared with the intermediate picker)                              |
| `src/state.rs`                                                                                                                                                             | delete | `last-tool` persistence superseded by the config `tool` key                                                      |
| `src/git.rs`                                                                                                                                                               | modify | read-only probes, two-phase `Scope`, per-tool argv fragments                                                     |
| `src/tuicr.rs`                                                                                                                                                             | new    | slug prediction + session adapter (`review comments` / `review list`)                                            |
| `src/convert.rs`                                                                                                                                                           | new    | tuicr JSON → canonical blocks; lumen export → canonical blocks                                                   |
| `src/review.rs`                                                                                                                                                            | modify | action (config read → direct open or setup popup); review-pane flow (spawn matrix, outcomes, delivery)           |
| `src/revdiff.rs`                                                                                                                                                           | delete | resolution logic generalizes into `src/tool.rs`; `REVDIFF_BIN` removed                                           |
| `tests/e2e.rs`                                                                                                                                                             | modify | stub herdr + stub revdiff/tuicr/lumen; action-decides (direct open vs setup popup) and per-tool pane flows       |
| `Cargo.toml`, `Cargo.lock`                                                                                                                                                 | modify | package `herdr-review`; lock regenerated                                                                         |
| `herdr-plugin.toml`                                                                                                                                                        | modify | identity `alexeyco.review`; action `open`; panes `review` (tab) + `setup` (popup, sized)                         |
| `.github/workflows/release.yml`                                                                                                                                            | modify | `bin: herdr-review` (line 63 today)                                                                              |
| `README.md`, `docs/how-it-works.md`, `docs/development.md`, `AGENTS.md`, `CHANGELOG.md`, `SECURITY.md`, `CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/*`, `assets/banner.svg` | modify | identity + multi-tool behavior content                                                                           |

`Cargo.toml` `repository` becomes `https://github.com/alexeyco/herdr-review`
(the git remote already is `git@github.com:alexeyco/herdr-review.git`).
Version numbers in `Cargo.toml`/`herdr-plugin.toml` stay untouched — the
release version is the maintainer's call.

## Flow

1. Action (`HERDR_PLUGIN_ACTION_ID=review`): parse
   `HERDR_PLUGIN_CONTEXT_JSON` (unchanged chain
   `worktree.checkout_path` → `workspace_cwd` → `focused_pane_cwd`), read
   `config.toml` once (tool/placement/direction), then branch:
   - **Direct open (main path)** — `tool` is a valid catalog id and its
     binary is detected on `PATH`: open the review pane
     `plugin pane open --plugin alexeyco.review --entrypoint review
--placement <placement> [--direction <d>] [--target-pane <caller>]
[--workspace <ws>] --cwd <dir> --env HERDR_REVIEWS_DIR=<dir>
--env HERDR_REVIEW_TOOL=<tool> [--env HERDR_REVIEWS_CALLER_PANE=<pane>]
[--env HERDR_REVIEWS_CALLER_AGENT=<agent>] --focus`, log
     `herdr-review: opened review pane {id}[ in tab {tab}]`, best-effort
     `tab focus` when a tab id parses, exit 0. The daemon's targeting
     contract (live-probed, herdr 0.9.5, unchanged from the intermediate
     build's task 4.16) rules the conditional pairs: `--direction` only for
     `split` with right/down; `--target-pane` only for `split`/`zoomed`
     from the resolved caller pane — without one those placements degrade
     to `tab` with the warning
     `herdr-review: warning: placement "<p>" needs a caller pane; using "tab"`
     because the daemon rejects targetless split/zoomed opens;
     `--workspace` only for `tab` (the daemon rejects it on every
     pane-anchored placement, and the tab fallback shape is the one
     placement it is legal on). The reply is parsed best-effort (exit 0
     already means the pane exists).
   - **Setup dialog** — `tool` unset/blank/invalid (invalid already
     degraded with a warning) or valid but missing from `PATH`: open
     `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup
--focus` — no `--workspace` flag (the daemon rejects popup opens that
     target a workspace with `invalid_params`, "overlay and popup plugin
     panes target the active pane", live-probed), no `--cwd`, no `--env`
     pairs: the dialog re-reads the config and `PATH` itself. Exit status
     alone decides success; popup replies carry no pane id
     (herdr-open-in src/tab.rs:45-49 attests). Log
     `herdr-review: opened review tool setup`, exit 0. No review pane is
     opened on this branch, and nothing is ever written to disk.
2. Setup pane (`HERDR_PLUGIN_ENTRYPOINT_ID=setup`): resolve the config
   directory (`HERDR_PLUGIN_CONFIG_DIR` or `herdr plugin config-dir
review`) solely to print the `config.toml` path, render the one
   universal read-only dialog, then wait: `Enter`, `q`, and `Esc` all
   close it (exit 0, popup auto-closes). No config-content read, no PATH
   detection, no selection, no state, no further herdr calls.
3. Review pane (`HERDR_PLUGIN_ENTRYPOINT_ID=review`): unchanged prelude
   (`require_bin`, `HERDR_REVIEWS_DIR` fallback cwd), then read
   `HERDR_REVIEW_TOOL`, probe git read-only, select the two-phase scope,
   announce it on stdout (`herdr-review: reviewing uncommitted changes` /
   `herdr-review: reviewing vs <base>`), create/prune the `.out` file
   exactly as today, resolve the binary PATH-only, spawn per the matrix,
   collect per tool, convert to canonical, decide the outcome, deliver or
   follow retention rules. `NothingToReview` prints
   `herdr-review: nothing to review`, holds, exits 0 before any state-dir
   work.

## Focus return to the caller tab

After the two clean outcomes — delivered and no-annotations — the pane
process best-effort calls `herdr tab focus <HERDR_REVIEWS_CALLER_TAB>`
once the pane close has been attempted. The action forwards the caller
tab id it resolves from the plugin context on the direct open only; the
resolution follows the existing context-field rules with the
`HERDR_TAB_ID` env fallback, mirroring the caller pane id.
Confirmed: the context field is the top-level `tab_id` — a live
`herdr plugin action invoke` dump on herdr 0.9.5 shows
`"context":{...,"tab_id":"w3S:t4",...}`; when no tab resolves, the pair
is omitted and the pane side skips the call silently.

Why keyed on the forwarded tab and not on placement: pane mode cannot see
the placement its open used — it is not in its environment — and for
`split`/`overlay`/`zoomed` the plugin pane lives inside the caller's tab
anyway, so focusing that tab is a semantic no-op (herdr `tab focus` on an
already-focused tab is assumed harmless — UNVERIFIED, and best-effort
regardless). For `tab` placement the call is the point: without it focus
stays wherever the closed review pane left it.

The pane-side requirement lives in annotation-delivery, not
review-launch: annotation-delivery owns the per-outcome teardown
sequence (deliver → remove → close → exit; no-annotations → remove →
close → exit; hold paths), and the focus call is its final clean-exit
step. review-launch owns open-time behavior and specs the env-pair
forwarding.

## Two-phase scope and base resolution

`select_scope` (pure, unit-tested) replaces `select_mode`:

- non-repo, or repo without HEAD → `NothingToReview`. The confirmed
  contract pins only "no base or detached HEAD"; extending the hold to
  non-repos and commit-less repos is an ASSUMPTION (recorded in
  proposal.md) — tuicr and lumen require a git repository, so the old bare
  revdiff fallback has no multi-tool meaning.
- staged or unstaged or untracked changes → `Uncommitted`. Staged-only
  content is not separately scoped: revdiff cannot express staged-only
  together with `--untracked` in one run — an accepted limitation from the
  confirmed contract. `tuicr -w` and `lumen diff` cover staged+unstaged
  (+untracked for lumen) in one session anyway.
- clean tree → probe `git symbolic-ref -q HEAD` (detached check) and the
  base chain `git rev-parse -q --verify origin/HEAD` → `origin/main` →
  `origin/master`, first hit wins. Base found and HEAD attached →
  `VsBase(<base>)`; otherwise `NothingToReview` (print, hold, exit 0).
- probe failure (spawn error or undocumented exit code) → one warning
  `herdr-review: git probe failed (<cmd>): <err>; nothing to review`, then
  the hold path. The old silent fallback to bare revdiff is gone.

Per-tool argv fragments (consumed by the spawn builder):

| Scope            | revdiff                   | tuicr             | lumen                                                                  |
| ---------------- | ------------------------- | ----------------- | ---------------------------------------------------------------------- |
| `Uncommitted`    | `--untracked`             | `-w`              | (none — `lumen diff` defaults to HEAD vs working tree incl. untracked) |
| `VsBase(<base>)` | positionals `<base> HEAD` | `-r <base>..HEAD` | positional `<base>...HEAD`                                             |

**Decision — revdiff vs-base is pinned to the two-positional form.** The
confirmed contract writes `revdiff <base>..HEAD`. The installed
revdiff 1.13.0 documents two positional refs: `revdiff --help → base: git
ref to diff against (default: uncommitted changes); against: second git
ref for two-ref diff (e.g. revdiff main feature)`; the upstream README
additionally documents the single-argument dot-dot form (`revdiff
main..feature`) as an equivalent alternative. We deliberately pin
`<base> HEAD` — the form documented by the installed binary — which is
semantically identical to `<base>..HEAD`; flagged in the planner report.

lumen's range syntax is confirmed by `lumen diff --help → [REFERENCE]
Commit reference: SHA, HEAD, HEAD~3..HEAD, main..feature, main...feature`
(local 2.31.0). Three dots (`<base>...HEAD`) review the branch side since
the merge base, matching the contract.

## Per-tool spawn matrix

| Tool    | stdio                                                           | argv                                                                      | collection after exit                                                           |
| ------- | --------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| revdiff | stdout+stderr inherit pane tty (`.status()`)                    | `<bin> [flags…] --output <out> --exit-code-on-annotations [positionals…]` | exit code from child + read `.out` (lossy); rc10 or rc0+non-blank ⇒ annotations |
| tuicr   | stdout+stderr inherit pane tty (`.status()`)                    | `<bin> -w` or `<bin> -r <base>..HEAD`                                     | session adapter (below); rc0 + non-empty comment list ⇒ annotations             |
| lumen   | **stdout piped** (captured in memory), stderr inherits pane tty | `<bin> diff` or `<bin> diff <base>...HEAD`                                | captured stdout; rc0 + non-blank ⇒ parse/convert ⇒ annotations                  |

lumen is the single exception to "argv-exec child, no shell layer, no
piped stdio": it must have stdout captured to export annotations, and it
renders its TUI to `/dev/tty` itself when stdout is not a terminal
(verified against lumen v2.32.0 `src/command/diff/app.rs:20-35` per
research). No shell is involved — the pipe is a Rust `Stdio::piped()` on
the direct child.

**Spike gate (task group 1).** Before any of this is built, verify on a
live herdr machine that a lumen child spawned with piped stdout inside a
plugin pane still renders to the pane pty. Method without code changes: in
a herdr pane run `lumen diff > /tmp/lumen-spike.out` — the shell redirect
is the same pipe condition. Expected: TUI renders interactively; `s`→Enter
writes the export to the file; exit 0. **Decision branch:** if lumen
instead fails or renders nothing when stdout is not a tty, lumen degrades
to a no-delivery pure TUI: spawn with inherited stdio, never capture, and
map every clean lumen exit to `NoAnnotations` (pane closes silently; the
README notes lumen delivers no annotations). The rest of the design is
unaffected; the reviewer-execution and annotation-delivery lumen scenarios
would need a matching delta revision before implementation continues —
stop and report instead of improvising.

## tuicr session adapter

tuicr has no `--output` and no exit-code signal (codes 0/1/2 only); its
review sessions persist on disk. Collection strategy, in order:

1. Record the launch timestamp (`SystemTime`) **before** spawning tuicr.
2. Predict the session slug
   `[<owner>/]<repo>@<sanitized-branch>/worktree/<7-char-head>`
   (grammar from unpublished tuicr `src/slug.rs` per research; docs.rs
   examples are stale):
   - `[<owner>/]<repo>`: parse the `origin` remote URL
     (`git remote get-url origin`, read-only); omit the `<owner>/` segment
     when no remote exists. Repo name from the remote path, falling back to
     the directory basename.
   - `<sanitized-branch>`: `git branch --show-current` through tuicr's
     sanitizer. Exact sanitize rules are unpublished — ASSUMPTION (untested);
     start with the identity mapping and let step 3 cover misses.
   - `<7-char-head>`: `git rev-parse --short=7 HEAD`.
3. `tuicr review comments --session <slug>` (JSON is the only output
   format; `--json` does not exist in 0.27.0 — live-probed). Valid JSON list ⇒ done.
4. Fallback on failure or empty session: `tuicr review list --repo .`
   (JSON, newest-first, carries `comment_count` and update times), select
   the newest session updated **after the launch timestamp**, then
   `tuicr review comments --session <slug-from-list>`.
5. Both paths failed (spawn error or invalid JSON twice) ⇒ hard error
   `tuicr session lookup failed: <detail>; output kept at <path>`, hold,
   exit 1. Empty comment list with rc0 ⇒ `NoAnnotations`. rc≠0 ⇒
   `Failed(rc)`, adapter never runs.

Converted canonical text is written to the `.out` file so retention,
truncation notes, and the no-agent path all keep working unchanged. The
adapter calls use the same resolved tuicr binary with
`.current_dir(<review dir>)` and `.output()` (captured, not tty — these are
non-interactive queries run after the TUI exited).

## lumen capture and parse

Captured stdout layout (verified against v2.32.0 research): an optional
`# <ref>` header line, then blocks separated by `---`, each starting with
one of:

- `**path** line N (RIGHT)` / `**path** line N (LEFT)` — line annotation
- `**path** lines A-B (RIGHT)` / `**path** lines A-B (LEFT)` — range
- `**path**` — file-level

Remaining lines of a block are the annotation text. Parse rules: drop
`# <ref>` headers and unrecognized lines before the first block; a range
anchors on `A`; non-blank stdout that parses to zero blocks is a hard
error (`cannot parse lumen annotation export; raw output kept at <path>`)
with the raw capture written to the `.out` file; rc≠0 writes the raw
capture best-effort and fails as `Failed(rc)`.

## Canonical format and mapping tables

Canonical block grammar (revdiff's native output format — revdiff passes
through unconverted):

```
## <path>:<line> (<side>)
<body line>
<body line>

## <path2>:<line2> (<side2>)
...
```

side ∈ `+ | -`; blocks separated by blank lines.

tuicr JSON comment → canonical:

| tuicr field                  | canonical use                                                              |
| ---------------------------- | -------------------------------------------------------------------------- |
| `path`                       | header `<path>`                                                            |
| `start_line`                 | header `<line>` (anchor; `end_line` is not rendered)                       |
| `side` RIGHT / LEFT / absent | `(+)` / `(-)` / `(+)` default — annotations overwhelmingly target new code |
| `content`                    | body lines                                                                 |

lumen export → canonical:

| lumen pattern                  | canonical header                                                                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `**p** line N (RIGHT)`         | `## p:N (+)`                                                                                                                                |
| `**p** line N (LEFT)`          | `## p:N (-)`                                                                                                                                |
| `**p** lines A-B (RIGHT/LEFT)` | `## p:A (+)` / `## p:A (-)`                                                                                                                 |
| `**p**` (file-level)           | `## p:0 (+)` — line 0 marks whole-file; `(+)` is the default side (user confirmed `## <path>:0`; the side marker is this design's decision) |
| `# <ref>`                      | dropped                                                                                                                                     |
| text lines up to `---`         | body                                                                                                                                        |

LEFT≈removed-side/context in lumen's viewer maps to `(-)`; RIGHT≈added
side maps to `(+)`. The mapping is lossy by design: the canonical format
has one marker per block, which is what the agent prompt contract expects.

## Setup dialog

**One universal layout (user ruling, this revision).** The interim
per-branch dialog (tool-unset instructions / configured-missing message /
nothing-installed greying) is gone: every entry case leads to the same
popup anyway, so the branches only duplicated the same information with
PATH- and config-state probing the dialog no longer needs. The layout is
the user's sketch, applied literally (the user does not vouch for the
English wording — keep it as written):

```
Config file:
  <resolved config.toml path, or the `herdr plugin config-dir alexeyco.review` hint when unresolvable>

Add one of these tools to config:
  lumen    brew install jnsahaj/lumen/lumen
  revdiff  brew install umputun/apps/revdiff
  tuicr    brew install tuicr

For more information:
  https://github.com/alexeyco/herdr-review
```

Rows are alphabetical (lumen, revdiff, tuicr — the sketch's order), and
each install command renders dim after the tool name on the SAME line
(2-column gap), not on an indented second line. The catalog rows are
static: the dialog runs no PATH detection and reads no config content —
its only environment interaction is the config-dir resolution for the path
line (hint fallback when unresolvable).

**Padding.** The blank top line already exists (open-in frame chrome).
Add a visible left padding of two columns on every content line (the
sketch's indent), and keep the full-width panel-background fill that
already acts as the right padding — top, left, and right all get
breathing room. Footer stays pinned to the drawable bottom via filler
lines and the per-frame `stty size` re-read (80×24 fallback).

**Popup sizing.** `herdr-plugin.toml`'s `[[panes]] id = "setup"` declares
`height = 15` and `width = "60%"`. Height budget (13 content rows: blank,
`Config file:` + path, blank, `Add one of these tools to config:` + 3
rows, blank, `For more information:` + URL, blank, footer) plus a
2-line wrap margin (height 15): on the live machine the config-path line
wrapped at 60% width and clipped inside the original 13-row budget —
the margin absorbs up to two wrapped lines, footer stays pinned last.
Evidence that
popup panes declare sizing keys: herdr-open-in `herdr-plugin.toml:16-17`
ships `width = "40%"`, `height = 16`; herdr-gitui's settings docs record
the width token value set as "positive integer cells or an `N%` string",
so the percentage style is the attested manifest form. Longest fixed
content line ≈ 45 columns (`  revdiff  brew install umputun/apps/revdiff`);
60% of an 80-column terminal is 48 cells, so the fixed content never
wraps even on the narrowest realistic terminal. Machine-dependent
`config.toml` paths longer than the popup width WRAP onto extra visual
lines — live-probed: the long home path wrapped and pushed the 13-row
frame past the popup bottom, clipping the footer. Fix: `height = 15`,
a two-line wrap margin; up to two wrapped path lines stay visible with
the footer still pinned last. The path stays copy-typable from
`herdr plugin config-dir alexeyco.review`, which the dialog names in
its hint fallback anyway.

**Install commands (ASSUMPTION, proposal.md).** `src/tool.rs` CATALOG
install strings become the three brew commands of the sketch — macOS-first
by user decision; the lumen/tuicr formulas were not probed (revdiff's tap
was: `brew info revdiff → umputun/apps/revdiff: stable 1.13.0`). The Linux
alternative `cargo install <id>` lives in README docs text only, not in
the dialog or the catalog. The same CATALOG strings feed the pane-side
not-found error (`<tool> not found on PATH; install it with <hint>`), so
that message now shows the brew command.

**What survives from the interim build** (`src/setup.rs`): raw mode via
`stty` child (no FFI, no extra dependency), saved `stty -g` settings
restored by RAII guard + explicit restore, cursor hide/show, frame redraw
with injected reader/writer seams for unit tests, ~0.1 s read timeout to
tell a lone Escape from a closed terminal, the open-in frame chrome (no
internal title, blank top line, pinned footer, no trailing newline), and
the `src/theme.rs` palette resolution (herdr config `[theme] name` from
`$HERDR_CONFIG_PATH` else `~/.config/herdr/config.toml`, built-ins,
ANSI-16 fallback). `Enter`, `q`, and `Esc` all just close (exit 0, no
calls, no writes). What is deleted: the `Branch` enum and per-branch
instruction blocks, the greyed/dim row-state rendering (dim survives only
as the inline install-command style), and the dialog-side PATH detection.

## Placement config

Port of the herdr-gitui settings pattern (`../herdr-gitui/src/
settings.rs`): flat-key `config.toml`, directory from
`HERDR_PLUGIN_CONFIG_DIR` or `herdr plugin config-dir alexeyco.review`, read once
per action launch before the branch decision (the setup dialog re-reads it
for its own rendering), every degradation a warning, never fatal, defaults
`placement = "tab"` / `direction = "right"` / `tool` unset. Keys: `tool` ∈
revdiff|tuicr|lumen (invalid → warning naming the value + treated as
unset), `placement` ∈ tab|split|overlay|zoomed, `direction` ∈
right|down|auto (auto ⇒ omit `--direction`; the herdr CLI accepts exactly
`right`/`down`: `herdr plugin pane open --help → --direction <DIRECTION>
[possible values: right, down]`). No width/height keys.

**Why `popup` is refused for the review pane.** The review pane is a
workspace pane; `popup` is not among the placements
`herdr plugin pane open --help` documents, and the refusal predates this
revision — with the intermediate picker flow herdr 0.9.5 answered
`ui_busy` to a second concurrent popup (locally probed). The rule and its
warning-then-`tab` fallback are kept unchanged by the new contract; the
warning text loses its stale "(the tool picker is already a popup)"
parenthetical and reads `herdr-review: warning: placement "popup" is not
supported for the review pane; using "tab"`. The setup popup's own
placement is fixed in the manifest, mirroring herdr-open-in
(`herdr-plugin.toml:12-17` ships `placement = "popup"`), and the CLI
accepts `popup` at request time even though `--help` lists only
overlay/split/tab/zoomed — probed:
`herdr plugin pane open --placement popup --plugin __no_such_plugin__ →
missing required --entrypoint`, and with an entrypoint id →
`{"error":{"code":"plugin_not_found",...}}` (a daemon-side answer, so
`popup` passed CLI value validation).

## Rename: breaking change and migration

- Plugin id `revdiff` → `alexeyco.review`, action id `review` → `open`:
  user keybindings bound to `revdiff.review` must be rebound to
  `alexeyco.review.open` (the user's exact requirement: "command в
  настройках был alexeyco.review.open"), and the plugin must be re-linked
  (`herdr plugin link` under the new manifest). Rationale (user's call): a
  namespaced id avoids the `review (review)` display collision and
  colliding action ids across plugins; dots are legal in herdr ids —
  precedent `ray.plugin-manager`. The manifest `name = "Review"` stays the
  human-friendly display value — id is the machine key, name is what
  humans read (this design's judgment). Pane entrypoint ids stay `review`
  and `setup`: dispatch env values, `--entrypoint` argv, and error strings
  are untouched; only the `--plugin` argv value, the `herdr plugin
config-dir` callback, and herdr-owned paths change. The unreleased
  interim identity `review`/`review.review` (groups 1–8) never shipped —
  migration is from the released `revdiff` 0.1.0 only. Both REJECTED-RISK
  lines in proposal.md cover the daemon-PATH and keybinding-breakage
  decisions.
- Binary/package `herdr-revdiff` → `herdr-review`; release workflow
  artifact name changes (`release.yml` `bin:` key).
- Config/state paths move under the new plugin id (herdr-owned): config to
  `~/.config/herdr/plugins/config/alexeyco.review/` (the `config.toml`
  keys are unchanged), state to
  `~/.local/state/herdr/plugins/alexeyco.review/`. Old kept `.out` files
  and any `REVDIFF_BIN` export stop being used. README gets a short
  migration note; no data migration tooling (kept outputs are ephemeral,
  7-day prune).
- `REVDIFF_BIN` removed outright — PATH-only detection for all tools
  (REJECTED-RISK 1 in proposal.md).
- Message texts change prefix (`herdr-review:`) and the delivery header
  gains the tool name (`Review annotations from <tool>:`); tests that grep
  these strings are updated in the same task groups.

## Testing strategy

- Unit tests (seams, no processes): `select_scope` branches; per-tool
  fragment tables; slug prediction; tuicr/lumen converters (all mapping
  rows, including malformed input); config parsing/degradation matrix
  (now including the `tool` key); catalog detection over synthetic PATH
  strings; setup-dialog universal-layout rendering (padding, alphabetical
  row order, dim inline install commands, height fit) and the
  close-on-any-key loop via injected reader/writer; argv builders (exact
  flag order) for both pane-open calls.
- Integration e2e (`tests/e2e.rs`, stub scripts + `STUB_LOG` grep, the
  existing regime): stub `herdr` answers both `plugin pane open` shapes
  (setup popup: no pane id needed; review: canned pane/tab ids), mirrors
  the daemon targeting rules (workspace only on tab, target-pane on
  split/zoomed), and answers `plugin config-dir`; action runs write a real
  `config.toml` into `HERDR_PLUGIN_CONFIG_DIR` to drive the branch
  (direct open vs setup popup, invalid-tool warning). Tool stubs:
  `revdiff` (existing, minus `REVDIFF_BIN` — stubs go on `PATH`),
  `tuicr` (answers `review comments --session <slug>` and
  `review list --repo .` from fixtures, with rc knobs), `lumen` (writes
  export fixtures to stdout, with rc and blank knobs). Repo fixtures:
  dirty repo, clean repo **with an `origin` remote and `origin/main`**
  (the new base chain replaces the old `HEAD~1` expectation), clean repo
  without remote (nothing-to-review hold), detached HEAD.
- Manual e2e on a live machine per tool (config `tool` → direct open →
  annotations to agent) plus every setup-dialog branch — the success
  criterion from the confirmed contract; `make check` green;
  `openspec validate add-multi-tool-review` VALID.

## Trade-offs decided

- **PATH-only detection** (no `<TOOL>_BIN`): uniformity across three tools,
  at the cost of depending on the herdr daemon's PATH (user accepted;
  REJECTED-RISK 1).
- **Two positional refs for revdiff vs-base** instead of the contract's
  range shorthand: matches revdiff's documented interface.
- **Slug prediction before list fallback** for tuicr: one cheap call in the
  common case, robust fallback when the unpublished sanitizer diverges.
- **Best-effort reply parsing in the action**: exit status 0 already means
  the pane exists; erroring on a parse failure would fail a launch that
  herdr accepted.
- **Canonical `.out` content for tuicr/lumen**: the kept file always holds
  what was (or would have been) delivered, so truncation notes and the
  no-agent path point at useful evidence; lumen raw captures survive only
  on failure paths.
- **Config is the only chooser, read in the action** (supersedes the
  interactive picker and its "config read in the picker pane" trade-off):
  the tool is fixed by hand in `config.toml`, the action owns the whole
  decision — direct open or setup dialog — and no dialog ever stands
  between the user and a review (user ruling). The setup dialog is
  deliberately inert: informational branches, close-on-any-key, zero
  writes, zero herdr calls.
