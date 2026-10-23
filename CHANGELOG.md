# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.3.1

### Fixed

- Review staged and unstaged tracked changes together in revdiff, include untracked files in mixed trees, and browse all files in clean committed repositories.

## 0.3.0

### Added

- Open a compact picker when multiple supported review tools are installed; open the only installed tool directly or show setup guidance when none are installed.

### Changed

- Choose tools from installed executables on `PATH` instead of the `tool` config key; retain `placement` and `direction` settings.
- Use terminal-default colors in the setup dialog.

### Fixed

- Retry launching a freshly written tool binary when the operating system reports `ETXTBSY`.

## 0.2.0

### Added

- Multi-tool review driven by `config.toml`: the `alexeyco.review.open` action reads the plugin's `config.toml` once, and the `tool` key (`revdiff` / `tuicr` / `lumen`) names the review tool — all human diff-review TUIs where you author the annotations; no LLM keys are involved. A configured tool detected on `PATH` opens the review pane directly; no dialog is shown.
- When no usable tool is configured — `tool` unset or blank, an invalid value (warned and treated as unset), or a valid id missing from `PATH` — the action opens a read-only setup dialog instead of a review pane. It shows one universal layout for every entry case: the config file path, the three catalog rows with their dim inline brew install commands, and the repository URL; `Enter`, `q`, and `Esc` each close it. The dialog runs no install detection and reads no config content. No review pane opens in that case.
- The chosen tool runs in the review pane and its annotations are delivered to the focused agent as one prompt headed `Review annotations from <tool>:`. revdiff keeps its output-file channel; tuicr's comments are collected after exit through its session adapter (`review comments --session <slug>` with a `review list` fallback); lumen's stdout export is captured while its TUI renders to the pane. tuicr and lumen exports are converted to the canonical `## <path>:<line> (+|-)` annotation blocks; revdiff output passes through unchanged.
- A `config.toml` for the review pane's placement (`tab`/`split`/`overlay`/`zoomed`) and split `direction` (`right`/`down`/`auto`), read from the plugin config directory. `popup` is refused with a warning and falls back to `tab` — the review pane is a workspace pane and `popup` is not a documented placement for it.
- A two-phase review scope shared by all tools: a dirty tree reviews the uncommitted changes; a clean tree reviews against the first base that verifies (`origin/HEAD` → `origin/main` → `origin/master`); a non-repo, commit-less repo, detached HEAD, or missing base prints `nothing to review` and holds the pane. This replaces the five revdiff-specific review modes.

### Changed

- **Breaking:** the plugin is renamed — plugin id `revdiff` → `review`, package and binary `herdr-revdiff` → `herdr-review`, message prefix `herdr-review:`. Rebind keybindings from `revdiff.review` to `review.review`, re-link or re-install the plugin so the new id is registered, set `tool = "<id>"` in the plugin's `config.toml`, and expect state and config paths under the new plugin id; the old registration and paths stop being used.
- **Breaking:** the plugin id is namespaced and the action renamed: id `review` → `alexeyco.review` (dots are legal in herdr ids) and action `review` → `open`, so the invoke command is `alexeyco.review.open`. Rebind keybindings — from `revdiff.review` in the last release, via the interim `review.review`, which existed only in unreleased builds — to `alexeyco.review.open`, re-link or re-install the plugin so the new id is registered, and expect the config directory at `~/.config/herdr/plugins/config/alexeyco.review/` (keys unchanged) and state under `~/.local/state/herdr/plugins/alexeyco.review/`.
- **Breaking:** `REVDIFF_BIN` is removed with no replacement. Tool binaries are resolved from `PATH` only, in the herdr daemon environment.
- The review tool is chosen by the `config.toml` `tool` key, not interactively: there is no tool picker and no remembered choice.

## 0.1.0

### Added

- `revdiff.review` action that opens revdiff in a new herdr tab rooted at the repository, straight into the TUI (the pane process execs revdiff directly, with no shell layer and no echoed command).
- Review mode picked from read-only git probes: `HEAD~1` (last commit) for a clean tree with a parent commit, `--staged` for staged-only changes, `--untracked` for unstaged or untracked changes, and `--all-files` for a repository with no commits (or a single commit and a clean tree). Outside a git work tree, revdiff runs bare. The probes never write the git index.
- Annotations delivered to the agent in the focused pane as a single prompt when revdiff exits with annotations; with no agent, or on a revdiff or delivery failure, the annotations are kept on disk and the path is printed, and the pane holds until Enter.
- Review state kept as a single `<pid>-<millis>.out` file per review under `HERDR_PLUGIN_STATE_DIR/reviews/`, removed on delivery or a clean no-annotation exit, and pruned after 7 days.
- Delivered prompts capped at 64 KiB on a UTF-8 boundary; when truncated, the full annotations stay in the kept file and the prompt says so.
- revdiff resolved from `REVDIFF_BIN` (canonicalized before exec; a non-executable value is a hard error) or the first executable `revdiff` on `$PATH`, with an install hint when neither is found.
- Release binaries for Linux (`x86_64`, `aarch64`) and macOS (`x86_64`, `aarch64`), built by a release pipeline that runs the full CI suite before publishing. The macOS binaries are unsigned.
- Documentation: README, the behavioral spec in `docs/how-it-works.md`, a development guide, a contributing guide, and a security policy.
