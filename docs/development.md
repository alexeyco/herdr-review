# Development

Development happens against a locally linked plugin; no GitHub install needed.

## Dev loop

```
make build                       # produces ./target/release/herdr-review
herdr plugin link .              # register the working directory as a plugin
herdr plugin action list --plugin alexeyco.review
herdr plugin action invoke alexeyco.review.open
herdr plugin log list --plugin alexeyco.review
```

`plugin link` does not run build commands, so rebuild after every source change. Unlink with `herdr plugin unlink alexeyco.review`.

The action calls back into herdr through `HERDR_BIN_PATH` (set by herdr), not through a bare `herdr` on `PATH`. That is the binary herdr launched, so callbacks work even when the plugin is linked.

To exercise the review flow you need the review tool you configure installed — [revdiff](https://github.com/umputun/revdiff) >= 1.12, [tuicr](https://github.com/agavra/tuicr) >= 0.27.0, or [lumen](https://github.com/jnsahaj/lumen) >= 2.26.0 — with `tool = "<id>"` in the plugin's `config.toml`. Tools resolve from the `PATH` of the herdr daemon (there is no override variable), so install the tool and restart herdr when it is missing. `make test` needs no tools at all — the integration suite drives stubs.

## Checks

```
make lint        # cargo fmt --check + cargo clippy
make test        # cargo test
make build       # release build
make fmt-check   # prettier check
```

`make check` runs all of them. CI runs the same commands verbatim (it does not use make); see [.github/workflows/ci.yml](../.github/workflows/ci.yml). Prettier is pinned to `prettier@3.9.6` in the Makefile and in CI, so the local formatting check matches what CI enforces.

`cargo test --locked` runs the unit tests plus the integration suite in `tests/`; the integration tests drive the real binary with stub scripts, so they need no network, no herdr install, and no review tools.

## Integration stubs

`tests/e2e.rs` puts four stubs in a scratch directory and prepends it to `PATH`, so the plugin's PATH-only resolution finds them. Every stub appends its argv to `$STUB_LOG`; assertions grep that log. Knob variables pick the fixture and the exit code per run:

- **`herdr`** answers `plugin pane open` per entrypoint — the setup popup replies without a pane id, the review pane with canned pane/tab ids — plus `plugin config-dir alexeyco.review` (a bare path line, override with `STUB_CONFIG_DIR`). `HERDR_STUB_FAIL_PROMPT=1` fails the `agent prompt` call, exercising the delivery-failure path.
- **`revdiff`** writes `## src/main.rs:1 (+)` annotations to its `--output` file; knobs `REV_EMPTY=1` (empty output), `REV_BIG=1` (~100 KB output, for the truncation cap), `REV_RC` (exit code).
- **`tuicr`** answers the post-exit adapter commands: `review list --repo .` returns one fixture session updated in 2100 (newer than any launch timestamp, so the fallback always picks it), and `review comments --session <slug>` returns comments per `TUICR_COMMENTS=one|empty|invalid`. `TUICR_SESSION_MISS=1` fails the comments call for every slug but the fixture one, forcing the list-fallback path. The TUI invocation itself (argv without a `review` subcommand) exits `$TUICR_RC`. No `--json` flag is passed — JSON is tuicr's only output format.
- **`lumen`** writes its export fixture to stdout — a `# origin/main` ref header plus RIGHT/LEFT/range/file-level blocks with `---` separators — per `LUMEN_OUT=export|blank|garbage`, and exits `$LUMEN_RC`. The pane pipes lumen's stdout, so writing to stdout is exactly the export channel the plugin captures.

A fifth stub, `git`, makes `diff --quiet` exit 2 (an undocumented quiet-diff code) and delegates everything else to the real git resolved at test start; a `git`-less `PATH` covers spawn failure. Fixtures build real repositories in a unique temp directory with an inline git identity, so no global git config is needed.

The setup-dialog unit tests in `src/setup.rs` drive the dialog over injected seams instead of a terminal: an in-memory reader/writer pair for the key loop and frames, a fixed theme palette, a scripted key source whose `None` entries model the `stty min 0 time 1` read timeout, and a recording `stty` fake that pins the raw-mode enter/restore protocol. The e2e `stty` stub (a sixth script on `PATH`) runs the dialog's real raw-mode path over piped stdin. Action e2e runs write a real `config.toml` under `HERDR_PLUGIN_CONFIG_DIR` — `tool = "revdiff"` with the stub on `PATH` drives the direct open; no key, a tool absent from `PATH`, or `tool = "diffy"` drives the setup branch.

## Manual smoke test

```
make build
herdr plugin link .
herdr plugin list                     # shows alexeyco.review 0.2.0
herdr plugin action invoke alexeyco.review.open
```

1. Bind `prefix+shift+r` in `~/.config/herdr/config.toml`:

   ```toml
   [[keys.command]]
   key = "prefix+shift+r"
   type = "plugin_action"
   command = "alexeyco.review.open"
   ```

2. Happy path, per tool (revdiff, tuicr, lumen): set `tool = "<id>"` in the plugin's `config.toml`, focus an agent pane, and invoke the action — the review pane opens directly, no dialog. Review a dirty repo (`herdr-review: reviewing uncommitted changes`), author an annotation, and export — quit revdiff with annotations, let the tuicr TUI save its session, or press `s`→Enter in lumen. The annotations arrive in the agent as one prompt prefixed `Review annotations from <tool>:`.
3. Quit the tool without annotating → nothing is sent.
4. Clean repo with an `origin` remote → `herdr-review: reviewing vs origin/main` and the tool opens the base comparison; clean repo without a remote → `herdr-review: nothing to review` and the pane holds.
5. Remove the `tool` key (or set it to a tool missing from `PATH`, or an invalid value) → the action opens the setup dialog showing the one universal layout — the config path, the catalog rows with their dim inline install commands, and the repo URL; `Enter`, `q`, and `Esc` each close it and no review pane opens.
6. Invoke the action from a pane with no agent → the annotations are kept on disk, the path is printed in the plugin log, and the review pane waits for Enter before closing.
7. Config: `placement = "split"` + `direction = "down"` opens a split; `placement = "popup"` warns and falls back to a tab; a corrupt `config.toml` warns and uses defaults.
8. Inspect the log: `herdr plugin log list --plugin alexeyco.review`.

## The lumen spike

lumen is the only tool spawned with piped stdout — its export channel — while its TUI must still render on the pane pty. If the piped-stdout behavior ever breaks upstream, re-run the spike before touching the spawn code:

1. Check `lumen --version` reports at least 2.26.0.
2. In a herdr pane run `lumen diff > /tmp/lumen-spike.out`. The shell redirect creates the same non-tty stdout condition as the plugin's pipe.
3. Expected: the TUI renders interactively on the pane pty (lumen routes to `/dev/tty`), `s`→Enter opens the exit-and-write confirmation and writes the export into the file, and lumen exits 0.
4. The capture begins with terminal-probe escape sequences (the live spike observed `ESC]11;?BEL` + `ESC[5n` before the payload text); the converter strips them before parsing, so a probe preamble in the file is expected, not a failure.
5. If lumen fails to render or exits nonzero with a redirected stdout, stop: the piped-stdout strategy no longer holds and the lumen spawn/collection design needs revisiting before any code change.

## Version sync

The version lives in two files that must match:

- `Cargo.toml`
- `herdr-plugin.toml`

Bump both in the same commit. A release is cut by pushing a tag `v<version>` (e.g. `v0.1.0`); the release workflow fails if the tag does not match both files.

CI enforces this on every PR: a `version-sync` job fails when the two `version = "..."` values diverge, so drift is caught before a release tag, not only at tag time. An `msrv` job reads `rust-version` from `Cargo.toml`, installs that exact toolchain, and runs `cargo check --locked`.

The release workflow first verifies the tag against both version files, then runs the full CI suite (via `workflow_call`) and only then creates the release and builds artifacts, so a release is gated on the same lint and test matrix as a PR. It ships binaries for four targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. The macOS binaries are unsigned and not notarized. For release steps, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Notes

- herdr validates `min_herdr_version` against the running herdr binary; linking with an older herdr is refused.
- Usage and requirements are documented in the [README](../README.md); behavior details in [how-it-works.md](how-it-works.md).
