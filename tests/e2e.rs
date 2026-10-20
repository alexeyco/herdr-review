//! End-to-end tests for the tri-mode plugin binary.
//!
//! Each test spawns the real binary (`CARGO_BIN_EXE_herdr-review`) with a
//! hermetic environment, a stub `herdr` callback, and stub review tools on
//! `PATH` (no per-tool override variable — tools resolve PATH-only). The
//! stubs append their argv to `$STUB_LOG`; assertions grep that log. The
//! `revdiff` stub writes annotations to its `--output` file; the `tuicr` stub
//! answers the post-exit adapter commands (`review comments`, `review list`)
//! from fixture JSON; the `lumen` stub writes its export fixture to the piped
//! stdout. Knobs (`REV_*`, `TUICR_*`, `LUMEN_*`) pick fixtures and exit codes
//! per run. The stub
//! `herdr` answers `plugin config-dir alexeyco.review` and both pane-open
//! shapes: the setup popup (no pane id in the reply) and the review pane
//! (canned pane/tab ids) — and it mirrors the herdr 0.9.5 daemon's targeting
//! contract: any `--workspace` on an `overlay`/`popup` open, or a
//! `split`/`zoomed` open without `--target-pane`, exits 1 with the daemon's
//! `invalid_params` body. Action runs write a real `config.toml` under
//! `HERDR_PLUGIN_CONFIG_DIR` to exercise placement, direction, and ignored
//! obsolete tool settings. Setup-dialog runs use piped input and the
//! non-interactive text fallback. Probe-failure tests
//! run with a `git`-less `PATH` or a stub `git` that fails one probe
//! command. Everything runs in a unique temp directory and needs no network
//! or global git config.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Stub `herdr`: logs every call, answers `plugin pane open` per entrypoint
/// (setup popup replies without a pane id; review pane replies with canned
/// ids) and `plugin config-dir alexeyco.review` with a bare path line. The
/// pane-open path mirrors the herdr 0.9.5 daemon's targeting contract
/// (live-probed): any `--workspace` on an `overlay`/`popup` open, or a
/// `split`/`zoomed` open without `--target-pane`, exits 1 with the daemon's
/// `invalid_params` body. Fails the `agent prompt` call when
/// `HERDR_STUB_FAIL_PROMPT=1` (the delivery-failure knob) and the
/// `tab focus` call when `HERDR_STUB_FAIL_FOCUS=1` (the focus-return
/// failure knob). Otherwise exits 0.
const HERDR_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$STUB_LOG"
if [ "$1" = "agent" ] && [ "$2" = "prompt" ] && [ "${HERDR_STUB_FAIL_PROMPT:-0}" = "1" ]; then
  echo 'stub agent prompt failure' >&2
  exit 1
fi
if [ "$1" = "tab" ] && [ "$2" = "focus" ] && [ "${HERDR_STUB_FAIL_FOCUS:-0}" = "1" ]; then
  echo 'stub tab focus failure' >&2
  exit 1
fi
if [ "$1" = "plugin" ] && [ "$2" = "pane" ] && [ "$3" = "open" ]; then
  # Mirror the daemon rules: overlay/popup reject --workspace; split/zoomed
  # require --target-pane.
  placement=""
  has_workspace=0
  has_target=0
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--placement" ]; then
      placement="$arg"
    fi
    case "$arg" in
      --workspace) has_workspace=1 ;;
      --target-pane) has_target=1 ;;
    esac
    prev="$arg"
  done
  if [ "$has_workspace" = "1" ] && { [ "$placement" = "overlay" ] || [ "$placement" = "popup" ]; }; then
    printf '%s\n' '{"error":{"code":"invalid_params","message":"overlay and popup plugin panes target the active pane"}}'
    exit 1
  fi
  if { [ "$placement" = "split" ] || [ "$placement" = "zoomed" ]; } && [ "$has_target" = "0" ]; then
    printf '%s\n' '{"error":{"code":"invalid_params","message":"split and zoomed plugin panes target an existing pane; use target_pane_id"}}'
    exit 1
  fi
  # $7 is the --entrypoint value: plugin pane open --plugin
  # alexeyco.review --entrypoint <ep> --placement <p> ...
  if [ "$7" = "setup" ]; then
    printf '%s\n' '{"id":"cli:plugin","result":{"type":"ok"}}'
  elif [ "$7" = "picker" ]; then
    printf '%s\n' '{"id":"cli:plugin","result":{"type":"ok"}}'
  else
    printf '%s\n' '{"result":{"plugin_pane":{"pane":{"pane_id":"pane-stub","tab_id":"tab-stub"}}}}'
  fi
elif [ "$1" = "plugin" ] && [ "$2" = "config-dir" ] && [ "$3" = "alexeyco.review" ]; then
  if [ "${HERDR_STUB_FAIL_CONFIG_DIR:-0}" = "1" ]; then
    echo 'stub config-dir failure' >&2
    exit 1
  fi
  printf '%s\n' "${STUB_CONFIG_DIR:-/nonexistent-stub-config}"
fi
exit 0
"#;

/// Stub `revdiff`: logs `REVDIFF <argv>`, finds the `--output` path and (unless
/// `REV_EMPTY=1`) writes annotations to it, then exits `$REV_RC` (default 0).
/// `REV_BIG=1` writes ~100000 bytes instead.
const REVDIFF_STUB: &str = r#"#!/bin/sh
printf 'REVDIFF %s\n' "$*" >> "$STUB_LOG"
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "--output" ]; then
    out="$arg"
  fi
  prev="$arg"
done
if [ "${REV_EMPTY:-0}" = "1" ]; then
  exit "${REV_RC:-0}"
fi
if [ "${REV_BIG:-0}" = "1" ]; then
  head -c 100000 /dev/zero | tr '\0' 'a' > "$out"
  exit "${REV_RC:-0}"
fi
printf '## src/main.rs:1 (+)\nbody line\n' > "$out"
exit "${REV_RC:-0}"
"#;

