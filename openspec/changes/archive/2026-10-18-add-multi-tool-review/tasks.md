# Tasks

Task groups run in order; groups 2–5 and 8 touch overlapping files
(`src/review.rs`, `tests/e2e.rs`) and serialize. Groups 1–6 are
implemented (the intermediate picker build); group 8 is the config-driven
refactor the user ruled mid-apply, and group 7 (manual verification) runs
after it. Scenario names cited in unchecked tasks match the current delta
spec `#### Scenario:` headings verbatim; citations inside the ticked
groups 1–6 are historical records of the intermediate build. Version
numbers stay untouched through groups 1–11 (the release version is the
maintainer's call); group 12 performs the maintainer-approved bump to
0.2.0.

## 1. Spike: lumen piped stdout inside a herdr pane (decision gate)

- [x] 1.1 On the live herdr machine, check `lumen --version` reports at
      least the 2.26.0 floor (the planning machine's brew install is 2.31.0,
      which already satisfies it — tap jnsahaj/lumen is pinned at stable
      2.31.0, so `brew upgrade lumen` cannot go higher; a newer build via
      `cargo install lumen` is fine but not required for the gate). No tree
      changes.
- [x] 1.2 In a herdr pane run `lumen diff > /tmp/lumen-spike.out`. Expected:
      the TUI renders interactively on the pane pty (lumen routes to
      `/dev/tty`), `s`→Enter writes the formatted export into the file, and
      lumen exits 0. Record the export head and the exit code.
      Result (2026-10-17, herdr 0.9.5, lumen 2.31.0, automated via
      `herdr pane split/run/send-keys/read`): TUI rendered in the pane with
      stdout redirected; annotation added; `s`→Enter modal
      "Exit lumen and write 1 annotation to stdout?"; exit 0; capture:
      `^[]11;?^G^[[5n**file.txt** line 2 (RIGHT)\n\nspike annotation 2`
      (59 bytes) — leading OSC/CSI terminal-probe sequences leak into stdout,
      the payload text itself is clean.
- [x] 1.3 Decision gate: if 1.2 renders and captures, the piped-stdout
      strategy stands and group 4 proceeds as specced. DECISION: piped-stdout
      strategy stands, with a mandatory converter rule: the lumen capture
      parser MUST strip leading/stray CSI/OSC/ANSI escape sequences before
      block parsing (the observed probe preamble `ESC]11;?BEL` + `ESC[5n`
      precedes the payload). Fallback branch not triggered. If lumen fails to render
      with a non-tty stdout, STOP and report — design.md "Spike gate" fallback
      branch applies (lumen becomes a no-delivery pure TUI) and the
      reviewer-execution / annotation-delivery lumen deltas must be revised
      before implementation continues.

## 2. Identity rename sweep (code, manifest, CI, mechanical docs)

Files: `Cargo.toml`, `Cargo.lock`, `herdr-plugin.toml`, `src/main.rs`,
`src/review.rs`, `src/herdr.rs`, `src/git.rs`, `src/revdiff.rs`,
`tests/e2e.rs`, `.github/workflows/release.yml`, `README.md`,
`docs/how-it-works.md`, `docs/development.md`, `AGENTS.md`, `SECURITY.md`,
`CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/bug_report.yml`,
`.github/ISSUE_TEMPLATE/proposal.yml`, `.github/ISSUE_TEMPLATE/config.yml`,
`assets/banner.svg`. Behavior stays revdiff-only; no version bumps.

- [x] 2.1 Rename the package to `herdr-review` in `Cargo.toml` (name,
      description, keywords, `repository =
"https://github.com/alexeyco/herdr-review"`) and regenerate `Cargo.lock`;
      leave both `version` keys untouched.
- [x] 2.2 Rewrite `herdr-plugin.toml` identity: `id = "review"`,
      `name = "review"`, description, action title/description, and both
      command arrays → `./target/release/herdr-review`. Do not add the `picker`
      pane yet (group 4).
- [x] 2.3 Rename symbols and strings across `src/` and `tests/e2e.rs`:
      error/warning prefix `herdr-revdiff:` → `herdr-review:` (`src/main.rs`,
      `src/review.rs`, `src/git.rs` probe warning), `HOLD_PROMPT`,
      `--plugin revdiff` → `--plugin review` (`src/herdr.rs:71` and its argv
      tests), `CARGO_BIN_EXE_herdr-revdiff` → `CARGO_BIN_EXE_herdr-review`,
      e2e fixture/tempdir name tags. Keep `PROMPT_HEADER` as `Review
annotations from revdiff:` (still accurate; group 4 generalizes it) and
      keep `REVDIFF_BIN`/`src/revdiff.rs` working for now. Run
      `grep -rn "herdr-revdiff" src tests .github Cargo.toml
herdr-plugin.toml Makefile` and fix every call and test site found.
- [x] 2.4 `.github/workflows/release.yml` line 63: `bin: herdr-revdiff` →
      `bin: herdr-review`.
- [x] 2.5 Mechanical name replacements in `README.md`, `docs/*.md`,
      `AGENTS.md`, `SECURITY.md`, `CONTRIBUTING.md`,
      `.github/ISSUE_TEMPLATE/*`, and the text nodes of `assets/banner.svg`.
      Do not touch `CHANGELOG.md` historical entries and do not touch
      `openspec/specs/**` (the archive step owns baseline updates).
- [x] 2.6 Expected build result after this group alone: `make check` green;
      `grep -rn "herdr-revdiff" .` outside `CHANGELOG.md`, `openspec/`, and
      `target/` returns nothing; the binary builds at
      `target/release/herdr-review` with unchanged behavior (action still
      opens the review pane directly, revdiff-only).

## 3. Two-phase scope selection (still revdiff-only)

Files: `src/git.rs`, `src/review.rs`, `tests/e2e.rs`.

- [x] 3.1 Replace `Mode`/`select_mode` in `src/git.rs` with
      `Scope`/`select_scope` (`Uncommitted`, `VsBase(String)`,
      `NothingToReview`): extend `GitStatus` with `detached` and
      `base: Option<String>`; drop the `HEAD~1` probe; add clean-only probes
      `git symbolic-ref -q HEAD` and the base chain `git rev-parse -q --verify`
      for `origin/HEAD` → `origin/main` → `origin/master`. Unit-test every
      branch, covering scenarios: "Dirty tree reviews uncommitted changes",
      "Clean tree with a base reviews against it", "Clean tree without any
      base holds", "Detached HEAD on a clean tree holds", "Non-repo directory
      holds", "Repo without commits holds", "Probe stops early for a non-repo
      directory", "Probe stops early when HEAD does not exist", "Full probe on
      a repository with commits", "Dirty tree skips the base chain", "Base
      chain stops at the first hit".
- [x] 3.2 Keep revdiff's fragment API on the new `Scope`: `Uncommitted` →
      flag `--untracked`; `VsBase(base)` → positionals `[base, "HEAD"]` (the
      two-ref form pinned by the design decision; upstream also documents an
      equivalent `<base>..HEAD` single-argument form we deliberately do not
      use); `NothingToReview` has no fragments.
      (Group 4 generalizes this to the per-tool table.)
- [x] 3.3 Rework the pane flow in `src/review.rs`: keep the
      `herdr::require_bin()` pre-check (scenario "Missing HERDR_BIN_PATH fails
      early"); print `herdr-review: reviewing uncommitted changes` or
      `herdr-review: reviewing vs <base>`; on `NothingToReview` print
      `herdr-review: nothing to review`, call `hold_pane()`, and exit 0 before
      any state-dir work or spawn. Probe failures print `herdr-review: git
probe failed (<cmd>): <err>; nothing to review` and take the same hold
      path, covering scenarios "Unexpected diff exit code holds" and "Probe
      spawn failure holds".
- [x] 3.4 Update `tests/e2e.rs`: the clean-repo fixture gains an `origin`
      remote with an `origin/main` ref (bare repo or `git update-ref`);
      assertions move from `HEAD~1` to the two positional refs; add a
      nothing-to-review e2e (clean repo, no remote → hold path, exit 0 with
      null stdin). Expected build result after this group alone: `make check`
      green, revdiff-only behavior intact.

## 4. Multi-tool core: picker, config, per-tool execution, conversion, delivery

Files: new `src/tool.rs`, `src/picker.rs`, `src/state.rs`, `src/config.rs`,
`src/tuicr.rs`, `src/convert.rs`; modified `src/main.rs`, `src/herdr.rs`,
`src/review.rs`, `src/git.rs`, `herdr-plugin.toml`, `tests/e2e.rs`;
deleted `src/revdiff.rs`. One group because all tasks are compile-coupled
through `src/review.rs`; splitting would leave a picker offering tools the
pane cannot run.

- [x] 4.1 `src/tool.rs`: catalog type (`revdiff`/`tuicr`/`lumen`, fixed
      order) with install-hint strings (`brew install umputun/apps/revdiff`,
      `cargo install tuicr`, `cargo install lumen`), PATH-only detection
      (executable regular file, empty entries skipped) and pre-spawn resolution
      with canonicalization; no env override is read. Unit tests cover:
      "Installed tool is selectable" (detection), "Non-executable file does not
      count", "First executable hit wins", "No binary anywhere", "Candidate
      cannot be canonicalized", "REVDIFF_BIN is ignored", "The selected tool
      comes from the pane environment" via its scenarios "Valid tool id selects
      the tool", "Missing tool id is an error", "Unknown tool id is an error".
- [x] 4.2 `src/state.rs`: `last-tool` under `HERDR_PLUGIN_STATE_DIR` —
      forgiving read, atomic `<id>\n` write (temp + rename), `Result` for the
      caller to warn. Unit tests cover: "Last-used installed tool is
      preselected with a check mark" (read side), "Stale last-tool falls back
      to the first installed tool", "Choice is persisted as a bare id line",
      "Save failure warns but launches" (error surface).
- [x] 4.3 `src/picker.rs`: raw-mode popup TUI ported from herdr-open-in
      (`stty` + RAII restore guard, injected reader/writer seams, ~0.1 s Escape
      timeout, `>` selection arrow, `✓` preselect suffix, greyed rows with the
      indented install-command line, footer help). Unit tests via seams cover:
      "Enter on a greyed row is inert", "Escape cancels", "Terminal is restored
      on every exit path", "Picker always asks", "Missing tool is greyed with
      its install command", "No tools installed" (message construction for the
      held-error requirement; the hold itself comes from the mode-dispatch
      policy).
- [x] 4.4 `src/config.rs`: `config.toml` load per the herdr-gitui settings
      pattern — dir from `HERDR_PLUGIN_CONFIG_DIR` or `herdr plugin config-dir
review`, read once, flat keys `placement`/`direction`, defaults
      tab/right, every degradation one `herdr-review: warning:` line, unknown
      keys ignored, `popup` refused with the specced warning text. Unit tests
      cover: "Environment directory wins", "CLI fallback resolves the
      directory", "No config directory available warns and uses defaults",
      "Missing file uses defaults", "Corrupt file warns and uses defaults",
      "Each accepted placement maps to argv", "Popup falls back to tab with a
      warning", "Unknown placement value falls back", "Split with direction",
      "Auto omits the flag", "Direction ignored without split", "Invalid
      direction warns and omits the flag".
- [x] 4.5 `src/herdr.rs`: add `config_dir()`, `picker_pane_open_args()` and
      `review_pane_open_args()` (exact argv orders pinned by the specs) plus
      reply handling — popup open: exit status only, body ignored; review open:
      best-effort `pane_id`/`tab_id` parse with a 200-char-snippet warning.
      Unit tests cover: "Picker-open argv carries the forwarded identity",
      "Popup open succeeds without a pane id in the reply", "Reply without a
      usable pane id warns but succeeds", "Overlay-style reply without a tab",
      "Popup reply body is ignored", "Resolved directory is passed twice", "The
      tool env pair is always present", "Direction is passed only for split
      placement", "Split and zoomed pass the caller pane as target", "The
      workspace flag is passed only for tab placement", "The pane-open argv
      always carries its own placement",
      "HERDR_BIN_PATH is missing or blank", "A CLI call exits nonzero", "The
      herdr binary cannot be spawned".
- [x] 4.6 `src/main.rs` + `herdr-plugin.toml`: dispatch on the entrypoint
      value (`review` → review pane, `picker` → picker, other non-blank →
      `unknown plugin entrypoint <value>; expected review or picker`); declare
      the second pane `[[panes]] id = "picker"`, `placement = "popup"`, same
      command array. Tests cover: "Only the entry-point variable is set",
      "Unknown entrypoint is an error", "Both mode variables are set", "Neither
      mode variable is set", "Pane-mode error is held for reading", "Error with
      closed stdin does not hang", "Action-mode error exits immediately",
      "Success in either mode", "revdiff child failure does not leak its code",
      "Whitespace-only entry point with a set action", "Both variables set to
      blank values", "Best-effort cleanup failure warns rather than fails",
      "Both entry points resolve to the same binary"; confirm the CI version-sync job stays
      untouched ("Manifest and Cargo versions diverge").
- [x] 4.7 `src/review.rs` action mode: replace the direct review-pane open
      with the picker-popup open; log `herdr-review: opened tool picker`; exit 0
      without focus calls. Tests cover: "Action success opens the popup and
      exits", "Pane id taken from the environment", "Agent without context
      entry is simply absent".
- [x] 4.8 `src/review.rs` picker mode: read `HERDR_REVIEWS_DIR` (hard error
      when missing), `HERDR_REVIEWS_WORKSPACE`/`CALLER_PANE`/`CALLER_AGENT`
      (optional); detect catalog; run picker with preselect; persist choice
      (warn-only failure); load config; open the review pane; log `herdr-review:
opened review pane {id}[ in tab {tab}]` + best-effort `tab focus`; cancel
      exits 0 silently. Tests cover: "The picker takes the review directory and
      forwarded identity from its injected environment" via "Forwarded values
      reach the review-pane open" and "Missing directory variable is an error",
      plus "Cancel exits cleanly", "Pane opens with a tab", "Split or zoomed
      without a caller pane falls back to tab" (and "The workspace flag is
      passed only for tab placement" at the picker layer).
- [x] 4.9 `src/git.rs`: generalize 3.2 to the per-tool fragment table
      (revdiff/tuicr/lumen × Uncommitted/VsBase). Unit tests cover:
      "Uncommitted fragments per tool", "Vs-base fragments per tool", "revdiff
      uses two positional refs, not a range argument".
- [x] 4.10 `src/review.rs` review-pane spawn: read `HERDR_REVIEW_TOOL`,
      resolve via `src/tool.rs`, spawn per the matrix — revdiff `.status()`
      with `<bin> [flags…] --output <out> --exit-code-on-annotations
[positionals…]`; tuicr `.status()` with `-w` / `-r <base>..HEAD`; lumen
      with piped stdout captured to memory and inherited stderr, `diff` /
      `diff <base>...HEAD`. Tests cover: "revdiff argv keeps the fixed trailing
      flags", "tuicr runs on the inherited tty", "lumen stdout is piped while
      stderr is inherited", "Spawn failure", "Killed child keeps the output".
- [x] 4.11 `src/tuicr.rs`: launch-timestamp capture, slug prediction
      (`origin` remote parse, `git branch --show-current`,
      `git rev-parse --short=7 HEAD`), `review comments --session <slug>` (JSON is the only output format; a
      `--json` flag does not exist in 0.27.0 — live-probed), then the
      `review list --repo .` fallback filtered to sessions
      updated after the launch timestamp. Unit tests with fixture JSON cover:
      "Predicted slug answers directly", "Fallback list picks the session
      created by this run", "Adapter failure keeps evidence", "Nonzero tuicr
      exit skips the adapter".
- [x] 4.12 `src/convert.rs`: tuicr JSON → canonical blocks and lumen export
      → canonical blocks per the design mapping tables (side defaults `+`,
      ranges anchor on the first line, file-level → `:0 (+)`, `# <ref>`
      dropped, revdiff pass-through). Unit tests cover: "tuicr comment converts
      with side mapping", "tuicr comment without side defaults to plus", "lumen
      RIGHT line converts", "lumen line range anchors at the first line",
      "lumen file-level annotation converts to line 0", "lumen ref header is
      dropped", "revdiff output passes through unchanged", "Terminal-probe
      preamble is stripped before parsing".
- [x] 4.13 `src/review.rs` per-tool result collection: revdiff file read
      (lossy, rc0-unreadable warns empty, rc10-missing hard error); tuicr
      adapter → convert → write canonical text to `.out`; lumen capture →
      convert → write canonical text, stdout blank after escape stripping =
      no annotations,
      unparseable non-blank = write raw + hard error, rc≠0 = write raw
      best-effort + `Failed(rc)`; `.out` write failure = `cannot write review
output <path>: <error>`. Keep the state-dir/prune machinery, renamed.
      Tests cover: "Output path layout", "State directory created on demand",
      "State directory creation failure is fatal", "System clock before the
      epoch is fatal", "Missing state-dir variable uses the temp directory",
      "Old output removed, fresh output kept", "Pruning never blocks the
      review", "Output file unreadable after a clean exit", "Missing output at
      an annotations exit code", "Invalid UTF-8 in the output file", "No
      comments is a clean no-annotations", "Tuicr canonical text write failure
      is fatal", "Lumen canonical text write failure is fatal", "Export on
      stdout delivers", "TUI closed without exporting",
      "Unparseable export keeps raw", "Nonzero exit keeps raw capture".
- [x] 4.14 `src/review.rs` outcomes + delivery: per-tool `decide`
      (revdiff rc10/rc0+non-blank; tuicr rc0+non-empty list; lumen
      rc0+non-blank conversion), header `Review annotations from <tool>:`, cap and
      truncation note unchanged, renamed messages (`herdr-review: sent review
annotations to agent pane <target>`, `herdr-review: no annotations`,
      `herdr-review: no agent in focused pane; annotations kept at <path>`,
      `<tool> failed with exit code <rc>; output kept at <path>`), retention
      rules for all tools; delete `src/revdiff.rs` and its `mod` line, and
      `grep -rn "REVDIFF_BIN\|revdiff::" src tests` must come up empty. Tests
      cover: "Nonzero success code with agent delivers", "tuicr with comments
      delivers", "lumen export delivers", "Clean exit with blank output is
      no-annotations", "Annotations without an agent", "Any other exit code
      fails regardless of output", "Header names the tool that ran", "Prompt
      within the cap is sent intact", "Oversized prompt is cut at a character
      boundary", "Successful delivery tears the pane down and removes the
      file", "Failed delivery keeps the file and holds the pane", "No
      annotations cleans up silently", "Unreadable output at a clean exit
      counts as no annotations", "Failed run surfaces the code and keeps
      evidence", "Pane close falls back to tab close".
- [x] 4.15 `tests/e2e.rs` adaptation: extend the stub `herdr` to answer
      `plugin config-dir review` and both pane-open shapes; put stub tools on
      `PATH` (no `REVDIFF_BIN`); action e2e asserts the picker-popup argv;
      review-pane e2e sets `HERDR_REVIEW_TOOL=revdiff`, plus error runs for
      missing and unknown tool env. Expected build result after this group
      alone: `make check` green with the full picker-first flow for revdiff
      and unit-level coverage for tuicr/lumen.

- [x] 4.16 Acceptance fix round (live herdr 0.9.5, group 7 finding): the
      daemon rejects targeted popup/overlay opens (`invalid_params: overlay
and popup plugin panes target the active pane`) and targetless
      split/zoomed opens. `picker_pane_open_args` passes no `--workspace`;
      `review_pane_open_args` passes `--workspace` only for `tab`,
      `--target-pane <caller>` for `split`/`zoomed`, and the picker flow
      degrades `split`/`zoomed` to `tab` with the specced warning when no
      caller pane is forwarded. Tests: the two new argv scenarios in 4.5,
      the fallback scenario in 4.8, and the e2e stub `herdr` now mirrors the
      daemon rules — any `--workspace` with `overlay`/`popup`, or
      `split`/`zoomed` without `--target-pane`, exits 1 with the daemon's
      `invalid_params` body.
- [x] 4.17 Acceptance fix round 2 (picker dialog, user eyeball finding): the
      picker frame must match herdr-open-in exactly — no internal title
      (the popup window already shows the manifest title), blank line above
      the rows, footer pinned to the drawable bottom via filler lines and
      re-read `stty size` per frame (80×24 fallback), no trailing newline,
      selected row `▸ name` (U+25B8, as in herdr-open-in) bold with fg panel_bg on bg accent padded to
      full width, unselected `  name` fg text on bg panel_bg, preselect row
      suffixed with a check mark; port the open-in theme palette resolution (herdr config
      `[theme] name` from `$HERDR_CONFIG_PATH` else
      `~/.config/herdr/config.toml`, 18 built-ins, ANSI-16 fallback) into a
      new `src/theme.rs`; greyed rows keep dim + indented install line.
      Tests: "Frame chrome carries no internal title", "Footer is pinned to
      the drawable bottom", "Selected row renders in the open-in style",
      "Frame colors follow the herdr theme palette", updated frame-shape
      assertions in 4.3's tests ("Picker always asks", check-mark and
      greyed-row frames).
