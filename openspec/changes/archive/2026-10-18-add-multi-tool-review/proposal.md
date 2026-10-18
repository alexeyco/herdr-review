# Proposal

## Why

The plugin was renamed to `herdr-review` and stops being a single-tool revdiff
wrapper. Users fix their review tool — `revdiff`, `tuicr`, or `lumen`, all
human diff-review TUIs where the user authors annotations — by hand in the
plugin's `config.toml`, and the plugin keeps its core value: collecting the
tool's annotation export and delivering it as a prompt to the caller agent
pane in one unified flow.

## What Changes

- **BREAKING** Identity rename: plugin id `revdiff` → `alexeyco.review`
  (namespaced id — user's explicit call: it avoids the `review (review)`
  display collision and colliding action ids across plugins; dots are legal
  in herdr ids, precedent `ray.plugin-manager`), action id `review` →
  `open`, so the keybinding/invoke target moves `revdiff.review` →
  `alexeyco.review.open` (user's exact requirement: "command в настройках
  был alexeyco.review.open"); binary `herdr-revdiff` → `herdr-review`,
  display `name = "Review"`, error prefix `herdr-revdiff:` →
  `herdr-review:`; config and state directories move to the new plugin id.
  Pane entrypoint ids (`review`, `setup`) are unchanged. Release version
  numbers are untouched (maintainer's call).
- The review tool is a new gitui-style `config.toml` key: `tool =
"revdiff" | "tuicr" | "lumen"`. When the key is set and its binary
  resolves on `PATH`, the `review` action opens the review pane directly —
  no dialog at all. This is the main path.
- Otherwise the action opens a read-only setup popup (entrypoint `setup`,
  popup placement sized `width = "60%"` / `height = 15`, frame/theme
  machinery patterned on sibling plugin herdr-open-in) showing ONE
  universal layout for every entry case — config file path, `Add one of
these tools to config:` with the alphabetical catalog rows `lumen /
revdiff / tuicr` (install command dim, inline after the name), and
  `For more information:` with the repo URL. No branching on install or
  config state, no greyed rows. `Enter`, `q`, and `Esc` all just close it.
- The action opens the review pane with placement from the same
  `config.toml` (`placement`/`direction`; `popup` is refused with a warning
  and falls back to `tab`).
- After a clean review-pane ending (delivered or no-annotations) the plugin
  best-effort returns focus to the caller's tab; the action forwards the
  resolved caller tab id as `HERDR_REVIEWS_CALLER_TAB` on the direct open
  only.

Superseded decision (revised after groups 1–6 were implemented): the
interactive tool picker with `last-tool` memory — "the choice is remembered
but always asked again" — is RETIRED by the user's ruling: the tool is fixed
by hand in config, and the choice dialog must never stand between the user
and a review. The `last-tool` state file, the ✓ preselect, the
persist-on-Enter, and `src/state.rs` are removed; the picker pane becomes
the read-only `setup` dialog; `HERDR_REVIEW_TOOL` is produced by the action
from config, never by an interactive choice.

- Scope selection replaces the five revdiff-specific modes with two phases:
  dirty tree → uncommitted review; clean tree → vs base
  (`origin/HEAD` → `origin/main` → `origin/master`); no base, detached HEAD,
  non-repo, or no HEAD commit → print `nothing to review` and hold the pane.
- Per-tool execution in the review pane: revdiff keeps its inherited-tty
  `--output`/`--exit-code-on-annotations` channel; tuicr runs on the
  inherited tty and its comments are collected after exit through a session
  adapter (`review comments --session <slug>`, falling back to
  `review list --repo .`); lumen's stdout is piped and captured (lumen
  renders its TUI to `/dev/tty` itself) — the single exception to argv-exec
  with no shell layer.
- Tool detection and binary resolution are PATH-only; **BREAKING** the
  `REVDIFF_BIN` override is removed with no replacement.
- tuicr and lumen annotation exports are converted to the canonical
  `## path:line (+|-)` block format; the delivered prompt header becomes
  `Review annotations from <tool>:`. Output-file model
  (`<pid>-<millis>.out` under `$HERDR_PLUGIN_STATE_DIR/reviews/`, 7-day
  pruning) and retention rules generalize unchanged to all three tools.

## Capabilities

### New Capabilities

- `setup-dialog`: the read-only setup popup — one universal layout (config
  path with a CLI hint fallback, static alphabetical catalog rows with dim
  inline install commands, repo URL), padding and popup sizing, raw-mode
  frame/theme/restore machinery, close-on-any-key behavior. (Replaces the
  earlier `tool-picker` capability and the interim per-branch dialog of
  this same unarchived change; the baseline has neither.)
- `reviewer-execution`: replaces `revdiff-execution` — tool selection from
  the pane environment, PATH-only binary resolution for all three tools, the
  `.out` file model and pruning, per-tool spawn/stdio strategies, and
  per-tool result collection (revdiff file channel, tuicr session adapter,
  lumen stdout capture).
- `placement-config`: gitui-style `config.toml` read once per action
  launch; `tool`/`placement`/`direction` keys with warning-and-default
  degradation (defaults mean "tool unset" → setup dialog); `popup` refused
  for the review pane.

### Modified Capabilities

- `review-launch`: the action decides — config-read, then either the direct
  review-pane open (configured placement, `HERDR_REVIEW_TOOL` env, daemon
  targeting rules) or the setup popup; pane-open reply handling adapts
  (popup replies carry no pane id, direct-open replies parse best-effort).
- `review-scope-selection`: two-phase dirty/clean scope with base-chain
  resolution and the nothing-to-review hold replaces the five-mode
  selection; per-tool argv fragments replace the revdiff-only flag map.
- `annotation-delivery`: per-tool outcome rules, canonical converters for
  tuicr/lumen, tool-named prompt header, retention and messages generalized
  to all three tools.
- `mode-dispatch`: renamed identity, error prefix, and manifest (id
  `alexeyco.review`, action `open`, panes `review` + `setup` with the setup
  popup sizing keys); entrypoint value now selects review pane mode vs
  setup-dialog mode.
- `revdiff-execution`: **all requirements removed** — the capability is
  superseded by `reviewer-execution`; behavior that survives (output file
  model, pruning, revdiff spawn channel) migrates there.

## Impact

- Code: `src/main.rs`, `src/herdr.rs`, `src/review.rs`, `src/git.rs`
  modified; `src/revdiff.rs` and `src/state.rs` deleted; new `src/tool.rs`,
  `src/setup.rs` (the dialog; intermediate build had `src/picker.rs`),
  `src/config.rs`, `src/theme.rs`, `src/tuicr.rs`, `src/convert.rs`;
  `src/context.rs` gains `tab_id` parsing (focus return, group 11).
  `tests/e2e.rs` reworked (stubs for all three
  tools, action-decides flow).
- Packaging/CI: `Cargo.toml` (name, description, keywords, repository),
  `Cargo.lock` (regenerated), `herdr-plugin.toml` (identity, action, two
  panes, setup popup sizing keys), `.github/workflows/release.yml`
  (`bin: herdr-review`).
- Docs/meta: `README.md`, `docs/how-it-works.md`, `docs/development.md`,
  `AGENTS.md`, `CHANGELOG.md` (Unreleased entry; historical entries keep the
  old name), `SECURITY.md`, `CONTRIBUTING.md`,
  `.github/ISSUE_TEMPLATE/*`, `assets/banner.svg`.
- Users: keybindings must move from `revdiff.review` to
  `alexeyco.review.open`; one re-link of the plugin; the old registration
  stops being used; config moves to
  `~/.config/herdr/plugins/config/alexeyco.review/` (the `config.toml` keys
  are unchanged) and state to the matching herdr-owned directory;
  `REVDIFF_BIN` stops working; the review tool must be declared as
  `tool = "<id>"` in the plugin's `config.toml`.
- Environment: herdr >= 0.9 (0.9.5 probed locally); platforms linux+macos;
  tool minimums revdiff >= 1.12, tuicr >= 0.27.0, lumen >= 2.26.0 — tools must
  never stage or mutate the branch/worktree; all plugin git probes stay
  read-only.
- Out of scope: LLM provider/keys (no catalog tool needs LLM auth for this
  flow; lumen's optional AI helpers stay out), Windows, user-configurable
  tool catalog, interactive tool choosing (the dialog never selects; config
  is the only chooser), tuicr forge features (`tuicr pr`, `:submit`),
  `popup` placement for the review pane, hunk-level scope.

REJECTED-RISK: herdr daemon PATH may lack user tools (~/.cargo/bin etc.) — per-tool <TOOL>_BIN overrides removed by explicit user choice, no escape hatch.
REJECTED-RISK: renaming plugin id revdiff→review breaks existing users' keybindings (revdiff.review), registration, and config/state paths — breaking-change release, user's call.

ASSUMPTION: config keys named exactly `placement`/`direction` like herdr-gitui (herdr-gitui src/settings.rs) plus the new flat key `tool`; no width/height keys because `popup` is refused for the review pane.
ASSUMPTION: the tool travels to the review pane as injected env `HERDR_REVIEW_TOOL`, produced by the action from the `tool` config key and passed on `plugin pane open --env` alongside `HERDR_REVIEWS_DIR`/`HERDR_REVIEWS_CALLER_PANE`/`HERDR_REVIEWS_CALLER_AGENT` (field-verified by the groups 1–6 e2e and live acceptance; the earlier popup-forwarding variant with `HERDR_REVIEWS_WORKSPACE` is superseded — the setup popup receives no env at all and resolves the config path itself).
ASSUMPTION: install commands are the user-sketched all-brew set — lumen `brew install jnsahaj/lumen/lumen`, revdiff `brew install umputun/apps/revdiff` (tap metadata verified locally: `brew info revdiff → umputun/apps/revdiff: stable 1.13.0`), tuicr `brew install tuicr` (untested — the lumen and tuicr formulas were not probed; a missing formula would disprove; macOS-first is the user's decision, the Linux alternative `cargo install <id>` lives in README docs text only, both crates verified via `cargo search`).
ASSUMPTION: the setup-dialog layout wording is user-drafted and applied literally; the user does not vouch for the English phrasing — keep it as written.
ASSUMPTION: file-level annotations map to `## <path>:0` (user-confirmed default); the trailing side marker `(+)` on that header is a design decision recorded in design.md.
ASSUMPTION: non-repo directories and repos without a HEAD commit take the nothing-to-review hold path — the confirmed contract only pins "no base or detached HEAD".
ASSUMPTION: tuicr CLI surface (`-w`, `-r <range>`, `review comments --session <slug>` — JSON is the only output format, a `--json` flag does not exist; disproven by live probe of installed tuicr 0.27.0 during group 4 and corrected in the artifacts; `review list --repo .`, slug grammar `[<owner>/]<repo>@<sanitized-branch>/worktree/<7-char-head>`) per research of agavra/tuicr at v0.27.0.
ASSUMPTION: release version choice is the maintainer's — no task bumps `version` in `Cargo.toml`/`herdr-plugin.toml`.