/// Stub `tuicr`: logs `TUICR <argv>` and answers the post-exit adapter
/// commands from fixtures — `review list --repo .` with one session updated
/// in 2100 (newer than any launch timestamp, so the fallback always picks
/// it) and `review comments --session <slug>` per `TUICR_COMMENTS=one|empty|
/// invalid`. `TUICR_SESSION_MISS=1` fails the comments call for every slug
/// but the fixture session's, forcing the list fallback. The TUI invocation
/// itself (argv without a `review` subcommand) exits `$TUICR_RC` (default 0);
/// no `--json` flag exists in tuicr 0.27.0 and none is passed.
const TUICR_STUB: &str = r#"#!/bin/sh
printf 'TUICR %s\n' "$*" >> "$STUB_LOG"
if [ "$1" = "review" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[{"slug":"stub-owner/stub-repo@stub-branch/worktree/abc1234","comment_count":2,"updated_at":"2100-01-01T00:00:00Z"}]'
  exit 0
fi
if [ "$1" = "review" ] && [ "$2" = "comments" ]; then
  # $4 is the slug: review comments --session <slug>
  if [ "${TUICR_SESSION_MISS:-0}" = "1" ] && [ "$4" != "stub-owner/stub-repo@stub-branch/worktree/abc1234" ]; then
    echo "stub: session '$4' was not found" >&2
    exit 1
  fi
  case "${TUICR_COMMENTS:-one}" in
    one)
      printf '%s\n' '[{"path":"src/main.rs","start_line":12,"end_line":12,"side":"new","content":"Handle the empty case here."},{"path":"src/lib.rs","start_line":3,"side":"old","content":"Drop the stale clone."}]'
      ;;
    empty)
      printf '[]\n'
      ;;
    invalid)
      printf '%s\n' '{not json'
      ;;
  esac
  exit 0
fi
exit "${TUICR_RC:-0}"
"#;

/// Stub `lumen`: logs `LUMEN <argv>` and writes the export fixture — a
/// `# origin/main` ref header plus RIGHT/LEFT/range/file-level blocks with
/// `---` separators — to stdout per `LUMEN_OUT=export|blank|garbage|probes`,
/// then exits `$LUMEN_RC` (default 0). The pane pipes lumen's stdout, so
/// writing to stdout is exactly the export channel the plugin captures.
/// `probes` writes only the live-observed terminal-probe preamble (OSC
/// `ESC]11;?BEL` + CSI `ESC[5n`) — the closed-without-exporting capture.
const LUMEN_STUB: &str = r#"#!/bin/sh
printf 'LUMEN %s\n' "$*" >> "$STUB_LOG"
case "${LUMEN_OUT:-export}" in
  export)
    cat <<'EOF'
# origin/main
**src/main.rs** line 2 (RIGHT)

spike annotation 2
---
**src/old.rs** line 5 (LEFT)

stale import
---
**src/lib.rs** lines 4-9 (LEFT)

simplify this range
---
**README.md**

file-level note
EOF
    ;;
  blank)
    ;;
  garbage)
    printf 'garbage capture\n'
    ;;
  probes)
    printf '\033]11;?\a\033[5n'
    ;;
esac
exit "${LUMEN_RC:-0}"
"#;

/// Stub `git` for the unexpected-exit-code probe test: `diff --quiet` exits 2
/// (an undocumented quiet-diff code); everything else delegates to the real
/// git resolved once at test start and baked in via `$REAL_GIT`.
const GIT_DIFF_FAIL_STUB: &str = r#"#!/bin/sh
if [ "$1" = "diff" ] && [ "$2" = "--quiet" ]; then
  exit 2
fi
exec "$REAL_GIT" "$@"
"#;

/// Absolute path of the real `git` used by [`GIT_DIFF_FAIL_STUB`].
fn real_git_path() -> String {
    let out = Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .unwrap();
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!path.is_empty(), "cannot resolve the real git binary");
    path
}

/// A unique scratch directory removed (best-effort) when the test ends.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("herdr-review-e2e-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// Writes the `herdr` and review-tool stubs (`revdiff`, `tuicr`, `lumen`)
    /// into a stub bin directory and returns `(herdr, log)`. The directory
    /// goes on `PATH` (see [`Fixture::path_with_stubs`]) so the review pane
    /// resolves the stub tools PATH-only.
    fn stubs(&self) -> (PathBuf, PathBuf) {
        let bin = self.subdir("bin");
        write_executable(&bin.join("herdr"), HERDR_STUB);
        write_executable(&bin.join("revdiff"), REVDIFF_STUB);
        write_executable(&bin.join("tuicr"), TUICR_STUB);
        write_executable(&bin.join("lumen"), LUMEN_STUB);
        (bin.join("herdr"), self.dir.join("stub.log"))
    }

    /// A `PATH` string with the stub bin directory first, then the system
    /// `PATH` (for git and sh).
    fn path_with_stubs(&self) -> String {
        format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn path_with_only_tool(&self, tool: &str) -> String {
        let bin = self.dir.join("bin");
        for candidate in ["revdiff", "tuicr", "lumen"] {
            if candidate != tool {
                let _ = fs::remove_file(bin.join(candidate));
            }
        }
        bin.to_string_lossy().into_owned()
    }

    fn path_without_tools(&self) -> String {
        let bin = self.dir.join("bin");
        for candidate in ["revdiff", "tuicr", "lumen"] {
            let _ = fs::remove_file(bin.join(candidate));
        }
        bin.to_string_lossy().into_owned()
    }

    /// Creates a subdirectory and returns its path.
    fn subdir(&self, name: &str) -> PathBuf {
        let p = self.dir.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Builds the plugin command with a cleared environment plus PATH/HOME/git
/// isolation and the caller-supplied env pairs. Stdio is the caller's choice.
fn plugin_command(envs: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-review"));
    cmd.env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("HOME", std::env::temp_dir());
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.env("GIT_CONFIG_SYSTEM", "/dev/null");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd
}

/// Spawns the plugin with null `stdin` so the hold paths return at once
/// instead of hanging.
fn run_plugin(envs: &[(&str, &str)]) -> Output {
    plugin_command(envs)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

/// Spawns the plugin with piped `stdin` carrying `keys` (the setup-dialog key
/// loop reads it; with the stub `stty` on `PATH` no real terminal is
/// touched), then waits for it to finish.
fn run_plugin_with_stdin(envs: &[(&str, &str)], keys: &[u8]) -> Output {
    use std::io::Write;
    let mut child = plugin_command(envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(keys).unwrap();
    }
    child.wait_with_output().unwrap()
}

/// Runs action mode with one hermetically installed review tool and the given
/// config source. `via_env` selects `HERDR_PLUGIN_CONFIG_DIR`; otherwise the
/// stub CLI resolves `config_dir` or fails when `fail_config_dir` is true.
fn run_action_config_case(
    fx: &Fixture,
    content: Option<&str>,
    via_env: bool,
    fail_config_dir: bool,
) -> (Output, String, PathBuf) {
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    if let Some(content) = content {
        fs::write(config_dir.join("config.toml"), content).unwrap();
    }
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );
    let mut env = vec![
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", ctx.as_str()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", path.as_str()),
        ("STUB_LOG", log.to_str().unwrap()),
        ("STUB_CONFIG_DIR", config_dir.to_str().unwrap()),
        (
            "HERDR_STUB_FAIL_CONFIG_DIR",
            if fail_config_dir { "1" } else { "0" },
        ),
    ];
    if via_env {
        env.push(("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()));
    }
    let out = run_plugin(&env);
    (out, read_log(&log), repo)
}

fn action_review_open(log: &str) -> &str {
    log.lines()
        .find(|line| line.starts_with("plugin pane open") && line.contains("--entrypoint review"))
        .unwrap_or("")
}

/// Runs git in `dir` with global config disabled and an inline identity where
/// needed. Panics with git's stderr on failure.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(dir: &Path, message: &str) {
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

/// Initializes a repository with two commits and a clean tree, but no remote,
/// so the probe finds no base.
fn clean_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(dir, &["add", "a.txt"]);
    commit(dir, "base");
    fs::write(dir.join("a.txt"), "b\n").unwrap();
    git(dir, &["add", "a.txt"]);
    commit(dir, "second");
}

/// A clean repo whose `origin` remote has an `origin/main` ref one commit
/// behind HEAD, so the probe selects `VsBase(origin/main)` and revdiff gets
/// the two positional refs.
fn clean_repo_with_origin_main(dir: &Path) {
    clean_repo(dir);
    git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://example.com/herdr-review.git",
        ],
    );
    git(dir, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `*.out` files directly under `dir`.
fn out_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("out"))
        .collect()
}