- [x] 4.18 Acceptance fix round 2 (lumen no-export, user finding): closing
      lumen without exporting leaves only terminal-probe escapes in the
      capture; the blank check must run on the stripped remainder —
      probe-preamble-only ⇒ NoAnnotations, nothing delivered, `.out`
      removed. Tests: "TUI closed without exporting" (probe-only capture
      variant), "Clean exit with blank output is no-annotations" (lumen
      leg), e2e `LUMEN_OUT=probes` asserting no `agent prompt` call.

## 5. Stub e2e for tuicr and lumen

Files: `tests/e2e.rs` only.

- [x] 5.1 Add `TUICR_STUB`: logs `TUICR <argv>` to `$STUB_LOG`; answers
      `review comments --session <slug>` from a fixture comments JSON
      (knobs: `TUICR_COMMENTS=one|empty|invalid`, `TUICR_SESSION_MISS=1` to
      force the fallback), answers `review list --repo .` from a fixture
      session list with an update time newer than any launch timestamp, exits
      `$TUICR_RC` (default 0).
- [x] 5.2 Add `LUMEN_STUB`: logs `LUMEN <argv>`; writes the export fixture
      (`# origin/main` header, RIGHT/LEFT/range/file-level blocks with `---`
      separators) to stdout per `LUMEN_OUT=export|blank|garbage`, exits
      `$LUMEN_RC` (default 0).
