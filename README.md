# herdr-review

![herdr-review](assets/banner.svg)

herdr plugin that reviews diffs with the human review TUI of your choice and delivers your annotations to the agent in the focused pane.

[herdr](https://herdr.dev) is a terminal workspace manager and agent multiplexer. The catalog holds three tools — [revdiff](https://github.com/umputun/revdiff), [tuicr](https://github.com/agavra/tuicr), and [lumen](https://github.com/jnsahaj/lumen). All three are human diff-review TUIs: you scroll the diff and author the annotations yourself. No LLM keys, accounts, or provider configuration are involved in this flow (lumen ships optional AI helpers; this plugin never calls them). The tool that runs is fixed by hand in the plugin's `config.toml` (`tool` key); one keypress opens it in a pane rooted at the repository, and when you finish the review the plugin collects the tool's annotation export and sends it to the focused agent as one prompt. It targets the herdr plugin API v1 and runs on Linux and macOS (Windows untested).

## Demo

One keypress opens the configured review tool directly; the annotations you author arrive in the focused agent as a single prompt. When no usable tool is configured, the same keypress opens a read-only setup dialog instead.

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

## The flow

1. The **action** (`alexeyco.review.open`) resolves the repository and the caller identity from the plugin context, reads `config.toml` once, and checks the configured `tool` against the daemon's `PATH`. A valid id with an executable binary opens the **review pane** directly — the main path, no dialog. Otherwise (unset, blank, invalid, or valid but missing from `PATH`) it opens the read-only **setup dialog** instead and no review pane opens: the dialog shows one universal layout — the config file path, the three catalog rows with their dim inline install commands, and the repository URL — and `Enter`, `q`, and `Esc` each close it.
2. The **review pane** opens with the placement from your `config.toml`, announces the scope (`herdr-review: reviewing uncommitted changes` or `herdr-review: reviewing vs <base>`), and runs the configured tool as a direct child — argv-exec, no shell layer, no echoed command, straight into the TUI.
3. When you finish, the plugin collects the annotations — revdiff's output file, tuicr's saved review session, or lumen's stdout export — converts them to a common block format, and delivers them to the agent in the focused pane as one prompt: `Review annotations from <tool>:` followed by `## <path>:<line> (+|-)` annotation blocks. After a clean ending — the prompt delivered or no annotations found — focus returns to the tab you launched the review from and the pane closes (best-effort; hold and error paths leave focus alone). With no agent to receive them, or when the tool fails, the pane prints the message, keeps the annotations on disk, and waits for Enter before closing.

Exit codes, per-tool collection channels, delivery rules, and the retention rules for kept output are in [docs/how-it-works.md](docs/how-it-works.md).

### The tool catalog

| Tool                                          | Install                             |
| --------------------------------------------- | ----------------------------------- |
| [lumen](https://github.com/jnsahaj/lumen)     | `brew install jnsahaj/lumen/lumen`  |
| [revdiff](https://github.com/umputun/revdiff) | `brew install umputun/apps/revdiff` |
| [tuicr](https://github.com/agavra/tuicr)      | `brew install tuicr`                |

The catalog is fixed and not user-configurable. Tools are detected on the `PATH` of the herdr daemon: a tool counts as installed when an executable file with its name is found there, scanned in order. There is no per-tool override variable; if a tool is missing, install it and restart herdr so the daemon's `PATH` picks it up. The install commands are brew-first — the setup dialog and the not-found errors show them verbatim. On Linux, install the Rust tools with `cargo install <id>` (`tuicr`, `lumen`); revdiff has no cargo crate, so use its [release binaries](https://github.com/umputun/revdiff/releases) or build from source.

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

The review tool and the review pane's placement come from `config.toml` in the plugin config directory (`herdr plugin config-dir alexeyco.review`; the `HERDR_PLUGIN_CONFIG_DIR` variable wins when set). Flat keys, read once per launch:

| Key         | Values                                        | Meaning                                     |
| ----------- | --------------------------------------------- | ------------------------------------------- |
| `tool`      | `revdiff`, `tuicr`, `lumen`                   | The review tool that runs; unset by default |
| `placement` | `tab` (default), `split`, `overlay`, `zoomed` | Where the review pane opens                 |
| `direction` | `right` (default), `down`, `auto`             | Split direction; `auto` lets herdr choose   |

```toml
tool = "revdiff"
placement = "split"
direction = "down"
```

`direction` applies only to `split`; any other placement ignores it. `placement = "popup"` is refused with a warning — the review pane is a workspace pane and `popup` is not among the placements the herdr CLI documents — and falls back to `tab`. A `tool` value outside the catalog warns and counts as unset, which routes the launch to the setup dialog. Every config problem (missing file, corrupt TOML, an unknown value) degrades to the defaults with a warning; a bad config never blocks the launch.

## Why

- **No git index mutation.** The repository state is probed read-only; the plugin never writes to the index. Third-party integrations that run `git add -A` to force a full staged diff are deliberately not followed.
- **Instant TUI.** Plugin pane commands are argv-exec, not send-text, so there is no shell layer and no echoed command line before the review tool appears.
- **Agent-agnostic.** Annotations go to whichever agent herdr reports in the focused pane; no agent kind is special-cased.
- **Kept output is pruned.** Review output that cannot be delivered stays on disk and is pruned automatically after 7 days.
- **Linux and macOS, ARM included.** Release binaries cover `x86_64` and `aarch64` on both platforms.

## Requirements

- herdr >= 0.9.0
- The review tool you configure, with the binary on the `PATH` of the herdr daemon (per-tool minimum versions: [docs/how-it-works.md](docs/how-it-works.md))
- Rust toolchain 1.96.1 (the plugin builds itself on install)

Release binaries are unsigned. macOS may quarantine them on first run; `xattr -d com.apple.quarantine <path>` clears the quarantine flag until the binaries are signed and notarized.

## Migrating from the pre-rename plugin

The plugin was renamed — plugin id `revdiff` → `alexeyco.review`, action `review` → `open` (the invoke command is `alexeyco.review.open`), package and binary `herdr-revdiff` → `herdr-review` — and became multi-tool. This is a breaking change. An interim `review` id existed only in unreleased development builds between the two renames; the released versions to migrate from are the `revdiff` 0.1.x line.

1. **Rebind your key**: the action target is now `alexeyco.review.open`, not `revdiff.review`.
2. **Re-register the plugin**: `herdr plugin install alexeyco/herdr-review` (or `herdr plugin link .` in a dev checkout) so the manifest with the new id is registered; the old `revdiff` registration stops being used.
3. **Set your tool**: add `tool = "<id>"` — one of `revdiff`, `tuicr`, `lumen` — to the plugin's `config.toml` (see [Configuration](#configuration)). The keys are unchanged; the file now lives at `~/.config/herdr/plugins/config/alexeyco.review/config.toml`. Without a usable tool the action opens the setup dialog instead of a review.
4. **`REVDIFF_BIN` is removed.** Tool binaries are resolved from `PATH` only; the variable is never read. Install revdiff on the herdr daemon's `PATH` instead.
5. **State moved to the new plugin id**: annotation files now live under `~/.local/state/herdr/plugins/alexeyco.review/reviews/`. Kept files under the old `revdiff` state directory are no longer read or pruned; they were ephemeral, so deleting the old directory is safe.

No migration tooling is shipped: kept annotation files are ephemeral (7-day prune), and the plugin keeps no other persistent state.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/development.md](docs/development.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