/// The `REVDIFF ...` log line, if any.
fn revdiff_line(log: &str) -> &str {
    log.lines()
        .find(|l| l.starts_with("REVDIFF "))
        .unwrap_or("")
}

/// The tuicr TUI invocation's `TUICR ...` log line — the one that is not a
/// `review` adapter command — if any.
fn tuicr_tui_line(log: &str) -> &str {
    log.lines()
        .find(|l| l.starts_with("TUICR ") && !l.starts_with("TUICR review "))
        .unwrap_or("")
}

/// The `LUMEN ...` log line, if any.
fn lumen_line(log: &str) -> &str {
    log.lines().find(|l| l.starts_with("LUMEN ")).unwrap_or("")
}

/// Asserts the focus-return ordering on the stub log: the `tab focus <tab>`
/// call precedes the `plugin pane close` attempt.
fn assert_focus_then_close(log: &str, focus_line: &str, close_line: &str) {
    let focus = log.lines().position(|l| l == focus_line);
    let close = log.lines().position(|l| l == close_line);
    assert!(focus.is_some(), "missing {focus_line:?} in {log}");
    assert!(close.is_some(), "missing {close_line:?} in {log}");
    assert!(
        focus.unwrap() < close.unwrap(),
        "{focus_line:?} must precede {close_line:?} in {log}"
    );
}

/// Review-pane-mode env pairs shared by the pane tests: entrypoint, the
/// selected tool, repo, state, pane/tab ids, the caller tab the clean
/// endings refocus, the stubs on `PATH`, and the stub log.
fn pane_env<'a>(
    repo: &'a Path,
    state: &'a Path,
    herdr: &'a Path,
    log: &'a Path,
    path: &'a str,
) -> Vec<(&'a str, &'a str)> {
    pane_env_for("revdiff", repo, state, herdr, log, path)
}

/// [`pane_env`] with the selected review tool id — the pane dispatches on
/// `HERDR_REVIEW_TOOL`, so the tuicr and lumen suites reuse the same base
/// environment with a different tool.
fn pane_env_for<'a>(
    tool: &'a str,
    repo: &'a Path,
    state: &'a Path,
    herdr: &'a Path,
    log: &'a Path,
    path: &'a str,
) -> Vec<(&'a str, &'a str)> {
    vec![
        ("HERDR_PLUGIN_ENTRYPOINT_ID", "review"),
        ("HERDR_REVIEW_TOOL", tool),
        ("HERDR_REVIEWS_DIR", repo.to_str().unwrap()),
        ("HERDR_PLUGIN_STATE_DIR", state.to_str().unwrap()),
        ("HERDR_PANE_ID", "pane-7"),
        ("HERDR_TAB_ID", "tab-7"),
        ("HERDR_REVIEWS_CALLER_TAB", "w3X:t9"),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]
}