- [x] 5.3 tuicr e2e runs: deliver (prompt logged with header `Review
annotations from tuicr:` and canonical `## <path>:<line> (+)` blocks,
      `.out` removed), list-fallback path, empty comments → `herdr-review: no
annotations` + file removed, `TUICR_RC=1` → `tuicr failed with exit code
1` + file kept + exit 1, dirty repo → argv `-w`, clean fixture repo →
      argv `-r origin/main..HEAD`.
- [x] 5.4 lumen e2e runs: export → delivered canonical prompt (file-level
      block arrives as `## <path>:0 (+)`), `LUMEN_OUT=blank` → no annotations +
      file removed, `LUMEN_OUT=garbage` → `cannot parse lumen annotation
export; raw output kept at` + file kept, `LUMEN_RC=1` → raw kept +
      failure, dirty repo → argv `diff`, clean fixture repo → argv
      `diff origin/main...HEAD`.
- [x] 5.5 Expected build result after this group alone: `make check` green
      with all three stub suites; every delivery/retention scenario asserted
      per tool.

## 6. Docs and meta content

Files: `README.md`, `docs/how-it-works.md`, `docs/development.md`,
`AGENTS.md`, `CHANGELOG.md`, `SECURITY.md`, `CONTRIBUTING.md`,
`.github/ISSUE_TEMPLATE/bug_report.yml`,
`.github/ISSUE_TEMPLATE/proposal.yml`, `assets/banner.svg`.

