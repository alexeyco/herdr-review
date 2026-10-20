# herdr-review

![herdr-review](assets/banner.svg)

herdr plugin that reviews diffs with the human review TUI of your choice and delivers your annotations to the agent in the focused pane.

[herdr](https://herdr.dev) is a terminal workspace manager and agent multiplexer. The catalog holds three tools — [revdiff](https://github.com/umputun/revdiff), [tuicr](https://github.com/agavra/tuicr), and [lumen](https://github.com/jnsahaj/lumen). All three are human diff-review TUIs: you scroll the diff and author the annotations yourself. No LLM keys, accounts, or provider configuration are involved in this flow (lumen ships optional AI helpers; this plugin never calls them). One keypress opens the sole installed tool directly or shows a compact picker when multiple tools are installed; the last choice is remembered. When you finish the review, the plugin collects the tool's annotation export and sends it to the focused agent as one prompt. It targets the herdr plugin API v1 and runs on Linux and macOS (Windows untested).

## Demo

One keypress opens the installed review tool directly when there is one, or shows a picker when there are several; the annotations you author arrive in the focused agent as a single prompt. When no supported tool is installed, the same keypress opens a read-only setup dialog.

<!-- TODO: record a short demo GIF or asciinema cast (keypress → review tool TUI → annotations delivered, plus the setup dialog) and embed here; keep it under ~15s -->

## Other herdr plugins

- [herdr-gitui](https://github.com/alexeyco/herdr-gitui) — open gitui in a herdr pane and report what the session changed to the agent.
- [herdr-open-in](https://github.com/alexeyco/herdr-open-in) — open the workspace or worktree in your editor.

## Quickstart

```
herdr plugin install alexeyco/herdr-review
```

Bind a key in your herdr config:

```toml
[[keys.command]]
key = "prefix+shift+r"
type = "plugin_action"
command = "alexeyco.review.open"
description = "Review changes"
```

Or invoke the action from the CLI:

```
herdr plugin action invoke alexeyco.review.open
```

## Comparison

How this plugin compares with the five other diff-review tools that integrate with herdr, cell by cell as documented in their public sources.

✅ documented in the linked sources · ⚠️ partial or implemented differently · ❌ not found in the public sources checked

|                                                                                                                                          | [herdr-review](https://github.com/alexeyco/herdr-review) | [reviewr](https://github.com/persiyanov/herdr-reviewr) | [hunk-diff](https://github.com/jhochenbaum/herdr-hunk-diff) | [differ](https://github.com/JanSmrcka/differ) | [quantk](https://github.com/quantk/herdr-review) | [lasso](https://github.com/skellleks/lasso) |
| ---------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------- | --------------------------------------------- | ------------------------------------------------ | ------------------------------------------- |
| Ships a herdr-plugin manifest (installs via `herdr plugin install`)                                                                      | ✅                                                       | ✅[^1]                                                 | ✅                                                          | ❌[^2]                                        | ✅                                               | ✅                                          |
| Human-authored line annotations                                                                                                          | ✅                                                       | ✅                                                     | ✅                                                          | ✅                                            | ✅                                               | ✅                                          |
| Dedicated git-diff review surface                                                                                                        | ✅                                                       | ✅                                                     | ✅                                                          | ✅                                            | ✅                                               | ✅                                          |
| Delivers on close, no extra keypress (annotations reach the agent pane when the review closes — key- or action-triggered delivery is ⚠️) | ✅                                                       | ⚠️[^3]                                                 | ⚠️[^4]                                                      | ⚠️[^5]                                        | ⚠️[^6]                                           | ⚠️[^7]                                      |
| Agent-agnostic delivery                                                                                                                  | ✅                                                       | ✅[^8]                                                 | ✅[^9]                                                      | ✅[^10]                                       | ✅[^11]                                          | ⚠️[^12]                                     |
| No LLM keys or accounts in the core flow                                                                                                 | ✅[^13]                                                  | ✅[^14]                                                | ✅[^15]                                                     | ✅[^16]                                       | ✅[^17]                                          | ✅[^18]                                     |
| User-selectable underlying review tool                                                                                                   | ✅                                                       | ❌                                                     | ❌                                                          | ❌                                            | ❌                                               | ❌                                          |

[^1]: reviewr ships `herdr-plugin.toml` in the repository tree — [source](https://github.com/persiyanov/herdr-reviewr).

[^2]: differ ships no `herdr-plugin.toml` in its repository tree; its README documents brew/go install and a "works with herdr" badge — [source](https://github.com/JanSmrcka/differ).

[^3]: reviewr binds `s` to pane.send_text and leaves the review unsubmitted — [source](https://github.com/persiyanov/herdr-reviewr).

[^4]: hunk-diff delivers through an explicit send-review action that runs `herdr agent prompt` — [source](https://github.com/jhochenbaum/herdr-hunk-diff).

[^5]: differ's `S` key submits the review via `herdr agent prompt` — a manual keypress — [source](https://github.com/JanSmrcka/differ).

[^6]: quantk's F7 drafts the annotations and never submits them — [source](https://github.com/quantk/herdr-review).

[^7]: lasso delivers through a Shift+S keypress — [source](https://github.com/skellleks/lasso).

[^8]: reviewr documents delivery to Claude Code, Codex, OpenCode, and Pi — [source](https://github.com/persiyanov/herdr-reviewr).

[^9]: hunk-diff's delivery code treats agent kinds as not addressable (`src/herdr.ts:73`) — [source](https://github.com/jhochenbaum/herdr-hunk-diff/blob/v0.5.0/src/herdr.ts).

[^10]: differ documents delivery to claude, codex, gemini, copilot, opencode, and aider — [source](https://github.com/JanSmrcka/differ).

[^11]: quantk passes an opaque agentKind through to herdr — [source](https://github.com/quantk/herdr-review).

[^12]: lasso's core delivery is generic; its cross-repo discovery is Claude-only (`src/runtime.rs:258`) — [source](https://github.com/skellleks/lasso/blob/v0.1.0/src/runtime.rs).

[^13]: herdr-review — no LLM keys or accounts in the core flow: none of the three catalog tools needs LLM credentials for this flow — [docs/how-it-works.md](docs/how-it-works.md).

[^14]: reviewr — no LLM keys or accounts required or documented in the core flow (only its PR tab needs a `gh`/`glab`/`az` token) — negative search 2026-10-18 — [source](https://github.com/persiyanov/herdr-reviewr).

[^15]: hunk-diff — no LLM keys or accounts required or documented in the core flow (local bundled hunk viewer; requirements are herdr, node, npm) — negative search 2026-10-18 — [source](https://github.com/jhochenbaum/herdr-hunk-diff).

[^16]: differ — no LLM keys or accounts required or documented in the core flow ("no model reads your diff") — negative search 2026-10-18 — [source](https://github.com/JanSmrcka/differ).

[^17]: quantk — no LLM keys or accounts required or documented in the core flow (performs no network access; assets bundled) — negative search 2026-10-18 — [source](https://github.com/quantk/herdr-review).

[^18]: lasso — no LLM keys or accounts required or documented in the core flow (human review only) — negative search 2026-10-18 — [source](https://github.com/skellleks/lasso).

All cells checked 2026-10-18 against the linked public sources.

<details>
<summary>Detailed comparison</summary>

|                    | [herdr-review](https://github.com/alexeyco/herdr-review)                                  | [reviewr](https://github.com/persiyanov/herdr-reviewr)                                | [hunk-diff](https://github.com/jhochenbaum/herdr-hunk-diff)                              | [differ](https://github.com/JanSmrcka/differ)                                                            | [quantk](https://github.com/quantk/herdr-review)                                 | [lasso](https://github.com/skellleks/lasso)                                  |
| ------------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Delivery form      | herdr plugin (manifest install) ([repo](https://github.com/alexeyco/herdr-review))        | herdr plugin (manifest install) ([repo](https://github.com/persiyanov/herdr-reviewr)) | herdr plugin (manifest install) ([repo](https://github.com/jhochenbaum/herdr-hunk-diff)) | standalone binary (brew/go install) integrating with herdr ([repo](https://github.com/JanSmrcka/differ)) | herdr plugin (manifest install) ([repo](https://github.com/quantk/herdr-review)) | herdr plugin (manifest install) ([repo](https://github.com/skellleks/lasso)) |
| Underlying tool    | user-selected: revdiff / tuicr / lumen ([repo](https://github.com/alexeyco/herdr-review)) | single built-in viewer ([repo](https://github.com/persiyanov/herdr-reviewr))          | single built-in viewer ([repo](https://github.com/jhochenbaum/herdr-hunk-diff))          | single built-in viewer ([repo](https://github.com/JanSmrcka/differ))                                     | single built-in viewer ([repo](https://github.com/quantk/herdr-review))          | single built-in viewer ([repo](https://github.com/skellleks/lasso))          |
| Latest release     | [releases](https://github.com/alexeyco/herdr-review/releases)                             | [v0.46.0](https://github.com/persiyanov/herdr-reviewr/releases/tag/v0.46.0)           | [v0.5.0](https://github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0)             | [v1.8.1](https://github.com/JanSmrcka/differ/releases/tag/v1.8.1)                                        | [v0.2.7](https://github.com/quantk/herdr-review/releases/tag/v0.2.7)             | [v0.1.0](https://github.com/skellleks/lasso/releases/tag/v0.1.0)             |
| License / language | [MIT · Rust](https://github.com/alexeyco/herdr-review)                                    | [MIT · Rust](https://github.com/persiyanov/herdr-reviewr)                             | [MIT · TypeScript](https://github.com/jhochenbaum/herdr-hunk-diff)                       | [MIT · Go](https://github.com/JanSmrcka/differ)                                                          | [MIT · JavaScript](https://github.com/quantk/herdr-review)                       | [MIT · Rust](https://github.com/skellleks/lasso)                             |

</details>

Among the tools compared above, this plugin is the only one that combines annotation delivery into the agent pane on review close — no extra keypress — with a choice among multiple installed review tools. The five competitors each deliver annotations through a key or an action, and each ships a single built-in viewer.

## The flow

1. The **action** (`alexeyco.review.open`) resolves the repository and caller identity, reads placement/direction from `config.toml`, and checks the daemon's `PATH` for installed catalog tools. No installed tools opens the read-only **setup dialog**; one installed tool opens the **review pane** directly; multiple tools open a compact picker. The setup dialog shows the config path, install commands, and repository URL, then closes on any key.
2. In the **picker**, Up/Down moves the highlight, Enter opens the selected tool, and Esc closes without launching. The last selected tool is highlighted the next time it remains installed. The review pane opens with configured placement and announces the scope (`herdr-review: reviewing uncommitted changes` or `herdr-review: reviewing vs <base>`), then runs the selected tool as a direct child — argv-exec, no shell layer, no echoed command, straight into the TUI.
3. When you finish, the plugin collects the annotations — revdiff's output file, tuicr's saved review session, or lumen's stdout export — converts them to a common block format, and delivers them to the agent in the focused pane as one prompt: `Review annotations from <tool>:` followed by `## <path>:<line> (+|-)` annotation blocks. After a clean ending — the prompt delivered or no annotations found — focus returns to the tab you launched the review from and the pane closes (best-effort; hold and error paths leave focus alone). With no agent to receive them, or when the tool fails, the pane prints the message, keeps the annotations on disk, and waits for Enter before closing.

Exit codes, per-tool collection channels, delivery rules, and the retention rules for kept output are in [docs/how-it-works.md](docs/how-it-works.md).

### The tool catalog

| Tool                                          | Install                             |
| --------------------------------------------- | ----------------------------------- |
| [lumen](https://github.com/jnsahaj/lumen)     | `brew install jnsahaj/lumen/lumen`  |
| [revdiff](https://github.com/umputun/revdiff) | `brew install umputun/apps/revdiff` |
| [tuicr](https://github.com/agavra/tuicr)      | `brew install tuicr`                |

The catalog is fixed and not user-configurable. Tools are detected on the `PATH` of the herdr daemon: a tool counts as installed when an executable file with its name is found there, scanned in order. The action opens the only installed tool directly, opens a picker for multiple tools, and opens setup when none are installed. There is no per-tool override variable; if a tool is missing, install it and restart herdr so the daemon's `PATH` picks it up. The install commands are brew-first — the setup dialog and the not-found errors show them verbatim. On Linux, install the Rust tools with `cargo install <id>` (`tuicr`, `lumen`); revdiff has no cargo crate, so use its [release binaries](https://github.com/umputun/revdiff/releases) or build from source.

### Review scope

The scope is picked from read-only git probes — the index and the working tree are never mutated:

| Situation                                                    | Scope                                                                   |
| ------------------------------------------------------------ | ----------------------------------------------------------------------- |
| Any staged, unstaged, or untracked changes                   | Uncommitted changes                                                     |
| Clean tree with an attached HEAD and a base that verifies    | vs the base: `origin/HEAD` → `origin/main` → `origin/master`, first hit |
| Detached HEAD on a clean tree                                | `nothing to review`: the pane holds, no tool runs                       |
| Not a git repo, no commits yet, or a clean tree with no base | `nothing to review`: the pane holds, no tool runs                       |

On `nothing to review` the pane prints the line and waits for Enter before closing. A staged-only tree reviews as uncommitted changes: each tool runs once, and none of the three can review the staged-only portion separately in that run.

### Configuration

The review pane's placement and direction come from `config.toml` in the plugin config directory (`herdr plugin config-dir alexeyco.review`; the `HERDR_PLUGIN_CONFIG_DIR` variable wins when set). Tool choice comes from the installed catalog, not config. Flat keys, read once per launch:

| Key         | Values                                        | Meaning                                   |
| ----------- | --------------------------------------------- | ----------------------------------------- |
| `placement` | `tab` (default), `split`, `overlay`, `zoomed` | Where the review pane opens               |
| `direction` | `right` (default), `down`, `auto`             | Split direction; `auto` lets herdr choose |

```toml
placement = "split"
direction = "down"
```

`direction` applies only to `split`; any other placement ignores it. `placement = "popup"` is refused with a warning and falls back to `tab`. The obsolete `tool` key is ignored. Config problems (missing file, corrupt TOML, or an unknown value) degrade to the defaults with a warning; a bad config never blocks the launch.

## Why

- **No git index mutation.** The repository state is probed read-only; the plugin never writes to the index. Third-party integrations that run `git add -A` to force a full staged diff are deliberately not followed.
- **Instant TUI.** Plugin pane commands are argv-exec, not send-text, so there is no shell layer and no echoed command line before the review tool appears.
- **Agent-agnostic.** Annotations go to whichever agent herdr reports in the focused pane; no agent kind is special-cased.
- **Kept output is pruned.** Review output that cannot be delivered stays on disk and is pruned automatically after 7 days.
- **Linux and macOS, ARM included.** Release binaries cover `x86_64` and `aarch64` on both platforms.

## Requirements

- herdr >= 0.9.0
- At least one catalog tool installed on the `PATH` of the herdr daemon (per-tool minimum versions: [docs/how-it-works.md](docs/how-it-works.md))
- Rust toolchain 1.96.1 (the plugin builds itself on install)

Release binaries are unsigned. macOS may quarantine them on first run; `xattr -d com.apple.quarantine <path>` clears the quarantine flag until the binaries are signed and notarized.

## Migrating from the pre-rename plugin

The plugin was renamed — plugin id `revdiff` → `alexeyco.review`, action `review` → `open` (the invoke command is `alexeyco.review.open`), package and binary `herdr-revdiff` → `herdr-review` — and became multi-tool. This is a breaking change. An interim `review` id existed only in unreleased development builds between the two renames; the released versions to migrate from are the `revdiff` 0.1.x line.

1. **Rebind your key**: the action target is now `alexeyco.review.open`, not `revdiff.review`.
2. **Re-register the plugin**: `herdr plugin install alexeyco/herdr-review` (or `herdr plugin link .` in a dev checkout) so the manifest with the new id is registered; the old `revdiff` registration stops being used.
3. **Install a review tool**: install one or more catalog tools on herdr's `PATH`. The action selects the sole installed tool directly or lets you choose when several are installed; without any supported tool it opens the setup dialog.
4. **`REVDIFF_BIN` is removed.** Tool binaries are resolved from `PATH` only; the variable is never read. Install revdiff on the herdr daemon's `PATH` instead.
5. **State moved to the new plugin id**: annotation files now live under `~/.local/state/herdr/plugins/alexeyco.review/reviews/`. Kept files under the old `revdiff` state directory are no longer read or pruned; they were ephemeral, so deleting the old directory is safe.

No migration tooling is shipped: kept annotation files are ephemeral (7-day prune), and the plugin remembers the last selected tool in its state directory.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/development.md](docs/development.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