#[test]
fn pane_rc10_delivers_annotations_and_closes_pane() {
    let fx = Fixture::new("rc10-deliver");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_RC", "10"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing vs origin/main"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert!(
        logged.contains("agent prompt caller-1 Review annotations from revdiff:"),
        "{logged}"
    );
    // revdiff output passes through unchanged: the delivered prompt carries
    // the stub's canonical text verbatim.
    assert!(
        logged.contains("## src/main.rs:1 (+)\nbody line"),
        "{logged}"
    );
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    // Scenario "Delivered run refocuses the caller tab": the focus return
    // runs before the pane close attempt, with the forwarded caller tab.
    assert_focus_then_close(&logged, "tab focus w3X:t9", "plugin pane close pane-7");
    // Two positional refs at the end of the argv, never a `..` range argument.
    assert!(
        revdiff_line(&logged).ends_with("origin/main HEAD"),
        "{}",
        logged
    );
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_rc0_with_output_delivers_annotations() {
    let fx = Fixture::new("rc0-deliver");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    assert!(
        logged.contains("agent prompt caller-1 Review annotations from revdiff:"),
        "{logged}"
    );
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_rc0_empty_is_no_annotations() {
    let fx = Fixture::new("rc0-empty");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_EMPTY", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: no annotations"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    // Scenario "No-annotations run refocuses the caller tab": the same
    // focus-then-close sequence runs on the quiet cleanup ending.
    assert_focus_then_close(&logged, "tab focus w3X:t9", "plugin pane close pane-7");
    assert!(out_files(&state.join("reviews")).is_empty());
    assert!(
        stderr(&out).contains("warning: cannot read"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn pane_focus_failure_warns_but_exits_clean() {
    // Scenario "Focus failure warns without changing the outcome": the
    // `tab focus` call fails after a delivered run, exactly one
    // `herdr-review: warning:` line names it, the exit code stays 0, and the
    // output file was already removed.
    let fx = Fixture::new("focus-fail");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_RC", "10"),
        ("HERDR_STUB_FAIL_FOCUS", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: warning: failed to focus caller tab w3X:t9"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(
        stderr(&out).matches("herdr-review: warning:").count(),
        1,
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: sent review annotations to agent pane caller-1"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    // The focus call still ran before the pane close, and failed loudly.
    assert_focus_then_close(&logged, "tab focus w3X:t9", "plugin pane close pane-7");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_clean_delivery_without_caller_tab_skips_focus_silently() {
    // Scenario "Missing caller tab is silently skipped": a clean delivered
    // ending with `HERDR_REVIEWS_CALLER_TAB` unset makes no focus call and
    // prints no warning.
    let fx = Fixture::new("focus-skip");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let env: Vec<(&str, &str)> = pane_env(&repo, &state, &herdr, &log, &path)
        .into_iter()
        .filter(|(k, _)| *k != "HERDR_REVIEWS_CALLER_TAB")
        .chain([
            ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
            ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ])
        .collect();
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: sent review annotations to agent pane caller-1"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("herdr-review: warning:"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("tab focus"), "{logged}");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_delivery_failure_keeps_output_and_holds() {
    // Scenario "Failed delivery keeps the file and holds the pane": the
    // agent prompt call fails, so the error names the target and the kept
    // output path, the file stays, the pane is held, and the process exits 1.
    let fx = Fixture::new("deliver-fail");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_RC", "10"),
        ("HERDR_STUB_FAIL_PROMPT", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("failed to send annotations to agent pane caller-1"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains(&format!("output kept at {}/reviews/", state.display())),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(out_files(&state.join("reviews")).len(), 1);
    let logged = read_log(&log);
    assert!(logged.contains("agent prompt caller-1"), "{logged}");
    // The teardown never ran: no pane close after a failed delivery, and —
    // scenario "Hold and error paths never refocus" — no focus call either.
    assert!(!logged.contains("plugin pane close"), "{logged}");
    assert!(!logged.contains("tab focus"), "{logged}");
}

#[test]
fn pane_rc1_keeps_output_and_fails() {
    let fx = Fixture::new("rc1-fail");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_RC", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("revdiff failed with exit code 1"),
        "stderr: {}",
        stderr(&out)
    );
    // A tool child's exit code never leaks as the binary's exit status.
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    // Scenario "Hold and error paths never refocus": Failed(rc) keeps focus
    // wherever the closed pane left it.
    assert!(!logged.contains("tab focus"), "{logged}");
    assert_eq!(out_files(&state.join("reviews")).len(), 1);
}

#[test]
fn pane_rc10_without_caller_keeps_output() {
    let fx = Fixture::new("rc10-noagent");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.push(("REV_RC", "10"));
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("no agent in focused pane; annotations kept at"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    // Scenario "Hold and error paths never refocus": the NoAgent hold never
    // focuses the caller tab.
    assert!(!logged.contains("tab focus"), "{logged}");
    assert_eq!(out_files(&state.join("reviews")).len(), 1);
}

#[test]
fn pane_rc10_missing_output_is_error() {
    let fx = Fixture::new("rc10-missing");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_EMPTY", "1"),
        ("REV_RC", "10"),
    ]);
    let out = run_plugin(&env);

    assert_ne!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot read revdiff output"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
}

#[test]
fn pane_untracked_repo_passes_untracked() {
    let fx = Fixture::new("untracked");
    let repo = fx.subdir("repo");
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("base.txt"), "b\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    commit(&repo, "base");
    fs::write(repo.join("new.txt"), "n\n").unwrap();
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.push(("REV_EMPTY", "1"));
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing uncommitted changes"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert!(revdiff_line(&logged).contains("--untracked"), "{logged}");
}

#[test]
fn pane_clean_repo_without_remote_holds_nothing_to_review() {
    let fx = Fixture::new("nothing-to-review");
    let repo = fx.subdir("repo");
    clean_repo(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let env = pane_env(&repo, &state, &herdr, &log, &path);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: nothing to review"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("REVDIFF"), "{logged}");
    // Nothing to review exits before any state-dir work.
    assert!(!state.join("reviews").exists());
}

#[test]
fn pane_git_spawn_failure_holds_nothing_to_review() {
    let fx = Fixture::new("git-spawn-fail");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let no_git = fx.subdir("no-git-path");
    let path = no_git.to_str().unwrap().to_string();

    let env = pane_env(&repo, &state, &herdr, &log, &path);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("herdr-review: git probe failed (git rev-parse --is-inside-work-tree):"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("; nothing to review"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("herdr-review: nothing to review"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("REVDIFF"), "{logged}");
    assert!(!state.join("reviews").exists());
}

#[test]
fn pane_unexpected_diff_exit_code_holds_nothing_to_review() {
    let fx = Fixture::new("git-diff-rc2");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let git_stub = fx.dir.join("git");
    write_executable(
        &git_stub,
        &GIT_DIFF_FAIL_STUB.replace("$REAL_GIT", &real_git_path()),
    );

    // The stub git shadows the real one via PATH order.
    let path = format!("{}:{}", fx.dir.display(), path);
    let env = pane_env(&repo, &state, &herdr, &log, &path);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: git probe failed (git diff --quiet): exit status: 2"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("; nothing to review"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("herdr-review: nothing to review"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("REVDIFF"), "{logged}");
    assert!(!state.join("reviews").exists());
}

#[test]
fn pane_missing_herdr_bin_fails_before_any_probe() {
    let fx = Fixture::new("no-herdr-bin");
    let log = fx.dir.join("stub.log");
    // A PATH without git: had the pre-check not fired first, the probe would
    // fail loudly here instead.
    let no_git = fx.subdir("no-git-path");

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ENTRYPOINT_ID", "review"),
        ("HERDR_REVIEW_TOOL", "revdiff"),
        ("STUB_LOG", log.to_str().unwrap()),
        ("PATH", no_git.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "herdr did not provide HERDR_BIN_PATH (plugin API v1 contract); cannot call back into herdr"
        ),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("git probe failed"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(stdout(&out).is_empty(), "stdout: {}", stdout(&out));
    assert!(read_log(&log).is_empty(), "{}", read_log(&log));
}

#[test]
fn pane_missing_review_tool_env_is_an_error() {
    // Scenario "Missing tool id is an error": the failure precedes any git
    // probe or spawn, and the pane-mode error policy holds the pane.
    let fx = Fixture::new("no-tool-env");
    let repo = fx.subdir("repo");
    clean_repo(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let env: Vec<(&str, &str)> = pane_env(&repo, &state, &herdr, &log, &path)
        .into_iter()
        .filter(|(k, _)| *k != "HERDR_REVIEW_TOOL")
        .collect();
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "HERDR_REVIEW_TOOL is not set; open the review pane through the review action"
        ),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("REVDIFF"), "{logged}");
    assert!(stdout(&out).is_empty(), "stdout: {}", stdout(&out));
}

#[test]
fn pane_unknown_review_tool_env_is_an_error() {
    // Scenario "Unknown tool id is an error".
    let fx = Fixture::new("unknown-tool");
    let repo = fx.subdir("repo");
    clean_repo(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.push(("HERDR_REVIEW_TOOL", "diffy"));
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("unknown review tool diffy; expected revdiff, tuicr or lumen"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("REVDIFF"), "{logged}");
}

#[test]
fn pane_rc10_big_output_is_capped() {
    let fx = Fixture::new("big-cap");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env(&repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("REV_RC", "10"),
        ("REV_BIG", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    assert!(
        logged.len() < 100_000,
        "delivered log grew to {}",
        logged.len()
    );
    let note = format!(
        "(output truncated; full annotations kept at {}/",
        state.join("reviews").display()
    );
    assert!(
        logged.contains(&note),
        "missing truncation note in {logged}"
    );
    assert!(logged.contains(".out)"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

// --- tuicr pane runs ------------------------------------------------------

#[test]
fn pane_tuicr_delivers_converted_comments() {
    // Scenarios "tuicr with comments delivers" and "Predicted slug answers
    // directly": the stub answers the predicted-slug call with two comments
    // (lowercase new/old sides, as tuicr 0.27 emits them), no `review list`
    // call runs, the delivered prompt carries the tuicr header and canonical
    // blocks, and the `.out` file is removed after delivery. Clean fixture
    // repo: the TUI argv is the base range fragment, never a `--json` flag.
    let fx = Fixture::new("tuicr-deliver");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing vs origin/main"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: sent review annotations to agent pane caller-1"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    // The delivered prompt: the tuicr header plus the converted canonical
    // blocks, exactly as the converter maps them.
    assert!(
        logged.contains(
            "agent prompt caller-1 Review annotations from tuicr:\n\n\
             ## src/main.rs:12 (+)\nHandle the empty case here.\n\n\
             ## src/lib.rs:3 (-)\nDrop the stale clone."
        ),
        "{logged}"
    );
    // The predicted slug answered directly: no list call, no --json flag
    // (tuicr has none).
    assert!(!logged.contains("review list"), "{logged}");
    assert!(!logged.contains("--json"), "{logged}");
    assert_eq!(tuicr_tui_line(&logged), "TUICR -r origin/main..HEAD");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_tuicr_list_fallback_delivers() {
    // Scenario "Fallback list picks the session created by this run": the
    // predicted-slug call fails (`TUICR_SESSION_MISS=1`), `review list
    // --repo .` answers a fixture session updated in 2100 — newer than any
    // launch timestamp — and its comments are fetched and delivered.
    let fx = Fixture::new("tuicr-fallback");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("TUICR_SESSION_MISS", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    assert!(logged.contains("TUICR review list --repo ."), "{logged}");
    // Two comments calls: the missed predicted slug, then the fixture
    // session's slug taken from the list.
    let comments_calls = logged
        .lines()
        .filter(|l| l.starts_with("TUICR review comments"))
        .count();
    assert_eq!(comments_calls, 2, "{logged}");
    assert!(
        logged.contains(
            "TUICR review comments --session stub-owner/stub-repo@stub-branch/worktree/abc1234"
        ),
        "{logged}"
    );
    assert!(
        logged.contains("agent prompt caller-1 Review annotations from tuicr:"),
        "{logged}"
    );
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_tuicr_empty_comments_is_no_annotations() {
    // Scenario "No comments is a clean no-annotations": the adapter yields
    // an empty comment list, so `herdr-review: no annotations` is printed,
    // the `.out` file is removed, and no prompt is delivered.
    let fx = Fixture::new("tuicr-empty");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("TUICR_COMMENTS", "empty"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: no annotations"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_tuicr_rc1_keeps_output_and_fails() {
    // Scenarios "Nonzero tuicr exit skips the adapter" and "Failed run
    // surfaces the code and keeps evidence": no adapter command runs, the
    // empty `.out` file is kept, the pane is held, and the process exits 1.
    let fx = Fixture::new("tuicr-rc1");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("TUICR_RC", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("tuicr failed with exit code 1"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains(&format!("output kept at {}/reviews/", state.display())),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    // The adapter never ran: no `review` subcommand reached the stub.
    let logged = read_log(&log);
    assert!(!logged.contains("TUICR review"), "{logged}");
    assert!(!logged.contains("agent prompt"), "{logged}");
    let files = out_files(&state.join("reviews"));
    assert_eq!(files.len(), 1);
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), "");
}

#[test]
fn pane_tuicr_invalid_comments_is_adapter_failure() {
    // Scenario "Adapter failure keeps evidence": invalid comments JSON fails
    // both the predicted-slug call and the fallback fetch, so the run fails
    // with `tuicr session lookup failed:`, keeps the `.out` file, and holds.
    let fx = Fixture::new("tuicr-invalid");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("TUICR_COMMENTS", "invalid"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("tuicr session lookup failed:"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("invalid tuicr comments JSON"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains(&format!("output kept at {}/reviews/", state.display())),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    assert_eq!(out_files(&state.join("reviews")).len(), 1);
}

#[test]
fn pane_tuicr_dirty_repo_passes_w() {
    // Scenario "tuicr runs on the inherited tty", argv side: a dirty tree
    // spawns the TUI with `-w` alone, and the comments still deliver.
    let fx = Fixture::new("tuicr-dirty");
    let repo = fx.subdir("repo");
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("base.txt"), "b\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    commit(&repo, "base");
    fs::write(repo.join("new.txt"), "n\n").unwrap();
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("tuicr", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing uncommitted changes"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert_eq!(tuicr_tui_line(&logged), "TUICR -w");
    assert!(
        logged.contains("agent prompt caller-1 Review annotations from tuicr:"),
        "{logged}"
    );
}

// --- lumen pane runs ------------------------------------------------------

#[test]
fn pane_lumen_export_delivers_canonical_prompt() {
    // Scenarios "lumen export delivers" and "Export on stdout delivers": the
    // piped capture converts to canonical blocks — the file-level block as
    // `## <path>:0 (+)`, the range anchored on its first line, the `# <ref>`
    // header dropped — and the delivered prompt carries the lumen header and
    // the `.out` file is removed. Clean fixture repo: argv
    // `diff origin/main...HEAD`.
    let fx = Fixture::new("lumen-export");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing vs origin/main"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    // The delivered prompt: the lumen header plus every converted block in
    // order — the file-level block arrives as `## <path>:0 (+)`.
    assert!(
        logged.contains(
            "agent prompt caller-1 Review annotations from lumen:\n\n\
             ## src/main.rs:2 (+)\nspike annotation 2\n\n\
             ## src/old.rs:5 (-)\nstale import\n\n\
             ## src/lib.rs:4 (-)\nsimplify this range\n\n\
             ## README.md:0 (+)\nfile-level note"
        ),
        "{logged}"
    );
    assert_eq!(lumen_line(&logged), "LUMEN diff origin/main...HEAD");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_lumen_blank_is_no_annotations() {
    // Scenario "TUI closed without exporting": blank captured stdout is the
    // clean no-annotations case — message printed, file removed, no prompt.
    let fx = Fixture::new("lumen-blank");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("LUMEN_OUT", "blank"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: no annotations"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_lumen_probe_preamble_only_delivers_nothing() {
    // Scenario "TUI closed without exporting", probe-only variant, and the
    // lumen leg of "Clean exit with blank output is no-annotations": lumen
    // writes the live-observed terminal-probe preamble (OSC `ESC]11;?BEL` +
    // CSI `ESC[5n`) and the user exits via q/Esc without `s`, so the capture
    // holds no annotation content. Raw bytes alone never count as
    // annotations: the run ends no-annotations with NO `agent prompt` call
    // (no empty `Review annotations from lumen:` body) and the `.out` file
    // removed.
    let fx = Fixture::new("lumen-probes");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("LUMEN_OUT", "probes"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: no annotations"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("sent review annotations"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    assert!(
        !logged.contains("Review annotations from lumen:"),
        "{logged}"
    );
    assert!(logged.contains("plugin pane close pane-7"), "{logged}");
    assert!(out_files(&state.join("reviews")).is_empty());
}

#[test]
fn pane_lumen_garbage_keeps_raw_and_fails() {
    // Scenario "Unparseable export keeps raw": a non-blank capture that
    // converts to zero blocks writes the raw text to the file, keeps it, and
    // fails with the parse error.
    let fx = Fixture::new("lumen-garbage");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("LUMEN_OUT", "garbage"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot parse lumen annotation export; raw output kept at"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    let files = out_files(&state.join("reviews"));
    assert_eq!(files.len(), 1);
    assert_eq!(fs::read_to_string(&files[0]).unwrap(), "garbage capture\n");
}

#[test]
fn pane_lumen_rc1_keeps_raw_and_fails() {
    // Scenario "Nonzero exit keeps raw capture": the export fixture was
    // captured, lumen exits 1, the raw capture is written best-effort and
    // kept, and the run fails with the code.
    let fx = Fixture::new("lumen-rc1");
    let repo = fx.subdir("repo");
    clean_repo_with_origin_main(&repo);
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("LUMEN_RC", "1"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("lumen failed with exit code 1"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains(&format!("output kept at {}/reviews/", state.display())),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("agent prompt"), "{logged}");
    let files = out_files(&state.join("reviews"));
    assert_eq!(files.len(), 1);
    let raw = fs::read_to_string(&files[0]).unwrap();
    assert!(raw.contains("# origin/main"), "{raw}");
    assert!(raw.contains("**src/main.rs** line 2 (RIGHT)"), "{raw}");
}

#[test]
fn pane_lumen_dirty_repo_passes_diff() {
    // Scenario "lumen stdout is piped while stderr is inherited", argv side:
    // a dirty tree spawns `lumen diff` with no base fragment, and blank
    // stdout closes as no-annotations.
    let fx = Fixture::new("lumen-dirty");
    let repo = fx.subdir("repo");
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("base.txt"), "b\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    commit(&repo, "base");
    fs::write(repo.join("new.txt"), "n\n").unwrap();
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let path = fx.path_with_stubs();

    let mut env = pane_env_for("lumen", &repo, &state, &herdr, &log, &path);
    env.extend([
        ("HERDR_REVIEWS_CALLER_PANE", "caller-1"),
        ("HERDR_REVIEWS_CALLER_AGENT", "claude"),
        ("LUMEN_OUT", "blank"),
    ]);
    let out = run_plugin(&env);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("herdr-review: reviewing uncommitted changes"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("herdr-review: no annotations"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert_eq!(lumen_line(&logged), "LUMEN diff");
    assert!(out_files(&state.join("reviews")).is_empty());
}

// --- action runs ----------------------------------------------------------

#[test]
fn action_opens_the_review_pane_directly_for_the_sole_installed_tool() {
    // Scenarios "One catalog tool is installed"
    // and "The tool env pair is always present" (e2e side): the review-pane
    // open argv pins the tab shape, the success log names the pane and tab,
    // a best-effort focus runs, and no setup popup is opened. "Caller tab
    // pair is present on the direct open": the context `tab_id` rides the
    // argv after the caller pane and agent pairs, right before `--focus`.
    let fx = Fixture::new("action-direct");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"lumen\"\n").unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","focused_pane_agent":"claude","tab_id":"w3X:t9","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: opened review pane pane-stub in tab tab-stub"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("--entrypoint picker"), "{logged}");
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(
        open.starts_with(
            "plugin pane open --plugin alexeyco.review --entrypoint review --placement tab --workspace ws-1 --cwd"
        ),
        "{open}"
    );
    assert!(
        open.contains(&format!("--cwd {}", repo.display())),
        "{open}"
    );
    assert!(
        open.contains(&format!("--env HERDR_REVIEWS_DIR={}", repo.display())),
        "{open}"
    );
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
    assert!(
        open.contains("--env HERDR_REVIEWS_CALLER_PANE=p1"),
        "{open}"
    );
    assert!(
        open.contains("--env HERDR_REVIEWS_CALLER_AGENT=claude"),
        "{open}"
    );
    // The caller tab pair closes the env pairs, immediately before `--focus`.
    assert!(
        open.contains(
            "--env HERDR_REVIEWS_CALLER_AGENT=claude --env HERDR_REVIEWS_CALLER_TAB=w3X:t9 --focus"
        ),
        "{open}"
    );
    assert!(open.ends_with("--focus"), "{open}");
    // No setup dialog on the direct path.
    assert!(!logged.contains("--entrypoint setup"), "{logged}");
    assert!(logged.contains("tab focus tab-stub"), "{logged}");
    assert!(
        !stderr(&out).contains("press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn action_opens_picker_for_multiple_installed_tools_without_opening_review() {
    let fx = Fixture::new("action-picker");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(
        config_dir.join("config.toml"),
        "tool = \"lumen\"\nplacement = \"split\"\ndirection = \"down\"\n",
    )
    .unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_stubs();
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|line| line.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(
        open.contains("--entrypoint picker --placement popup"),
        "{open}"
    );
    assert!(
        open.contains("HERDR_REVIEWS_PICKER_TOOL_IDS=lumen,revdiff,tuicr"),
        "{open}"
    );
    assert!(open.contains("HERDR_REVIEWS_PLACEMENT=split"), "{open}");
    assert!(open.contains("HERDR_REVIEWS_DIRECTION=down"), "{open}");
    assert!(!logged.contains("--entrypoint review"), "{logged}");
}

#[test]
fn picker_without_interactive_input_exits_without_launching_a_tool() {
    let fx = Fixture::new("picker-noninteractive");
    let (herdr, log) = fx.stubs();
    let state = fx.subdir("state");
    let repo = fx.subdir("repo");
    let path = fx.path_with_stubs();
    let out = run_plugin(&[
        ("HERDR_PLUGIN_ENTRYPOINT_ID", "picker"),
        ("HERDR_REVIEWS_PICKER_TOOL_IDS", "lumen,revdiff"),
        ("HERDR_REVIEWS_DIR", repo.to_str().unwrap()),
        ("HERDR_REVIEWS_PLACEMENT", "split"),
        ("HERDR_REVIEWS_DIRECTION", "down"),
        ("HERDR_PLUGIN_STATE_DIR", state.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("tool selection unavailable without an interactive terminal"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(read_log(&log).is_empty(), "{}", read_log(&log));
}

#[test]
fn action_opens_the_split_review_pane_on_the_caller() {
    // Scenarios "Split and zoomed pass the caller pane as target" and
    // "Direction is passed only for split placement", e2e side: the caller
    // pane reaches the argv as `--target-pane` after the direction pair, and
    // the split shape carries no `--workspace` (the stub herdr rejects both
    // illegal shapes like the daemon).
    let fx = Fixture::new("action-split");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(
        config_dir.join("config.toml"),
        "placement = \"split\"\ndirection = \"down\"\ntool = \"revdiff\"\n",
    )
    .unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","focused_pane_agent":"claude","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(
        open.contains(
            "--plugin alexeyco.review --entrypoint review --placement split --direction down --target-pane p1 --cwd"
        ),
        "{open}"
    );
    assert!(
        open.contains(&format!("--cwd {}", repo.display())),
        "{open}"
    );
    assert!(
        open.contains(&format!("--env HERDR_REVIEWS_DIR={}", repo.display())),
        "{open}"
    );
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
    assert!(
        open.contains("--env HERDR_REVIEWS_CALLER_PANE=p1"),
        "{open}"
    );
    assert!(
        open.contains("--env HERDR_REVIEWS_CALLER_AGENT=claude"),
        "{open}"
    );
    // The split shape drops the workspace entirely: no flag (the daemon
    // rejects it), and no workspace env pair exists on the review open.
    assert!(!open.contains("--workspace "), "{open}");
    // Scenario "Caller tab pair is omitted when no tab resolves": the
    // context carries no `tab_id`, so no caller tab pair rides the argv.
    assert!(!open.contains("HERDR_REVIEWS_CALLER_TAB"), "{open}");
    assert!(open.ends_with("--focus"), "{open}");
    assert!(logged.contains("tab focus tab-stub"), "{logged}");
    assert!(!logged.contains("--entrypoint setup"), "{logged}");
}

#[test]
fn action_split_without_a_caller_falls_back_to_tab() {
    // Scenario "Split or zoomed without a caller pane falls back to tab":
    // the exact warning names the degraded placement and the open uses the
    // tab shape — no direction, no target — which is the one placement the
    // workspace flag is legal on, so `--workspace` reappears there.
    let fx = Fixture::new("action-fallback");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(
        config_dir.join("config.toml"),
        "placement = \"split\"\ndirection = \"down\"\ntool = \"revdiff\"\n",
    )
    .unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "herdr-review: warning: placement \"split\" needs a caller pane; using \"tab\""
        ),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(
        open.contains(
            "--plugin alexeyco.review --entrypoint review --placement tab --workspace ws-1 --cwd"
        ),
        "{open}"
    );
    assert!(!open.contains("--target-pane"), "{open}");
    assert!(!open.contains("--direction"), "{open}");
    assert!(open.ends_with("--focus"), "{open}");
}

#[test]
fn action_with_zero_installed_tools_opens_the_setup_dialog() {
    // Scenarios "No catalog tools are installed" and "Action success
    // opens the popup and exits": the exact setup argv is logged, no review
    // pane opens, and no focus call or hold follows the open. "Setup popup
    // never carries the caller tab": the context carries a `tab_id`, yet the
    // setup open still has no `--env` pair at all.
    let fx = Fixture::new("action-unset");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "placement = \"tab\"\n").unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_without_tools();
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","focused_pane_agent":"claude","tab_id":"w3X:t9","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: opened review tool setup"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert_eq!(
        open,
        "plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus"
    );
    // No review pane is opened, no focus call is made, nothing holds.
    assert!(!logged.contains("--entrypoint review"), "{logged}");
    assert!(!logged.contains("tab focus"), "{logged}");
    assert!(
        !stderr(&out).contains("press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn action_with_zero_tools_ignores_obsolete_tool_config_and_opens_setup() {
    // Scenario "No catalog tools are installed": even an obsolete configured
    // tool id cannot bypass PATH detection, so setup opens without a review pane.
    let fx = Fixture::new("action-missing");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"lumen\"\n").unwrap();
    // A hermetic PATH carrying only the herdr stub: no catalog tool is executable.
    let bin = fx.subdir("herdr-only");
    write_executable(&bin.join("herdr"), HERDR_STUB);
    let herdr = bin.join("herdr");
    let log = fx.dir.join("stub.log");
    let path = bin.to_str().unwrap().to_string();
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","focused_pane_agent":"claude","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("herdr-review: opened review tool setup"),
        "stderr: {}",
        stderr(&out)
    );
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert_eq!(
        open,
        "plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus"
    );
    assert!(!logged.contains("--entrypoint review"), "{logged}");
}

#[test]
fn action_ignores_obsolete_tool_config_and_uses_installed_tool() {
    // The obsolete `tool` key is ignored; the sole installed tool wins.
    let fx = Fixture::new("action-invalid");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"diffy\"\n").unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","focused_pane_agent":"claude","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));

    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(open.contains("--entrypoint review"), "{open}");
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
    assert!(
        !stderr(&out).contains("invalid tool"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn action_config_directory_failure_uses_defaults_and_selects_installed_tool() {
    let fx = Fixture::new("action-config-dir-failure");
    let (out, logged, _) = run_action_config_case(&fx, None, false, true);
    let open = action_review_open(&logged);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("config directory unavailable:"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(open.contains("--placement tab"), "{open}");
    assert!(!open.contains("--direction"), "{open}");
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
}

#[test]
fn action_missing_config_file_uses_defaults_and_selects_installed_tool() {
    let fx = Fixture::new("action-missing-config-file");
    let (out, logged, _) = run_action_config_case(&fx, None, false, false);
    let open = action_review_open(&logged);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("no config.toml at "),
        "stderr: {}",
        stderr(&out)
    );
    assert!(open.contains("--placement tab"), "{open}");
    assert!(!open.contains("--direction"), "{open}");
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
}

#[test]
fn action_corrupt_config_uses_defaults_and_selects_installed_tool() {
    let fx = Fixture::new("action-corrupt-config");
    let (out, logged, _) = run_action_config_case(&fx, Some("placement = [\n"), true, false);
    let open = action_review_open(&logged);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("invalid config.toml:"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(open.contains("--placement tab"), "{open}");
    assert!(!open.contains("--direction"), "{open}");
    assert!(open.contains("--env HERDR_REVIEW_TOOL=revdiff"), "{open}");
}

#[test]
fn action_maps_overlay_placement_to_review_open() {
    let fx = Fixture::new("action-overlay");
    let (out, logged, _) =
        run_action_config_case(&fx, Some("placement = \"overlay\"\n"), true, false);
    let open = action_review_open(&logged);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(open.contains("--placement overlay"), "{open}");
    assert!(!open.contains("--workspace"), "{open}");
    assert!(
        !stderr(&out).contains("herdr-review: warning:"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn action_refuses_popup_and_unknown_placement_with_tab_fallback() {
    for (tag, placement, warning) in [
        (
            "popup",
            "popup",
            r#"placement "popup" is not supported for the review pane; using "tab""#,
        ),
        (
            "floating",
            "floating",
            r#"invalid placement in config.toml: "floating""#,
        ),
    ] {
        let fx = Fixture::new(&format!("action-placement-{tag}"));
        let config = format!("placement = \"{placement}\"\n");
        let (out, logged, _) = run_action_config_case(&fx, Some(&config), true, false);
        let open = action_review_open(&logged);

        assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
        assert!(stderr(&out).contains(warning), "stderr: {}", stderr(&out));
        assert!(open.contains("--placement tab"), "{open}");
    }
}

#[test]
fn action_applies_direction_only_to_valid_split_directions() {
    for (tag, config, expected_placement, warning) in [
        (
            "auto",
            "placement = \"split\"\ndirection = \"auto\"\n",
            "split",
            None,
        ),
        (
            "tab-down",
            "placement = \"tab\"\ndirection = \"down\"\n",
            "tab",
            None,
        ),
        (
            "left",
            "placement = \"split\"\ndirection = \"left\"\n",
            "split",
            Some(r#"invalid direction in config.toml: "left""#),
        ),
    ] {
        let fx = Fixture::new(&format!("action-direction-{tag}"));
        let (out, logged, _) = run_action_config_case(&fx, Some(config), true, false);
        let open = action_review_open(&logged);

        assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
        if let Some(warning) = warning {
            assert!(stderr(&out).contains(warning), "stderr: {}", stderr(&out));
        }
        assert!(
            open.contains(&format!("--placement {expected_placement}")),
            "{open}"
        );
        assert!(!open.contains("--direction"), "{open}");
    }
}

#[test]
fn action_takes_pane_id_from_the_environment() {
    // Scenario "Pane id taken from the environment": no context pane id, so
    // `HERDR_PANE_ID` forwards as the caller pane of the review-pane open.
    let fx = Fixture::new("action-pane-env");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"revdiff\"\n").unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PANE_ID", "p-from-env"),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(
        open.contains("--env HERDR_REVIEWS_CALLER_PANE=p-from-env"),
        "{open}"
    );
}

#[test]
fn action_without_context_agent_omits_the_env_pair() {
    // Scenario "Agent without context entry is simply absent": no
    // focused_pane_agent in the context and no environment fallback exist,
    // so the pair is omitted.
    let fx = Fixture::new("action-no-agent");
    let repo = fx.subdir("repo");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"revdiff\"\n").unwrap();
    let (herdr, log) = fx.stubs();
    let path = fx.path_with_only_tool("revdiff");
    let ctx = format!(
        r#"{{"workspace_id":"ws-1","focused_pane_id":"p1","worktree":{{"checkout_path":"{}"}}}}"#,
        repo.display()
    );

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("HERDR_PLUGIN_CONTEXT_JSON", &ctx),
        ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
        ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
        ("PATH", &path),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let logged = read_log(&log);
    let open = logged
        .lines()
        .find(|l| l.starts_with("plugin pane open"))
        .unwrap_or("");
    assert!(!open.contains("HERDR_REVIEWS_CALLER_AGENT"), "{open}");
}

#[test]
fn action_error_exits_immediately_without_holding() {
    // Scenario "Action-mode error exits immediately": a missing context JSON
    // fails on stderr with exit 1 and no hold prompt.
    let fx = Fixture::new("action-error");
    let log = fx.dir.join("stub.log");

    let out = run_plugin(&[
        ("HERDR_PLUGIN_ACTION_ID", "review"),
        ("STUB_LOG", log.to_str().unwrap()),
    ]);

    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("HERDR_PLUGIN_CONTEXT_JSON is not set"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(read_log(&log).is_empty(), "{}", read_log(&log));
}

// --- setup-dialog runs ------------------------------------------------------

#[test]
fn setup_dialog_closes_on_a_key_without_any_call() {
    // Setup-dialog mode, process level: the pane resolves the config
    // directory for its path line, draws the one universal layout, and
    // closes on the scripted key — exit 0, no pane open, no hold. Scenarios
    // "Cancel exits cleanly" and "Every entry case renders the same dialog"
    // (the dialog ignores config content and PATH and always renders the
    // same static rows) over
    // the real binary, plus the brew install-command strings and the repo
    // URL of "Tool rows are alphabetical with dim inline install commands"
    // and "Repo URL line closes the content".
    let fx = Fixture::new("setup-close");
    let config_dir = fx.subdir("config");
    fs::write(config_dir.join("config.toml"), "tool = \"lumen\"\n").unwrap();
    // A PATH with the herdr stub only: no catalog tool is executable. The
    // dialog performs no PATH detection, so this renders identically to any
    // other entry case.
    let bin = fx.subdir("herdr-only");
    write_executable(&bin.join("herdr"), HERDR_STUB);
    let herdr = bin.join("herdr");
    let log = fx.dir.join("stub.log");
    let path = bin.to_str().unwrap().to_string();

    let out = run_plugin_with_stdin(
        &[
            ("HERDR_PLUGIN_ENTRYPOINT_ID", "setup"),
            ("HERDR_PLUGIN_CONFIG_DIR", config_dir.to_str().unwrap()),
            ("HERDR_BIN_PATH", herdr.to_str().unwrap()),
            ("PATH", &path),
            ("STUB_LOG", log.to_str().unwrap()),
        ],
        b"q",
    );

    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    // The universal layout: the config file path line, the three catalog
    // rows with their dim inline install commands, and the repo URL.
    assert!(
        stdout(&out).contains("Config file:"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains(&format!("{}/config.toml", config_dir.display())),
        "stdout: {}",
        stdout(&out)
    );
    for install in [
        "brew install jnsahaj/lumen/lumen",
        "brew install umputun/apps/revdiff",
        "brew install tuicr",
    ] {
        assert!(stdout(&out).contains(install), "stdout: {}", stdout(&out));
    }
    assert!(
        stdout(&out).contains("https://github.com/alexeyco/herdr-review"),
        "stdout: {}",
        stdout(&out)
    );
    // The dialog drew a frame and closed cleanly; nothing was held.
    assert!(
        stdout(&out).contains("enter/q/esc close"),
        "stdout: {}",
        stdout(&out)
    );
    let logged = read_log(&log);
    assert!(!logged.contains("plugin pane open"), "{logged}");
    assert!(
        logged.is_empty(),
        "dialog unexpectedly queried herdr configuration: {logged}"
    );
    assert!(
        !stderr(&out).contains("press Enter to close"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn outside_herdr_errors() {
    let out = run_plugin(&[]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("invoked outside herdr plugin context"),
        "stderr: {}",
        stderr(&out)
    );
}