- [x] 6.1 `README.md`: multi-tool flow (action → picker → review pane →
      annotations to agent), picker keys and greying, catalog table with
      install commands and repo URLs (Linux note for the brew-only revdiff
      hint), minimum versions (herdr >= 0.9, revdiff >= 1.12, tuicr >= 0.27.0,
      lumen >= 2.26.0), `config.toml` keys with the popup refusal, and a
      migration section: rebind `revdiff.review` → `review.review`, re-link the
      plugin, `REVDIFF_BIN` removed, state/config paths moved to the new plugin
      id. Make clear all three tools are human review TUIs — the user authors
      annotations; no LLM keys are involved in this flow.
- [x] 6.2 `docs/how-it-works.md`: rewrite the flow — two-phase scope with
      the base chain and nothing-to-review hold, per-tool spawn matrix, tuicr
      adapter, lumen capture, canonical format and mapping tables, retention
      rules.
- [x] 6.3 `AGENTS.md` gotchas: replace the `REVDIFF_BIN` gotcha with
      PATH-only detection; tri-mode dispatch (`review`/`picker` entrypoints +
      action); lumen piped-stdout exception to argv-exec; picker popup and the
      `ui_busy` second-popup constraint; `config.toml` keys; `.out` model
      wording generalized to all tools.
- [x] 6.4 `docs/development.md`: build/link/test under the new names;
      describe the three tool stubs and the spike procedure for lumen.
- [x] 6.5 `CHANGELOG.md`: add an `Unreleased` section describing the rename
      (breaking) and multi-tool support; no version number; historical entries
      untouched.
- [x] 6.6 `SECURITY.md`, `CONTRIBUTING.md`, issue templates: name updates;
      bug template gains a tool selector (revdiff/tuicr/lumen) and asks for the
      scope line printed by the pane.
- [x] 6.7 `assets/banner.svg`: replace the `herdr-revdiff` text nodes with
      `herdr-review`.
- [x] 6.8 Expected build result after this group alone: `make fmt` applied,
      `make check` green (fmt-check covers the markdown), no code changes.

## 7. Manual verification on the live machine (after group 8)

- [x] 7.1 `cargo build --release --locked`; remove the old plugin
      registration and re-link (`herdr plugin link` — the manifest id is
      `alexeyco.review`, the action is `open`, the setup pane replaced the
      picker pane); rebind the keymap entry to `alexeyco.review.open`
      (Leader+Ctrl+R per the user's keymap); note the config file now
      lives at `~/.config/herdr/plugins/config/alexeyco.review/config.toml`
      and state under `~/.local/state/herdr/plugins/alexeyco.review/`.
- [x] 7.2 Direct-open happy path per tool (revdiff; tuicr — install via
      `brew install tuicr` if absent; lumen): set `tool = "<id>"` in
      `config.toml` → Leader+Ctrl+R opens the review pane directly with no
      dialog → dirty repo shows `reviewing uncommitted changes` → author an
      annotation → export (revdiff quit-with-annotations, tuicr session,
      lumen `s`→Enter) → the caller agent pane receives the prompt with
      header `Review annotations from <tool>:` and canonical blocks; `.out`
      removed after delivery.
- [x] 7.3 Per tool on a clean repo with an `origin` remote:
      `reviewing vs <base>` and the tool opens the base comparison; on a
      repo without a remote: `nothing to review` + hold. Lumen closed with
      Esc and no export closes as no-annotations; a review started from a
      pane without an agent keeps the `.out` and holds.
- [x] 7.4 Universal dialog, tool-unset case: remove the `tool` key → the
      action opens the setup popup showing the one universal layout —
      `Config file:` + the resolved `config.toml` path (the line reads
      `.../plugins/config/alexeyco.review/config.toml`), blank line,
      `Add one of these tools to config:` + the alphabetical rows
      `lumen`/`revdiff`/`tuicr` with dim inline brew install commands,
      blank line, `For more information:` +
      `https://github.com/alexeyco/herdr-review`, footer pinned at the
      bottom. Content fits the `height = 13` popup with no scrolling;
      top, left, and right padding visible; `Enter`, `q`, and `Esc` each
      close it and no review pane opens.
- [x] 7.5 Universal dialog, configured-missing and invalid cases:
      `tool = "tuicr"` with tuicr absent from PATH opens the identical
      layout with no case-status line; `tool = "diffy"` prints
      `herdr-review: warning:` naming `diffy` and opens the same layout.
- [x] 7.6 Universal dialog, nothing-installed case: temporarily move all
      three tool binaries off the daemon's PATH (restore afterwards) → the
      identical layout, no greyed state, no error, no hold; any key closes.
      (The unresolvable-config-path hint fallback is unit-tested —
      "Unresolvable config directory still renders" — not inducible on a
      healthy live install.)
- [x] 7.7 Placement matrix unchanged: `placement = "split"` +
      `direction = "down"` opens a split anchored at the caller pane;
      `placement = "popup"` warns and falls back to tab; corrupt
      `config.toml` warns, defaults, and lands in the setup dialog;
      split/zoomed without a caller pane degrades to tab with the warning.
- [x] 7.8 Expected final result: `make check` green and
      `openspec validate add-multi-tool-review` VALID.

## 8. Config-driven tool refactor (supersedes the picker flow of group 4)

Files: `src/config.rs`, `src/herdr.rs`, `src/review.rs`, `src/setup.rs`
(renamed from `src/picker.rs`), `src/main.rs`, `src/state.rs` (deleted),
`herdr-plugin.toml`, `tests/e2e.rs`, docs. The tasks are compile-coupled
through `src/review.rs`/`src/main.rs` — `make check` is green again at 8.8,
not between tasks. Superseded ticked tasks (4.2 state, 4.3 selection, 4.7
picker-open action, 4.8 picker flow, the picker legs of 4.5/4.6) stay
ticked as history; the replacements are listed here explicitly.

- [x] 8.1 `src/config.rs`: add the flat key `tool` — valid ids
      `revdiff`/`tuicr`/`lumen` taken as configured, unset/blank silently
      unset, any other value one `herdr-review: warning:` naming it and
      treated as unset; defaults keep `placement = "tab"`,
      `direction = "right"`, `tool` unset; update the popup-refusal warning
      constant to `placement "popup" is not supported for the review pane; using "tab"`
      (stale picker parenthetical dropped; refusal behavior
      unchanged). Unit tests cover: "Valid tool id is taken as configured",
      "Invalid tool value warns and counts as unset", "Blank tool value
      counts as unset", "Popup falls back to tab with a warning" (new
      text), plus the degradation legs of "No config directory available
      warns and uses defaults", "Missing file uses defaults", "Corrupt file
      warns and uses defaults" (tool-unset routing).
- [x] 8.2 `src/herdr.rs`: replace `picker_pane_open_args` with
      `setup_pane_open_args()` producing exactly `plugin pane open --plugin review --entrypoint setup --placement popup --focus` (no
      `--workspace`, no `--cwd`, no `--env`); `review_pane_open_args` keeps
      its argv and daemon targeting rules (4.16) but is now called by the
      action. Unit tests cover: "Setup-open argv carries no forwarding",
      and the surviving argv scenarios "Resolved directory is passed
      twice", "The tool env pair is always present", "Direction is passed
      only for split placement", "Split and zoomed pass the caller pane as
      target", "The workspace flag is passed only for tab placement", "The
      pane-open argv always carries its own placement", "Popup open
      succeeds without a pane id in the reply", "Popup reply body is
      ignored", "Reply without a usable pane id warns but succeeds",
      "Overlay-style reply without a tab".
- [x] 8.3 `src/review.rs`: rewrite `run_action` — parse context (errors
      unchanged), resolve caller identity, read config once, then branch:
      configured tool detected on PATH → degrade split/zoomed to tab when
      no caller pane (warning `placement "<p>" needs a caller pane; using "tab"`, logic
      moved from the deleted picker flow), open the review
      pane, log `herdr-review: opened review pane {id}[ in tab {tab}]`,
      best-effort `tab focus`; else open the setup popup and log
      `herdr-review: opened review tool setup`. Delete `run_picker`,
      `PickerEnv`, `PickerDeps`, `src/state.rs` and its `mod` line. Tests
      cover: "The action decides between the direct open and the setup
      dialog" via "Configured installed tool opens the review pane
      directly", "Unset tool opens the setup dialog", "Configured missing
      tool opens the setup dialog", "Invalid tool value opens the setup
      dialog after a warning", plus "Action success opens the popup and
      exits", "Pane opens with a tab", "Split or zoomed without a caller
      pane falls back to tab", "Pane id taken from the environment", "Agent
      without context entry is simply absent".
- [x] 8.4 `src/picker.rs` → `src/setup.rs`: drop selection, preselect,
      `✓`, movement keys, persistence, and the no-tools held error;
      `Enter`/`q`/`Esc` all close (exit 0, no calls, no writes); keep the
      stty raw-mode/restore guard, seams, frame chrome, footer pinning,
      and `src/theme.rs` palette; render the three branches — tool unset
      (config instructions naming the three valid ids + resolved
      `<config dir>/config.toml` path, or the `herdr plugin config-dir review` hint when no dir resolves), configured-missing (message
      naming the tool + its install command + config path, other rows
      greyed), nothing-installed (all rows greyed) — every row with its
      install-command line. `src/main.rs`: dispatch `setup` (was `picker`),
      error text `unknown plugin entrypoint <value>; expected review or setup`. `herdr-plugin.toml`: pane id `picker` → `setup`, title
      "Review tool setup", placement `popup` unchanged. Tests cover:
      "Catalog rows show install commands", "Missing tool renders greyed",
      "Non-executable file does not count", "Cancel exits cleanly",
      "Escape cancels", "Terminal is restored on every exit path", "Unset
      tool shows config instructions", "Config path points at the resolved
      file", "Unresolvable config directory still renders", "Configured
      missing tool is named with its install command", "Other rows grey out
      beside the missing configured tool", "All rows grey when nothing is
      installed", "Frame chrome carries no internal title", "Footer is
      pinned to the drawable bottom", "Frame colors follow the herdr theme
      palette", and from mode-dispatch "Only the entry-point variable is
      set", "Unknown entrypoint is an error", "Both entry points resolve to
      the same binary" (manifest pane id assertion updated).
- [x] 8.5 Wording sweep: `src/review.rs` tool-env errors become
      `HERDR_REVIEW_TOOL is not set; open the review pane through the review action`; caller-env comments drop the picker hop. Then
      `grep -rn "picker\|last.tool\|last_tool" src tests herdr-plugin.toml` must come up empty. Tests cover: "Missing tool id
      is an error", "Unknown tool id is an error", "Valid tool id selects
      the tool" (unchanged behavior, new message).
- [x] 8.6 `tests/e2e.rs`: action e2e writes a real `config.toml` under
      `HERDR_PLUGIN_CONFIG_DIR` — direct-open run (`tool = "revdiff"`, stub
      on PATH) asserting the review-pane argv per 8.2; setup-branch runs
      (no `tool`; `tool` naming a missing binary; `tool = "diffy"` with the
      warning assertion) asserting the exact setup argv and that no
      review-pane open is logged; delete the picker-flow and `last-tool`
      tests; pane-mode suites (revdiff/tuicr/lumen delivery, retention,
      scope) keep working with `HERDR_REVIEW_TOOL` set by the test env.
      The stub `herdr` keeps the daemon targeting rules from 4.16.
- [x] 8.7 Docs sync: `README.md` (flow = action → direct open or setup
      dialog; config table gains `tool`; drop picker/last-tool text;
      migration note gains `tool = "<id>"`), `docs/how-it-works.md` (branch
      decision + dialog contents), `AGENTS.md` (tri-mode dispatch with
      `setup`; action-decides gotcha; PATH-only detection unchanged; drop
      last-tool wording), `docs/development.md` (setup-dialog seams),
      `CHANGELOG.md` Unreleased wording (config-driven tool choice, no
      interactive picker).
- [x] 8.8 Expected build result after this group alone: `make check` green
      (unit suites for config `tool`, setup-dialog branches, action
      decision; e2e for both action branches and all three tools);
      `grep -rn "picker\|last-tool" src tests herdr-plugin.toml` empty;
      `openspec validate add-multi-tool-review` VALID; no version bumps.

## 9. Universal setup dialog (supersedes the group-8 branch renderer)

Files: `src/tool.rs`, `src/setup.rs`, `herdr-plugin.toml`, `tests/e2e.rs`
(string assertions only), docs. Tasks are sequentially coupled through the
catalog constants and the renderer — `make check` is green again at 9.6.
Ticked 8.4/8.7 stay as history of the interim build; replacements below.

- [x] 9.1 `src/tool.rs`: CATALOG install strings become the user-sketched
      brew commands — lumen `brew install jnsahaj/lumen/lumen`, revdiff
      `brew install umputun/apps/revdiff`, tuicr `brew install tuicr`;
      expose the alphabetical display order for dialog rows (lumen,
      revdiff, tuicr). Detection/resolution logic is unchanged
      (PATH-only, action routing and pane spawn); the pane's not-found
      error follows the constant. Update every unit/e2e assertion that
      greps `cargo install`. Tests cover: "No binary anywhere" (new hint
      text) and the catalog-side data of "Tool rows are alphabetical with
      dim inline install commands".
- [x] 9.2 `herdr-plugin.toml`: `[[panes]] id = "setup"` gains
      `width = "60%"` and `height = 13` (13-row line budget and the
      percentage-style evidence in design.md "Popup sizing"). The manifest
      unit test asserts the sizing keys alongside "Both entry points
      resolve to the same binary".
- [x] 9.3 `src/setup.rs`: delete the `Branch` enum, the per-branch
      instruction blocks, the row-state greying, and the dialog-side PATH
      detection and config-content read; the dialog resolves only the
      config directory for the path line (fallback: the
      `herdr plugin config-dir alexeyco.review` hint) and renders the one
      universal
      block exactly per the user sketch in design.md — `Config file:` +
      path, blank, `Add one of these tools to config:` + the three static
      alphabetical rows with the install command dim inline after the name
      on the same line (2-column gap), blank, `For more information:` +
      `https://github.com/alexeyco/herdr-review`, blank, pinned footer.
      Two-column left padding on every content line; the full-width
      panel-background fill (right padding), blank top line, stty
      machinery, theme palette, and close-on-any-key all stay. Tests
      cover: "Every entry case renders the same dialog", "Tool rows are
      alphabetical with dim inline install commands", "Repo URL line
      closes the content", "Config path points at the resolved file",
      "Unresolvable config directory still renders", "Content carries top,
      left, and right padding", "Content fits the declared popup height",
      "Cancel exits cleanly", "Escape cancels", "Terminal is restored on
      every exit path", "Frame chrome carries no internal title", "Footer
      is pinned to the drawable bottom", "Frame colors follow the herdr
      theme palette".
- [x] 9.4 `tests/e2e.rs`: the setup-branch runs keep asserting the exact
      setup argv (unchanged by this group); replace install-hint string
      assertions with the brew commands. No fixture or flow changes.
- [x] 9.5 Docs sync: `README.md` (dialog = one universal layout; install
      table brew-first, with the Linux alternative `cargo install <id>` in
      docs text only), `AGENTS.md` (setup gotcha: universal text, sizing
      keys, no dialog-side detection), `docs/how-it-works.md` (dialog
      contents), `CHANGELOG.md` Unreleased wording.
- [x] 9.6 Expected build result after this group alone: `make check`
      green; `grep -rn "Branch\|greyed\|rows_for_installed" src/setup.rs`
      empty; `openspec validate add-multi-tool-review` VALID; no version
      bumps.

- [x] 9.7 Acceptance fix round 3 (live eyeball finding): a long
      `Config file:` path wraps at 60% width and pushed the 13-row frame
      past the popup bottom. Raise the declared popup height to 15 (two
      extra rows as wrap margin; footer stays pinned last). Update:
      `herdr-plugin.toml` (`height = 15`), the manifest test assertion,
      the `content fits the declared popup height` unit test and the
      `src/setup.rs:299` budget comment, and every `height = 13` /
      thirteen-line-budget mention in docs (README if present,
      `docs/how-it-works.md`, `AGENTS.md`). Gates: `make check` green;
      `grep -rn "height = 13" src tests herdr-plugin.toml docs AGENTS.md
README.md` empty.

## 10. Plugin identity rename: `alexeyco.review` + action `open`

Files: `herdr-plugin.toml`, `src/herdr.rs`, `src/config.rs`,
`src/review.rs`, `src/setup.rs` (hint string), `tests/e2e.rs`,
`README.md`, `docs/how-it-works.md`, `docs/development.md`, `AGENTS.md`,
`CHANGELOG.md`. Sequential after group 9; same-wave files — `make check`
is green again at 10.6. Final user-confirmed identity: manifest
`id = "alexeyco.review"` (dots are legal in herdr ids — precedent
`ray.plugin-manager`), display `name = "Review"`, `[[actions]] id = "open"`
so the invoke command is `alexeyco.review.open`. Pane entrypoint ids
`review`/`setup`, all `HERDR_PLUGIN_ENTRYPOINT_ID` dispatch values, and
the error strings stay untouched; only the `--plugin` argv value, the
`config-dir` callback id, and herdr-owned paths change.

- [x] 10.1 `herdr-plugin.toml`: `id = "alexeyco.review"`, `name =
"Review"`, `[[actions]] id = "open"` (title/description kept);
      panes `review`/`setup` with their placement, title, sizing keys, and
      command arrays unchanged. The manifest unit test asserts the new ids
      alongside "Both entry points resolve to the same binary".
- [x] 10.2 `src/herdr.rs`: `base_pane_open_args` emits `--plugin
alexeyco.review`; `config_dir()` runs `plugin config-dir
alexeyco.review`. Update the argv-order tests — "Setup-open argv
      carries no forwarding", "Resolved directory is passed twice", "The
      tool env pair is always present", "Direction is passed only for
      split placement", "Split and zoomed pass the caller pane as target",
      "The workspace flag is passed only for tab placement", "The
      pane-open argv always carries its own placement" — and the
      config-dir parse tests behind "Environment directory wins" and "CLI
      fallback resolves the directory".
- [x] 10.3 `src/setup.rs` fallback hint and any `src/config.rs` warning
      text quoting the callback become `herdr plugin config-dir
alexeyco.review` (tests: "Unresolvable config directory still
      renders", "No config directory available warns and uses defaults");
      sweep `src/review.rs` log/error strings for plugin-id or
      action-id mentions.
- [x] 10.4 `tests/e2e.rs`: the stub `herdr` answers `plugin config-dir
alexeyco.review`; every pane-open argv assertion expects `--plugin
alexeyco.review` for both shapes (review and setup); action-mode
      fixtures unchanged (`HERDR_PLUGIN_ACTION_ID` values are not parsed).
- [x] 10.5 Docs sweep: `README.md` quickstart keybinding →
      `alexeyco.review.open` and the migration section rewritten (from
      the shipped `revdiff` 0.1.0: rebind `revdiff.review` →
      `alexeyco.review.open`, one re-link, config moves to
      `~/.config/herdr/plugins/config/alexeyco.review/` with unchanged
      `config.toml` keys, state to
      `~/.local/state/herdr/plugins/alexeyco.review/`; the interim
      unreleased `review` id gets one historical sentence);
      `docs/how-it-works.md` and `docs/development.md` id/command
      literals; `AGENTS.md` gotchas (manifest ids, config-dir callback,
      keybinding target, config/state paths); `CHANGELOG.md` Unreleased
      gains a second Breaking entry for the id/action rename. Historical
      CHANGELOG entries stay untouched.
- [x] 10.6 Gates: `make check` green;
      `grep -rnE 'plugin review([^.]|$)|config-dir review([^.]|$)|review\.review|id = "review"' src tests herdr-plugin.toml README.md docs AGENTS.md`
      empty (historical `CHANGELOG.md` entries excluded; the only
      legitimate `review.review`-era strings live in the change's own
      openspec artifacts); then `grep -rn '"review"' src` reviewed
      case-by-case — every remaining hit is a pane/entrypoint id
      (`--entrypoint` argv element, dispatch match arm, pane title
      assertion), never a `--plugin` or `config-dir` argument;
      `openspec validate add-multi-tool-review` VALID; no version bumps.

## 11. Focus return to the caller tab (user-requested addition)

Files: `src/herdr.rs` (forwarding + focus call), `src/review.rs`
(action env, pane-mode clean-exit legs), `tests/e2e.rs`, docs. User
ruling: return focus ONLY after clean review-pane endings (delivered,
no-annotations); hold/error paths and the setup popup are untouched;
the call is keyed on the forwarded caller tab — for split/overlay/zoomed
the plugin sits in that same tab and the focus call is a semantic no-op.

- [x] 11.1 Action forwards the resolved context tab id as
      `--env HERDR_REVIEWS_CALLER_TAB=<tab>` on the review-pane open only
      (the setup popup still receives no env); absent/blank tab id ⇒ pair
      omitted. Tests: pair present in the direct-open argv with the
      resolved id; omitted when context carries no tab; setup argv
      unchanged. Covers: "Caller tab pair is present on the direct open",
      "Caller tab pair is omitted when no tab resolves", "Setup popup never
      carries the caller tab".
- [x] 11.2 Pane mode on clean endings only — delivered successfully and
      no-annotations cleanup — after the pane close best-effort calls
      `herdr tab focus <HERDR_REVIEWS_CALLER_TAB>` when that env
      resolves; missing env, spawn failure, or nonzero exit ⇒ one warning
      line via the existing warning sink, outcome unchanged. Hold/error
      paths never call it. Tests: focus after delivery with the forwarded
      id; focus after no-annotations; not called on the hold path; focus
      failure warns but exit stays 0. Covers: "Delivered run refocuses the
      caller tab", "No-annotations run refocuses the caller tab", "Hold and
      error paths never refocus", "Focus failure warns without changing the
      outcome", "Missing caller tab is silently skipped".
- [x] 11.3 `tests/e2e.rs`: action fixture asserts the caller-tab pair in
      the review-pane argv; delivery e2e asserts stub-log order
      `plugin pane close` → `tab focus <caller-tab>`; no-annotations run
      the same; hold path asserts absence.
- [x] 11.4 Docs: README flow line, `docs/how-it-works.md` close sequence,
      `AGENTS.md` gotcha — focus returns to the caller's tab after a
      clean review close, best-effort.
- [x] 11.5 Gates: `make check` green; `openspec validate
add-multi-tool-review` VALID zero warnings; no version bumps.

- [x] 11.6 Acceptance fix round 4 (live herdr 0.9.5 finding): a pane
      process closing its OWN pane is killed by the daemon at that call,
      so the post-close `tab focus` never ran — focus never returned.
      Swap the clean-ending sequence to `tab focus` BEFORE the pane close
      (live CLI probe: focus t4 then close, focus sticks). Update:
      `src/review.rs` `finish()` ordering (both clean legs), the call-
      order assertions in the four focus tests ("Delivered run refocuses
      the caller tab", "No-annotations run refocuses the caller tab",
      focus-failure + e2e `assert_close_then_focus` helper renamed/
      inverted), and the docs wording (`AGENTS.md` bullet,
      `docs/how-it-works.md` close sequence). Gates: `make check` green;
      validate VALID.

## 12. Release preparation (maintainer decision 2026-10-18: version 0.2.0, no date in CHANGELOG)

Files: `Cargo.toml`, `herdr-plugin.toml`, `Cargo.lock`, `CHANGELOG.md`.

- [x] 12.1 Bump `version` to `0.2.0` in `Cargo.toml` and
      `herdr-plugin.toml` (both, identically — the CI version-sync job
      and the manifest unit test enforce equality); regenerate
      `Cargo.lock`.
- [x] 12.2 `CHANGELOG.md`: rename the `Unreleased` section to
      `## 0.2.0` with NO date (maintainer's call), leaving its content
      and the `0.1.0` history untouched.
- [x] 12.3 Gates: `make check` green (version-sync logic, manifest test,
      prettier); `grep -n "version" Cargo.toml herdr-plugin.toml` shows
      `0.2.0` in both; no git writes (tag/PR stay the maintainer's).
