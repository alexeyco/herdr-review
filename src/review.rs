//! Review orchestration.
//!
//! The binary is tri-mode. Action mode parses the plugin context, resolves
//! the caller identity, reads `config.toml` once, and opens the review pane
//! directly when the configured tool is detected on `PATH` — otherwise it
//! opens the read-only setup popup — then exits. Setup mode (the popup
//! pane, see [`crate::setup`]) never opens a pane itself. Review-pane mode
//! runs the chosen tool as a direct child on the pane tty with a per-tool
//! stdio strategy, collects its annotations (revdiff's output file, tuicr's
//! session adapter, lumen's captured stdout), converts them to the canonical
//! block format, and delivers them to the agent pane that requested the
//! review.
//!
//! Review outputs are kept under `HERDR_PLUGIN_STATE_DIR/reviews` as
//! `<pid>-<unix millis>.out`. The pid keeps two reviews started in the same
//! millisecond from clobbering each other, and kept outputs older than
//! `REVIEW_MAX_AGE` are pruned best-effort at pane start.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config;
use crate::context;
use crate::convert;
use crate::git::{self, Scope};
use crate::herdr;
use crate::tool::{self, Tool};
use crate::tuicr;

/// Printed before blocking on stdin so an auto-closing plugin pane stays open
/// on error and no-agent paths.
const HOLD_PROMPT: &str = "herdr-review: press Enter to close";
/// Kept review outputs older than this are pruned at pane start.
const REVIEW_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Cap on the delivered agent prompt, well under ARG_MAX (1 MiB macOS / 2 MiB
/// Linux): annotations travel as a single argv element.
const MAX_PROMPT_BYTES: usize = 65536;

/// What to do with the tool's result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Annotations exist and an agent target is available.
    Deliver,
    /// The tool quit cleanly with nothing to report.
    NoAnnotations,
    /// Annotations exist, but no agent to send them to.
    NoAgent,
    /// The tool exited with an error; the output is kept.
    Failed(i32),
}

/// Decides the outcome from the tool's exit code, its collection channel, and
/// agent availability. `annotations` is the per-tool "annotations exist"
/// signal: revdiff exits 10 or exits 0 with non-blank `.out` content, tuicr
/// exits 0 with a non-empty comment list, lumen exits 0 with non-blank
/// captured stdout. Any exit code outside the tool's success set is
/// `Failed(rc)` regardless of collected output.
pub(crate) fn decide(tool_id: &str, rc: i32, annotations: bool, has_agent: bool) -> Outcome {
    // revdiff's success set also includes its annotations code 10; every
    // other tool's is 0 only.
    let success = rc == 0 || (tool_id == "revdiff" && rc == 10);
    if !success {
        return Outcome::Failed(rc);
    }
    if annotations {
        if has_agent {
            Outcome::Deliver
        } else {
            Outcome::NoAgent
        }
    } else {
        Outcome::NoAnnotations
    }
}

/// Prompt text sent to the agent: the tool-named header plus the trimmed
/// canonical annotation text.
pub(crate) fn prompt_text(tool_id: &str, output: &str) -> String {
    format!("Review annotations from {tool_id}:\n\n{}", output.trim())
}

/// Caps the delivered prompt at `max_bytes`, cutting on a UTF-8 boundary and
/// appending a note pointing at the kept output file when truncation happens.
/// Returns the text unchanged when it already fits.
fn cap_prompt(text: &str, max_bytes: usize, kept_path: &Path) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut capped = text[..cut].to_string();
    capped.push_str(&format!(
        "\n\n(output truncated; full annotations kept at {})",
        kept_path.display()
    ));
    capped
}

/// Full argv for the review-tool child, per the spawn matrix: revdiff takes
/// its scope fragments around the fixed trailing flags, tuicr its fragment
/// alone, lumen the `diff` subcommand plus its fragment.
fn child_argv(tool_id: &str, bin: &str, out_path: &str, scope: &Scope) -> Vec<String> {
    match tool_id {
        "revdiff" => revdiff_argv(bin, out_path, scope),
        "tuicr" => {
            let mut argv = vec![bin.to_string()];
            argv.extend(scope.fragments("tuicr"));
            argv
        }
        "lumen" => {
            let mut argv = vec![bin.to_string(), "diff".to_string()];
            argv.extend(scope.fragments("lumen"));
            argv
        }
        _ => unreachable!("tool ids come from the catalog"),
    }
}

/// Full argv for the revdiff child: binary, the scope's flag fragments, the
/// fixed trailing `--output <out> --exit-code-on-annotations`, and the
/// scope's positional refs last so both strict and lenient parsers accept
/// them.
fn revdiff_argv(bin: &str, out_path: &str, scope: &Scope) -> Vec<String> {
    let fragments = scope.fragments("revdiff");
    // `Uncommitted`'s fragment is the `--untracked` flag, which goes before
    // the fixed trailing flags; `VsBase`'s are the two positional refs, which
    // go after them.
    let (leading, trailing): (&[String], &[String]) = match scope {
        Scope::Uncommitted => (&fragments, &[]),
        Scope::VsBase(_) | Scope::NothingToReview => (&[], &fragments),
    };
    let mut argv = vec![bin.to_string()];
    argv.extend(leading.iter().cloned());
    argv.push("--output".to_string());
    argv.push(out_path.to_string());
    argv.push("--exit-code-on-annotations".to_string());
    argv.extend(trailing.iter().cloned());
    argv
}

/// The stdio strategy for one child stream.
#[derive(Debug, PartialEq, Eq)]
enum StreamPlan {
    /// The child inherits the pane tty.
    Inherit,
    /// The child's stream is a pipe the pane process reads to completion.
    Piped,
}

/// The per-tool stdio plan: lumen's stdout is piped — its annotation export is
/// captured while the TUI renders to `/dev/tty` itself — and its stderr
/// inherits the pane tty; revdiff and tuicr inherit both streams.
fn stdio_plan(tool_id: &str) -> (StreamPlan, StreamPlan) {
    match tool_id {
        "lumen" => (StreamPlan::Piped, StreamPlan::Inherit),
        _ => (StreamPlan::Inherit, StreamPlan::Inherit),
    }
}

/// The review-tool child's result: its exit code (absent when the child was
/// killed) and, for lumen, the captured stdout.
#[derive(Debug, Default)]
struct ChildResult {
    code: Option<i32>,
    stdout: String,
}

/// Spawns the resolved binary as a direct child in `dir` with the tool's
/// stdio strategy and waits for it to exit. A spawn failure (for example the
/// binary was removed between resolution and spawn) is the hard error
/// `failed to run <bin> in <dir>: <error>`.
fn spawn_child(tool_id: &str, argv: &[String], dir: &Path) -> Result<ChildResult, String> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(dir);
    if stdio_plan(tool_id).0 == StreamPlan::Piped {
        // Piped stdout, inherited stderr: the pipe is drained to completion
        // while the child runs, so a large export cannot deadlock.
        cmd.stdout(Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to run {} in {}: {e}", argv[0], dir.display()))?;
        let output = child
            .wait_with_output()
            .map_err(|e| format!("failed to wait for {}: {e}", argv[0]))?;
        Ok(ChildResult {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    } else {
        let status = cmd
            .status()
            .map_err(|e| format!("failed to run {} in {}: {e}", argv[0], dir.display()))?;
        Ok(ChildResult {
            code: status.code(),
            stdout: String::new(),
        })
    }
}

/// Everything the collection stage needs about one review run.
struct PaneRun<'a> {
    tool: &'a Tool,
    bin: &'a str,
    dir: &'a Path,
    out_path: &'a Path,
    /// Recorded before the spawn; the tuicr adapter selects sessions updated
    /// after it.
    launched_at: SystemTime,
    caller_pane: Option<&'a str>,
    caller_agent: Option<&'a str>,
    /// The forwarded caller tab (`HERDR_REVIEWS_CALLER_TAB`); a clean ending
    /// returns focus to it best-effort.
    caller_tab: Option<&'a str>,
}

/// Action mode: parse the context, resolve the caller identity, read the
/// config once, then decide — the configured tool detected on `PATH` opens
/// the review pane directly, anything else (unset, blank, invalid and
/// already degraded to unset, or valid but missing from `PATH`) opens the
/// setup popup. Either way the action exits without holding; herdr already
/// applies `--focus` at open.
pub(crate) fn run_action() -> Result<(), String> {
    let json = env_non_blank("HERDR_PLUGIN_CONTEXT_JSON").ok_or_else(|| {
        "HERDR_PLUGIN_CONTEXT_JSON is not set; invoke this action from herdr".to_string()
    })?;
    let ctx = context::parse(&json)?;
    let identity = CallerIdentity::resolve(
        &ctx,
        env_non_blank("HERDR_WORKSPACE_ID").as_deref(),
        env_non_blank("HERDR_PANE_ID").as_deref(),
        env_non_blank("HERDR_TAB_ID").as_deref(),
    );
    let path = std::env::var("PATH").unwrap_or_default();
    let deps = ActionDeps {
        load_config: &config::load,
        open: &|config, dir, tool_id, workspace, caller_pane, caller_agent, caller_tab| {
            herdr::open_review_pane(
                config,
                workspace,
                dir,
                tool_id,
                caller_pane,
                caller_agent,
                caller_tab,
            )
        },
        open_setup: &herdr::open_setup_popup,
        focus: &herdr::tab_focus,
        log: &|msg| eprintln!("{msg}"),
        warn: &|msg| eprintln!("herdr-review: warning: {msg}"),
    };
    run_action_with(&identity, &ctx.dir, &path, &deps)
}

/// The caller identity the action resolves from the context JSON with its
/// environment fallbacks. The resolved values feed the action's own
/// review-pane open directly — `--workspace <id>` for `tab` placement,
/// `--target-pane <pane>` for `split`/`zoomed`, and the
/// `HERDR_REVIEWS_CALLER_PANE` / `HERDR_REVIEWS_CALLER_AGENT` /
/// `HERDR_REVIEWS_CALLER_TAB` env pairs — with no forwarding hop: the
/// action-time context is the only trusted source for caller identity.
struct CallerIdentity {
    /// `workspace_id` from the context, or `HERDR_WORKSPACE_ID`.
    workspace: Option<String>,
    /// `focused_pane_id` from the context, or `HERDR_PANE_ID`.
    pane: Option<String>,
    /// `focused_pane_agent` from the context only — no environment fallback
    /// exists, because a pane's own environment cannot name the original
    /// caller.
    agent: Option<String>,
    /// `tab_id` from the context, or `HERDR_TAB_ID` — the tab a clean
    /// review-pane ending returns focus to.
    tab: Option<String>,
}

impl CallerIdentity {
    fn resolve(
        ctx: &context::Context,
        workspace_env: Option<&str>,
        pane_env: Option<&str>,
        tab_env: Option<&str>,
    ) -> Self {
        Self {
            workspace: ctx
                .workspace_id
                .clone()
                .or_else(|| workspace_env.map(str::to_string)),
            pane: ctx
                .focused_pane_id
                .clone()
                .or_else(|| pane_env.map(str::to_string)),
            agent: ctx.focused_pane_agent.clone(),
            tab: ctx.tab_id.clone().or_else(|| tab_env.map(str::to_string)),
        }
    }
}

/// The review-pane open step: (config, directory, tool id, workspace, caller
/// pane, caller agent, caller tab) to the parsed reply. The config borrows
/// for the call only, not for the lifetime of the deps.
type OpenFn<'a> = &'a dyn for<'c> Fn(
    &'c crate::config::Config,
    &str,
    &str,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<&str>,
) -> Result<Option<herdr::PaneIds>, String>;

/// The process-bound steps of the action flow, injected so tests script them.
struct ActionDeps<'a> {
    load_config: &'a dyn Fn() -> crate::config::Config,
    open: OpenFn<'a>,
    open_setup: &'a dyn Fn() -> Result<(), String>,
    focus: &'a dyn Fn(&str) -> Result<(), String>,
    log: &'a dyn Fn(&str),
    warn: &'a dyn Fn(&str),
}

/// The action's decision flow over the resolved identity, the review
/// directory, the `PATH` the tool detection runs on, and the process seams.
/// Requirement: "The action decides between the direct open and the setup dialog".
fn run_action_with(
    identity: &CallerIdentity,
    dir: &str,
    path: &str,
    deps: &ActionDeps,
) -> Result<(), String> {
    let config = (deps.load_config)();
    // The main path: a valid configured tool id whose binary is detected on
    // `PATH` with the catalog's detection rules. Anything else routes the
    // launch to the setup dialog, and no review pane is opened.
    let Some(tool) = config
        .tool
        .filter(|tool| tool::installed(path).contains(tool))
    else {
        (deps.open_setup)()?;
        (deps.log)("herdr-review: opened review tool setup");
        return Ok(());
    };
    let mut config = config;
    // The herdr 0.9.5 daemon rejects targetless split/zoomed opens with
    // `invalid_params`; without a caller pane those placements degrade to
    // tab — which still opens the pane — with one warning naming the
    // degraded knob, consistent with the never-fatal config layer.
    if matches!(
        config.placement,
        config::Placement::Split | config::Placement::Zoomed
    ) && identity.pane.is_none()
    {
        (deps.warn)(&format!(
            "placement \"{}\" needs a caller pane; using \"tab\"",
            config.placement.as_str()
        ));
        config.placement = config::Placement::Tab;
    }
    let ids = (deps.open)(
        &config,
        dir,
        tool.id,
        identity.workspace.as_deref(),
        identity.pane.as_deref(),
        identity.agent.as_deref(),
        identity.tab.as_deref(),
    )?;
    // An unparsable reply already warned; exit status 0 means the pane
    // exists, so the launch still succeeds without the log line.
    if let Some(ids) = ids {
        match &ids.tab_id {
            Some(tab) => {
                (deps.log)(&format!(
                    "herdr-review: opened review pane {} in tab {}",
                    ids.pane_id, tab
                ));
                // Defensive: re-assert focus best-effort against a
                // client-side rendering race; herdr already applies `--focus`
                // at open.
                let _ = (deps.focus)(tab);
            }
            None => (deps.log)(&format!("herdr-review: opened review pane {}", ids.pane_id)),
        }
    }
    Ok(())
}

/// Review-pane mode: probe git read-only, announce the scope, spawn the chosen
/// tool, collect and convert its annotations, and deliver them.
pub(crate) fn run_pane() -> Result<(), String> {
    herdr::require_bin()?;
    // Runtime commands may default to the plugin directory as cwd; prefer the
    // directory the action resolved and exported, falling back to the process
    // cwd when the variable is absent.
    let dir = match env_non_blank("HERDR_REVIEWS_DIR") {
        Some(d) => PathBuf::from(d),
        None => {
            std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?
        }
    };
    // The tool selection precedes any git probe or spawn.
    let tool = tool::selected()?;

    let scope = git::select_scope(&git::probe(&dir));
    match &scope {
        // A failed probe already printed its warning; either way no tool runs
        // and no output file is created — hold the pane and exit cleanly.
        Scope::NothingToReview => {
            println!("herdr-review: nothing to review");
            hold_pane();
            return Ok(());
        }
        Scope::Uncommitted => println!("herdr-review: reviewing uncommitted changes"),
        Scope::VsBase(base) => println!("herdr-review: reviewing vs {base}"),
    }

    let reports = state_dir().join("reviews");
    ensure_state_dir(&reports)?;
    prune_old_reviews(&reports, SystemTime::now());
    let out_path = reports.join(state_file_name(std::process::id(), unix_millis()?));
    // revdiff writes the file itself; for tuicr and lumen the plugin owns the
    // content, so an empty file exists before the spawn and failure paths
    // keep evidence on disk.
    if tool.id != "revdiff" {
        write_output(&out_path, "")?;
    }
    let bin = tool::resolve(tool)?;
    let argv = child_argv(tool.id, &bin, &out_path.to_string_lossy(), &scope);
    let caller_pane = env_non_blank("HERDR_REVIEWS_CALLER_PANE");
    let caller_agent = env_non_blank("HERDR_REVIEWS_CALLER_AGENT");
    let caller_tab = env_non_blank("HERDR_REVIEWS_CALLER_TAB");
    // The launch timestamp precedes the spawn so the tuicr adapter can tell
    // this run's sessions from older ones.
    let run = PaneRun {
        tool,
        bin: &bin,
        dir: &dir,
        out_path: &out_path,
        launched_at: SystemTime::now(),
        caller_pane: caller_pane.as_deref(),
        caller_agent: caller_agent.as_deref(),
        caller_tab: caller_tab.as_deref(),
    };
    let child = spawn_child(tool.id, &argv, &dir)?;
    collect_and_finish(&run, &child)
}

/// Per-tool collection and outcome: reads revdiff's output file, runs the
/// tuicr session adapter, converts lumen's captured stdout, and maps each to
/// the delivery decision.
fn collect_and_finish(run: &PaneRun, child: &ChildResult) -> Result<(), String> {
    collect_and_finish_with(run, child, &real_teardown())
}

/// [`collect_and_finish`] with the process-bound herdr calls injected, so
/// the clean-ending teardown is testable without the environment.
fn collect_and_finish_with(
    run: &PaneRun,
    child: &ChildResult,
    teardown: &Teardown,
) -> Result<(), String> {
    let tool = run.tool;
    let Some(rc) = child.code else {
        // No exit code means the child was killed; lumen's partial capture is
        // kept, the other tools' files are already on disk. The run fails
        // with the killed message and the output file is kept.
        if tool.id == "lumen" {
            keep_raw(run.out_path, &child.stdout);
        }
        return Err(format!(
            "{} terminated without an exit code (killed?); output kept at {}",
            tool.id,
            run.out_path.display()
        ));
    };
    match tool.id {
        "revdiff" => {
            let output = read_output(rc, run.out_path)?;
            let annotations = rc == 10 || (rc == 0 && !output.trim().is_empty());
            finish(run, rc, annotations, &output, teardown)
        }
        "tuicr" => {
            if rc != 0 {
                // No adapter command runs on a nonzero exit; the (empty)
                // output file is kept and the run fails with the code.
                return finish(run, rc, false, "", teardown);
            }
            let comments = tuicr::collect_comments(
                run.dir,
                run.launched_at,
                &|args| tuicr::exec_tuicr(run.bin, run.dir, args),
                &|args| tuicr::exec_git(run.dir, args),
            )
            .map_err(|detail| {
                format!(
                    "tuicr session lookup failed: {detail}; output kept at {}",
                    run.out_path.display()
                )
            })?;
            let canonical = convert::tuicr_to_canonical(&comments);
            write_output(run.out_path, &canonical)?;
            finish(run, rc, !comments.is_empty(), &canonical, teardown)
        }
        "lumen" => {
            if rc != 0 {
                // Whatever was captured is written best-effort and the run
                // fails with the code.
                keep_raw(run.out_path, &child.stdout);
                return finish(run, rc, false, "", teardown);
            }
            let canonical = collect_lumen_output(&child.stdout, run.out_path)?;
            // The annotations signal runs on the escape-stripped remainder,
            // matching the collection's blank check: lumen emits terminal
            // probes to stdout even when closed without exporting
            // (live-observed), so raw bytes alone never make annotations.
            let annotations = !convert::strip_terminal_escapes(&child.stdout)
                .trim()
                .is_empty();
            finish(run, rc, annotations, &canonical, teardown)
        }
        _ => unreachable!("tool ids come from the catalog"),
    }
}

/// Reads revdiff's output file. A missing or unreadable file is only harmless
/// when revdiff exited 0 (nothing to annotate); otherwise annotations are
/// expected, so the read failure is fatal and the error path keeps the file.
fn read_output(rc: i32, out_path: &Path) -> Result<String, String> {
    match std::fs::read(out_path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if rc == 0 => {
            eprintln!(
                "herdr-review: warning: cannot read {}: {e}",
                out_path.display()
            );
            Ok(String::new())
        }
        Err(e) => Err(format!(
            "cannot read revdiff output {}: {e}",
            out_path.display()
        )),
    }
}

/// lumen collection after a clean exit: the blank check runs on the
/// escape-stripped remainder — lumen writes terminal-probe escapes (OSC
/// `ESC]11;?BEL` + CSI `ESC[5n`) to stdout at TUI startup even when the user
/// closes without exporting, so a probe-only capture is the clean
/// no-annotations case, never a parse attempt. A stripped-blank capture
/// collects nothing; a parseable export is converted and written to the
/// `.out` file; a stripped non-blank export that converts to zero blocks is
/// the hard parse error, with the raw capture written and kept.
fn collect_lumen_output(capture: &str, out_path: &Path) -> Result<String, String> {
    if convert::strip_terminal_escapes(capture).trim().is_empty() {
        return Ok(String::new());
    }
    match convert::lumen_to_canonical(capture) {
        Some(canonical) => {
            write_output(out_path, &canonical)?;
            Ok(canonical)
        }
        None => {
            write_output(out_path, capture)?;
            Err(format!(
                "cannot parse lumen annotation export; raw output kept at {}",
                out_path.display()
            ))
        }
    }
}

/// Writes collected text to the `.out` file; a write failure is the fatal
/// `cannot write review output <path>: <error>`.
fn write_output(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|e| format!("cannot write review output {}: {e}", path.display()))
}

/// Best-effort raw write: whatever the tool produced is kept for evidence, a
/// write failure only warns.
fn keep_raw(out_path: &Path, raw: &str) {
    if let Err(e) = std::fs::write(out_path, raw) {
        eprintln!(
            "herdr-review: warning: cannot write review output {}: {e}",
            out_path.display()
        );
    }
}

/// Maps the tool's result to the delivery decision. Quiet paths return focus
/// to the caller tab and then close the pane; error and no-agent paths keep
/// the output file and block on stdin so the auto-closing pane stays
/// readable.
fn finish(
    run: &PaneRun,
    rc: i32,
    annotations: bool,
    output: &str,
    teardown: &Teardown,
) -> Result<(), String> {
    let tool = run.tool;
    let has_agent = run.caller_pane.is_some() && run.caller_agent.is_some();
    match decide(tool.id, rc, annotations, has_agent) {
        Outcome::Deliver => {
            // `decide` returns Deliver only when both caller pane and agent are
            // present, so the pane is always Some here; avoid expect() anyway.
            let Some(target) = run.caller_pane else {
                return Err("internal: deliver outcome without a caller pane".to_string());
            };
            let prompt = cap_prompt(
                &prompt_text(tool.id, output),
                MAX_PROMPT_BYTES,
                run.out_path,
            );
            match (teardown.deliver)(target, &prompt) {
                Ok(()) => {
                    eprintln!("herdr-review: sent review annotations to agent pane {target}");
                    remove(run.out_path);
                    // Focus BEFORE the close: the daemon kills a pane process
                    // at the moment it closes its own pane, so a post-close
                    // focus call never runs (live herdr 0.9.5 finding).
                    refocus_caller_tab(run.caller_tab, teardown);
                    (teardown.close)();
                    Ok(())
                }
                Err(e) => Err(format!(
                    "failed to send annotations to agent pane {target}: {e}; output kept at {}",
                    run.out_path.display()
                )),
            }
        }
        Outcome::NoAnnotations => {
            eprintln!("herdr-review: no annotations");
            remove(run.out_path);
            // Same focus-before-close ordering as the delivered leg.
            refocus_caller_tab(run.caller_tab, teardown);
            (teardown.close)();
            Ok(())
        }
        Outcome::NoAgent => {
            eprintln!(
                "herdr-review: no agent in focused pane; annotations kept at {}",
                run.out_path.display()
            );
            (teardown.hold)();
            Ok(())
        }
        Outcome::Failed(code) => Err(format!(
            "{} failed with exit code {code}; output kept at {}",
            tool.id,
            run.out_path.display()
        )),
    }
}

/// The process-bound herdr calls of the delivery and teardown stage, injected
/// so tests script them: the agent prompt, the focus return, the best-effort
/// pane close, the warning sink, and the hold.
struct Teardown<'a> {
    /// `herdr agent prompt <pane> <text>`.
    deliver: &'a dyn Fn(&str, &str) -> Result<(), String>,
    /// The best-effort pane close (pane id from env, tab-close fallback).
    close: &'a dyn Fn(),
    /// `herdr tab focus <tab>` — the focus return on a clean ending, run
    /// before the pane close.
    focus: &'a dyn Fn(&str) -> Result<(), String>,
    /// The `herdr-review: warning:` sink.
    warn: &'a dyn Fn(&str),
    /// The stdin hold for the no-agent and error paths.
    hold: &'a dyn Fn(),
}

/// The production [`Teardown`]: the real herdr calls.
fn real_teardown() -> Teardown<'static> {
    Teardown {
        deliver: &herdr::agent_prompt,
        close: &close_pane,
        focus: &herdr::tab_focus,
        warn: &warn_line,
        hold: &hold_pane,
    }
}

/// The `herdr-review: warning:` line sink.
fn warn_line(msg: &str) {
    eprintln!("herdr-review: warning: {msg}");
}

/// Focus return after a clean ending: best-effort `herdr tab focus` on the
/// forwarded caller tab — one warning on a failed or unspawnable call, never
/// an error, because the review itself already succeeded. A missing or blank
/// caller tab skips the call silently. The call runs BEFORE the pane close:
/// closing its own pane kills the process, so a post-close focus never runs
/// (live herdr 0.9.5 finding).
fn refocus_caller_tab(tab: Option<&str>, teardown: &Teardown) {
    if let Some(tab) = tab
        && let Err(e) = (teardown.focus)(tab)
    {
        (teardown.warn)(&format!("failed to focus caller tab {tab}: {e}"));
    }
}

/// Best-effort pane teardown: close the pane by id, falling back to closing
/// its tab. Failures are logged, never propagated.
fn close_pane() {
    close_pane_with(
        env_non_blank("HERDR_PANE_ID").as_deref(),
        env_non_blank("HERDR_TAB_ID").as_deref(),
        &herdr::pane_close,
        &herdr::tab_close,
    );
}

/// [`close_pane`] with the pane and tab ids and the closers injected, so the
/// fallback is testable without the process environment.
fn close_pane_with(
    pane: Option<&str>,
    tab: Option<&str>,
    close_pane: &dyn Fn(&str) -> Result<(), String>,
    close_tab: &dyn Fn(&str) -> Result<(), String>,
) {
    if let Some(pane) = pane {
        match close_pane(pane) {
            Ok(()) => return,
            Err(e) => eprintln!("herdr-review: failed to close pane {pane}: {e}"),
        }
    }
    if let Some(tab) = tab
        && let Err(e) = close_tab(tab)
    {
        eprintln!("herdr-review: failed to close tab {tab}: {e}");
    }
}

/// Blocks the pane process on stdin so an auto-closing plugin pane stays
/// readable on error and no-agent paths. A closed stdin (EOF) returns at once,
/// so the hold never hangs a non-interactive run.
pub(crate) fn hold_pane() {
    eprintln!("{HOLD_PROMPT}");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

/// State filename: `<pid>-<unix millis>.out`. The pid keeps two reviews
/// started in the same millisecond from clobbering each other.
fn state_file_name(pid: u32, millis: u128) -> String {
    format!("{pid}-{millis}.out")
}

/// Creates `<state_dir>/reviews` on demand; a failure is fatal.
fn ensure_state_dir(reports: &Path) -> Result<(), String> {
    std::fs::create_dir_all(reports)
        .map_err(|e| format!("failed to create state dir {}: {e}", reports.display()))
}

/// The state directory: the trimmed `HERDR_PLUGIN_STATE_DIR`, or the system
/// temporary directory when unset or blank.
fn state_dir() -> PathBuf {
    state_dir_from(std::env::var("HERDR_PLUGIN_STATE_DIR").ok().as_deref())
}

/// [`state_dir`] against an already-read raw env value.
fn state_dir_from(raw: Option<&str>) -> PathBuf {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Removes `*.out` files directly under `dir` whose mtime is older than
/// `REVIEW_MAX_AGE`. Best-effort: a missing dir is ignored, removal failures
/// are logged as warnings, and non-`.out` entries are never touched.
fn prune_old_reviews(dir: &Path, now: SystemTime) {
    prune_old_reviews_with(dir, now, |warning| {
        eprintln!("herdr-review: warning: {warning}")
    });
}

/// [`prune_old_reviews`] with the warning sink injected, so the failure path
/// is testable.
fn prune_old_reviews_with(dir: &Path, now: SystemTime, mut warn: impl FnMut(String)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("out") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age < REVIEW_MAX_AGE {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            warn(format!("cannot remove {}: {e}", path.display()));
        }
    }
}

/// The current unix millisecond stamp; a clock before the epoch is fatal.
fn unix_millis() -> Result<u128, String> {
    millis_at(SystemTime::now())
}

/// [`unix_millis`] at a fixed time, so the pre-epoch error is testable.
fn millis_at(now: SystemTime) -> Result<u128, String> {
    now.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .map_err(|e| format!("system clock error: {e}"))
}

/// Removes a kept output file; already gone counts as success, any other
/// failure is logged, never propagated.
fn remove(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() == std::io::ErrorKind::NotFound {
            return;
        }
        eprintln!(
            "herdr-review: warning: cannot remove {}: {e}",
            path.display()
        );
    }
}

fn env_non_blank(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Direction, Placement};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A canonicalized scratch directory, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "herdr-review-review-test-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(std::fs::canonicalize(&dir).unwrap())
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Writes an executable stub and returns its path.
        fn stub(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, body).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn set_mtime(path: &Path, t: SystemTime) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(t))
            .unwrap();
    }

    // --- Outcome decisions -------------------------------------------------

    #[test]
    fn nonzero_success_code_with_agent_delivers() {
        // Scenario "Nonzero success code with agent delivers": revdiff's
        // annotations code 10 with both caller env vars set.
        assert_eq!(decide("revdiff", 10, true, true), Outcome::Deliver);
    }

    #[test]
    fn annotations_without_an_agent() {
        // Scenario "Annotations without an agent".
        for tool in ["revdiff", "tuicr", "lumen"] {
            assert_eq!(decide(tool, 0, true, false), Outcome::NoAgent);
            assert_eq!(decide(tool, 0, true, true), Outcome::Deliver);
        }
        assert_eq!(decide("revdiff", 10, true, false), Outcome::NoAgent);
    }

    #[test]
    fn clean_exit_with_blank_output_is_no_annotations() {
        // Scenario "Clean exit with blank output is no-annotations" for all
        // three tools.
        for tool in ["revdiff", "tuicr", "lumen"] {
            assert_eq!(decide(tool, 0, false, true), Outcome::NoAnnotations);
            assert_eq!(decide(tool, 0, false, false), Outcome::NoAnnotations);
        }
    }

    #[test]
    fn any_other_exit_code_fails_regardless_of_output() {
        // Scenario "Any other exit code fails regardless of output": revdiff
        // outside {0, 10}, tuicr/lumen nonzero — even with annotations.
        assert_eq!(decide("revdiff", 1, true, true), Outcome::Failed(1));
        assert_eq!(decide("revdiff", 2, false, false), Outcome::Failed(2));
        assert_eq!(decide("tuicr", 1, true, true), Outcome::Failed(1));
        assert_eq!(decide("lumen", 2, true, true), Outcome::Failed(2));
    }

    #[test]
    fn tuicr_with_comments_delivers() {
        // Scenario "tuicr with comments delivers".
        assert_eq!(decide("tuicr", 0, true, true), Outcome::Deliver);
    }

    #[test]
    fn lumen_export_delivers() {
        // Scenario "lumen export delivers".
        assert_eq!(decide("lumen", 0, true, true), Outcome::Deliver);
    }

    // --- Prompt construction ----------------------------------------------

    #[test]
    fn header_names_the_tool_that_ran() {
        // Scenario "Header names the tool that ran".
        assert_eq!(
            prompt_text("tuicr", "## a.rs:1 (+)\nnote\n"),
            "Review annotations from tuicr:\n\n## a.rs:1 (+)\nnote"
        );
        assert_eq!(
            prompt_text("lumen", "  body  "),
            "Review annotations from lumen:\n\nbody"
        );
    }

    #[test]
    fn cap_prompt_short_text_is_unchanged() {
        // Scenario "Prompt within the cap is sent intact".
        let path = Path::new("/state/reviews/1.out");
        assert_eq!(cap_prompt("short", MAX_PROMPT_BYTES, path), "short");
    }

    #[test]
    fn cap_prompt_truncates_long_ascii_and_notes_path() {
        let path = Path::new("/state/reviews/1.out");
        let text = "a".repeat(MAX_PROMPT_BYTES + 100);
        let capped = cap_prompt(&text, MAX_PROMPT_BYTES, path);
        assert!(capped.starts_with(&"a".repeat(MAX_PROMPT_BYTES)));
        assert!(
            capped.contains("(output truncated; full annotations kept at /state/reviews/1.out)"),
            "{capped}"
        );
        assert!(capped.len() < text.len());
    }

    #[test]
    fn cap_prompt_does_not_split_multibyte_char() {
        // Scenario "Oversized prompt is cut at a character boundary".
        let path = Path::new("/state/reviews/1.out");
        let text = "é".repeat(100);
        let capped = cap_prompt(&text, 3, path);
        assert!(capped.starts_with('é'), "{capped}");
        assert!(capped.contains("(output truncated"), "{capped}");
    }

    // --- Child argv and stdio ---------------------------------------------

    #[test]
    fn revdiff_child_argv_keeps_the_fixed_trailing_flags() {
        // Scenario "revdiff argv keeps the fixed trailing flags": the scope
        // flag before the fixed pair, the scope positionals last.
        assert_eq!(
            revdiff_argv("/bin/revdiff", "/s/1.out", &Scope::Uncommitted),
            [
                "/bin/revdiff",
                "--untracked",
                "--output",
                "/s/1.out",
                "--exit-code-on-annotations"
            ]
        );
        assert_eq!(
            revdiff_argv(
                "/bin/revdiff",
                "/s/1.out",
                &Scope::VsBase("origin/main".to_string())
            ),
            [
                "/bin/revdiff",
                "--output",
                "/s/1.out",
                "--exit-code-on-annotations",
                "origin/main",
                "HEAD"
            ]
        );
        assert_eq!(
            revdiff_argv("/bin/revdiff", "/s/1.out", &Scope::NothingToReview),
            [
                "/bin/revdiff",
                "--output",
                "/s/1.out",
                "--exit-code-on-annotations"
            ]
        );
    }

    #[test]
    fn child_argv_per_tool() {
        assert_eq!(
            child_argv("tuicr", "/bin/tuicr", "/s/1.out", &Scope::Uncommitted),
            ["/bin/tuicr", "-w"]
        );
        assert_eq!(
            child_argv(
                "tuicr",
                "/bin/tuicr",
                "/s/1.out",
                &Scope::VsBase("origin/main".to_string())
            ),
            ["/bin/tuicr", "-r", "origin/main..HEAD"]
        );
        assert_eq!(
            child_argv("lumen", "/bin/lumen", "/s/1.out", &Scope::Uncommitted),
            ["/bin/lumen", "diff"]
        );
        assert_eq!(
            child_argv(
                "lumen",
                "/bin/lumen",
                "/s/1.out",
                &Scope::VsBase("origin/main".to_string())
            ),
            ["/bin/lumen", "diff", "origin/main...HEAD"]
        );
    }

    #[test]
    fn stdio_plan_pipes_only_lumens_stdout() {
        // Scenarios "tuicr runs on the inherited tty" (stdio plan: both
        // streams inherit the pane tty) and "lumen stdout is piped while
        // stderr is inherited" (plan: piped stdout, inherited stderr). The
        // live-tty behavior is exercised by the spawn tests and the manual
        // gate; this pins the plan the spawn builder follows.
        assert_eq!(
            stdio_plan("tuicr"),
            (StreamPlan::Inherit, StreamPlan::Inherit)
        );
        assert_eq!(
            stdio_plan("revdiff"),
            (StreamPlan::Inherit, StreamPlan::Inherit)
        );
        assert_eq!(
            stdio_plan("lumen"),
            (StreamPlan::Piped, StreamPlan::Inherit)
        );
    }

    #[test]
    fn spawn_failure() {
        // Scenario "Spawn failure": the binary vanished between resolution
        // and spawn.
        let dir = TempDir::new("spawn-fail");
        let argv = vec!["/nonexistent-tool-bin".to_string(), "-w".to_string()];
        let err = spawn_child("tuicr", &argv, dir.path()).unwrap_err();
        assert!(
            err.contains("failed to run /nonexistent-tool-bin in"),
            "{err}"
        );
    }

    #[test]
    fn revdiff_child_argv_reaches_the_child() {
        // Scenario "revdiff argv keeps the fixed trailing flags", live: the
        // child sees the exact argv (logged by the stub) in order.
        let dir = TempDir::new("revdiff-argv");
        let log = dir.path().join("argv.log");
        let out = dir.path().join("1.out");
        let bin = dir.stub(
            "revdiff",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nexit 0\n",
                log.display()
            ),
        );
        let argv = revdiff_argv(
            &bin.to_string_lossy(),
            &out.to_string_lossy(),
            &Scope::VsBase("origin/main".to_string()),
        );
        spawn_child("revdiff", &argv, dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().trim(),
            format!(
                "--output {} --exit-code-on-annotations origin/main HEAD",
                out.display()
            )
        );
    }

    #[test]
    fn tuicr_child_argv_reaches_the_child() {
        // Scenario "tuicr runs on the inherited tty", argv side: the child
        // receives exactly the scope fragment on the inherited stdio.
        let dir = TempDir::new("tuicr-argv");
        let log = dir.path().join("argv.log");
        let bin = dir.stub(
            "tuicr",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nexit 1\n",
                log.display()
            ),
        );
        let argv = child_argv(
            "tuicr",
            &bin.to_string_lossy(),
            "/s/1.out",
            &Scope::Uncommitted,
        );
        let child = spawn_child("tuicr", &argv, dir.path()).unwrap();
        assert_eq!(child.code, Some(1));
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "-w");
    }

    #[test]
    fn lumen_stdout_is_piped_and_captured() {
        // Scenario "lumen stdout is piped while stderr is inherited", capture
        // side: the stub's stdout lands in the capture (so it was piped),
        // and its stderr marker does not.
        let dir = TempDir::new("lumen-pipe");
        let bin = dir.stub(
            "lumen",
            "#!/bin/sh\nprintf 'export-marker'\nprintf 'stderr-marker' >&2\nexit 1\n",
        );
        let argv = child_argv(
            "lumen",
            &bin.to_string_lossy(),
            "/s/1.out",
            &Scope::Uncommitted,
        );
        let child = spawn_child("lumen", &argv, dir.path()).unwrap();
        assert_eq!(child.code, Some(1));
        assert_eq!(child.stdout, "export-marker");
    }

    #[test]
    fn killed_child_keeps_the_output() {
        // Scenario "Killed child keeps the output": a stub that writes its
        // output then kills itself reports no exit code; the collection
        // errors with the killed message and the file stays.
        let dir = TempDir::new("killed");
        let out = dir.path().join("1.out");
        let bin = dir.stub(
            "revdiff",
            &format!(
                "#!/bin/sh\nprintf '## f:1 (+)\\n' > '{}'\nkill -9 $$\n",
                out.display()
            ),
        );
        let argv = vec![bin.to_string_lossy().into_owned()];
        let child = spawn_child("revdiff", &argv, dir.path()).unwrap();
        assert!(child.code.is_none());
        let run = PaneRun {
            tool: tool::by_id("revdiff").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        let err = collect_and_finish(&run, &child).unwrap_err();
        assert!(
            err.contains("revdiff terminated without an exit code (killed?); output kept at"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "## f:1 (+)\n");
    }

    // --- revdiff collection ------------------------------------------------

    #[test]
    fn output_file_unreadable_after_a_clean_exit_is_empty() {
        // Scenario "Output file unreadable after a clean exit".
        let dir = TempDir::new("read-rc0");
        let missing = dir.path().join("missing.out");
        assert_eq!(read_output(0, &missing).unwrap(), "");
    }

    #[test]
    fn missing_output_at_an_annotations_exit_code_is_an_error() {
        // Scenario "Missing output at an annotations exit code".
        let dir = TempDir::new("read-rc10");
        let missing = dir.path().join("missing.out");
        let err = read_output(10, &missing).unwrap_err();
        assert!(err.contains("cannot read revdiff output"), "{err}");
        assert!(err.contains("missing.out"), "{err}");
    }

    #[test]
    fn invalid_utf8_in_the_output_file_is_replaced() {
        // Scenario "Invalid UTF-8 in the output file".
        let dir = TempDir::new("read-lossy");
        let path = dir.path().join("lossy.out");
        std::fs::write(&path, [b'f', 0xff, b'f']).unwrap();
        let out = read_output(10, &path).unwrap();
        assert!(out.contains('\u{fffd}'), "{out:?}");
    }

    // --- lumen collection --------------------------------------------------

    #[test]
    fn export_on_stdout_is_converted_and_written() {
        // Scenario "Export on stdout delivers", collection side: the
        // canonical conversion is written to the `.out` file.
        let dir = TempDir::new("lumen-export");
        let out = dir.path().join("1.out");
        let capture =
            "**src/a.rs** line 7 (RIGHT)\n\nrename this\n---\n**src/b.rs** lines 1-2 (LEFT)\n\nx\n";
        let canonical = collect_lumen_output(capture, &out).unwrap();
        assert_eq!(
            canonical,
            "## src/a.rs:7 (+)\nrename this\n\n## src/b.rs:1 (-)\nx\n"
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), canonical);
    }

    #[test]
    fn tui_closed_without_exporting_is_no_annotations() {
        // Scenario "TUI closed without exporting": blank capture collects
        // nothing (the `.out` removal happens in `finish`), and so does the
        // live-observed probe-only capture — the stripped remainder is blank.
        let dir = TempDir::new("lumen-blank");
        let out = dir.path().join("1.out");
        std::fs::write(&out, "").unwrap();
        assert_eq!(collect_lumen_output("", &out).unwrap(), "");
        assert_eq!(collect_lumen_output("  \n", &out).unwrap(), "");
        // The exact live byte sequence lumen emits at TUI startup (spike 1.2):
        // OSC `ESC]11;?BEL` + CSI `ESC[5n`.
        let probes = "\u{1b}]11;?\u{7}\u{1b}[5n";
        assert_eq!(probes.as_bytes(), b"\x1b]11;?\x07\x1b[5n");
        assert_eq!(collect_lumen_output(probes, &out).unwrap(), "");
        assert_eq!(
            collect_lumen_output(&format!("{probes}  \n"), &out).unwrap(),
            ""
        );
    }

    #[test]
    fn probe_preamble_only_capture_is_no_annotations() {
        // Scenario "TUI closed without exporting", probe-only variant: lumen
        // exits 0 with only the terminal-probe preamble on stdout (the user
        // closed the TUI with q/Esc, no `s`), so the outcome is
        // NoAnnotations — nothing is delivered, and the `.out` file is
        // removed instead of an empty prompt reaching the agent.
        let dir = TempDir::new("lumen-probe-only");
        let out = dir.path().join("1.out");
        std::fs::write(&out, "").unwrap();
        // The exact live byte sequence from the user's run.
        let child = ChildResult {
            code: Some(0),
            stdout: "\u{1b}]11;?\u{7}\u{1b}[5n".to_string(),
        };
        let run = PaneRun {
            tool: tool::by_id("lumen").unwrap(),
            bin: "",
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        // No caller agent: were the probe bytes still counted as
        // annotations, the outcome would be NoAgent and the file would stay.
        collect_and_finish(&run, &child).unwrap();
        assert!(!out.exists(), "no annotations removes the output file");
    }

    #[test]
    fn unparseable_export_keeps_raw() {
        // Scenario "Unparseable export keeps raw".
        let dir = TempDir::new("lumen-garbage");
        let out = dir.path().join("1.out");
        let err = collect_lumen_output("garbage capture\n", &out).unwrap_err();
        assert!(
            err.contains("cannot parse lumen annotation export; raw output kept at"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "garbage capture\n");
    }

    #[test]
    fn lumen_canonical_text_write_failure_is_fatal() {
        // Scenario "Lumen canonical text write failure is fatal".
        let dir = TempDir::new("lumen-write");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "file").unwrap();
        let out = blocker.join("reviews").join("1.out");
        let err = collect_lumen_output("**a.rs** line 1 (RIGHT)\n\nnote\n", &out).unwrap_err();
        assert!(err.contains("cannot write review output"), "{err}");
    }

    #[test]
    fn nonzero_lumen_exit_keeps_raw_capture() {
        // Scenario "Nonzero exit keeps raw capture": the raw capture is
        // written best-effort and the run fails with the code.
        let dir = TempDir::new("lumen-rc1");
        let out = dir.path().join("1.out");
        let bin = dir.stub("lumen", "#!/bin/sh\nprintf 'partial raw output'\nexit 2\n");
        let argv = child_argv(
            "lumen",
            &bin.to_string_lossy(),
            "/s/1.out",
            &Scope::Uncommitted,
        );
        let child = spawn_child("lumen", &argv, dir.path()).unwrap();
        let run = PaneRun {
            tool: tool::by_id("lumen").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        let err = collect_and_finish(&run, &child).unwrap_err();
        assert!(
            err.contains("lumen failed with exit code 2; output kept at"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "partial raw output");
    }

    #[test]
    fn lumen_blank_exit_is_no_annotations_and_removes_the_file() {
        let dir = TempDir::new("lumen-blank-exit");
        let out = dir.path().join("1.out");
        std::fs::write(&out, "").unwrap();
        let bin = dir.stub("lumen", "#!/bin/sh\nexit 0\n");
        let child = ChildResult {
            code: Some(0),
            stdout: String::new(),
        };
        let run = PaneRun {
            tool: tool::by_id("lumen").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        // A missing HERDR_PANE_ID makes the pane close a no-op; stdin under
        // the test harness is never held on this outcome.
        collect_and_finish(&run, &child).unwrap();
        assert!(!out.exists(), "no annotations removes the output file");
    }

    // --- tuicr collection --------------------------------------------------

    #[test]
    fn nonzero_tuicr_exit_skips_the_adapter_and_keeps_the_empty_file() {
        // Scenario "Nonzero tuicr exit skips the adapter": no adapter command
        // runs, the (empty) output file is kept, and the run fails with the
        // code.
        let dir = TempDir::new("tuicr-rc1");
        let log = dir.path().join("argv.log");
        let out = dir.path().join("1.out");
        let bin = dir.stub(
            "tuicr",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 1\n",
                log.display()
            ),
        );
        let argv = child_argv(
            "tuicr",
            &bin.to_string_lossy(),
            "/s/1.out",
            &Scope::Uncommitted,
        );
        let child = spawn_child("tuicr", &argv, dir.path()).unwrap();
        std::fs::write(&out, "").unwrap();
        let run = PaneRun {
            tool: tool::by_id("tuicr").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        let err = collect_and_finish(&run, &child).unwrap_err();
        assert!(
            err.contains("tuicr failed with exit code 1; output kept at"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "");
        // The stub saw only the TUI spawn, never a `review` adapter command.
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(!logged.contains("review"), "{logged}");
    }

    #[test]
    fn no_tuicr_comments_is_a_clean_no_annotations() {
        // Scenario "No comments is a clean no-annotations": the adapter's
        // empty list writes empty canonical text and the file is removed.
        let dir = TempDir::new("tuicr-empty");
        let out = dir.path().join("1.out");
        std::fs::write(&out, "").unwrap();
        let bin = dir.stub(
            "tuicr",
            "#!/bin/sh\nif [ \"$1\" = review ]; then printf '[]\\n'; fi\nexit 0\n",
        );
        let child = ChildResult {
            code: Some(0),
            stdout: String::new(),
        };
        let run = PaneRun {
            tool: tool::by_id("tuicr").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        collect_and_finish(&run, &child).unwrap();
        assert!(!out.exists(), "no annotations removes the output file");
    }

    #[test]
    fn tuicr_canonical_text_write_failure_is_fatal() {
        // Scenario "Tuicr canonical text write failure is fatal": the adapter
        // succeeds (an empty session list is a clean empty result) and the
        // canonical write into a blocked path fails.
        let dir = TempDir::new("tuicr-write");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "file").unwrap();
        let out = blocker.join("reviews").join("1.out");
        let bin = dir.stub(
            "tuicr",
            "#!/bin/sh\nif [ \"$1\" = review ]; then printf '[]\\n'; fi\nexit 0\n",
        );
        let child = ChildResult {
            code: Some(0),
            stdout: String::new(),
        };
        let run = PaneRun {
            tool: tool::by_id("tuicr").unwrap(),
            bin: &bin.to_string_lossy(),
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: None,
            caller_agent: None,
            caller_tab: None,
        };
        let err = collect_and_finish(&run, &child).unwrap_err();
        assert!(err.contains("cannot write review output"), "{err}");
    }

    // --- State directory and pruning ---------------------------------------

    #[test]
    fn output_path_layout() {
        // Scenario "Output path layout".
        let name = state_file_name(4242, 1_700_000_000_000);
        assert_eq!(name, "4242-1700000000000.out");
        assert_eq!(
            Path::new("/state").join("reviews").join(name),
            Path::new("/state/reviews/4242-1700000000000.out")
        );
    }

    #[test]
    fn state_directory_creation_failure_is_fatal() {
        // Scenario "State directory creation failure is fatal".
        let dir = TempDir::new("state-blocked");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "file").unwrap();
        let err = ensure_state_dir(&blocker.join("reviews")).unwrap_err();
        assert!(err.contains("failed to create state dir"), "{err}");
    }

    #[test]
    fn missing_state_dir_variable_uses_the_temp_directory() {
        // Scenario "Missing state-dir variable uses the temp directory".
        assert_eq!(state_dir_from(None), std::env::temp_dir());
        assert_eq!(state_dir_from(Some("   ")), std::env::temp_dir());
        assert_eq!(state_dir_from(Some("/state")), PathBuf::from("/state"));
        assert_eq!(state_dir_from(Some("  /state  ")), PathBuf::from("/state"));
    }

    #[test]
    fn system_clock_before_the_epoch_is_fatal() {
        // Scenario "System clock before the epoch is fatal".
        let err = millis_at(UNIX_EPOCH - Duration::from_secs(1)).unwrap_err();
        assert!(err.contains("system clock error"), "{err}");
        assert!(millis_at(UNIX_EPOCH).is_ok());
    }

    #[test]
    fn old_output_removed_fresh_output_kept() {
        // Scenario "Old output removed, fresh output kept".
        let dir = TempDir::new("prune");
        let old = dir.path().join("old.out");
        let fresh = dir.path().join("fresh.out");
        let keep = dir.path().join("keep.txt");
        std::fs::write(&old, "x").unwrap();
        std::fs::write(&fresh, "x").unwrap();
        std::fs::write(&keep, "x").unwrap();
        let now = SystemTime::now();
        set_mtime(&old, now - Duration::from_secs(8 * 24 * 60 * 60));
        set_mtime(&fresh, now);

        let mut warnings = Vec::new();
        prune_old_reviews_with(dir.path(), now, |w| warnings.push(w));

        assert!(!old.exists(), "old .out should be pruned");
        assert!(fresh.exists(), "fresh .out should be kept");
        assert!(keep.exists(), "non-.out entry should be kept");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn prune_missing_dir_is_noop() {
        let dir = TempDir::new("prune-missing");
        let missing = dir.path().join("does-not-exist");
        prune_old_reviews(&missing, SystemTime::now());
    }

    #[test]
    fn pruning_never_blocks_the_review() {
        // Scenario "Pruning never blocks the review": a failed removal warns
        // and the pane continues (nothing after the prune step fails).
        let dir = TempDir::new("prune-fail");
        let old = dir.path().join("old.out");
        std::fs::write(&old, "x").unwrap();
        let now = SystemTime::now();
        set_mtime(&old, now - Duration::from_secs(8 * 24 * 60 * 60));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        let mut warnings = Vec::new();
        prune_old_reviews_with(dir.path(), now, |w| warnings.push(w));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with("cannot remove") && warnings[0].contains("old.out"),
            "{warnings:?}"
        );
        assert!(old.exists(), "the unremovable file stays");
    }

    // --- Pane teardown -----------------------------------------------------

    #[test]
    fn pane_close_falls_back_to_tab_close() {
        // Scenario "Pane close falls back to tab close": the pane close
        // failure is logged and the tab close runs; a successful pane close
        // stops there; no pane id skips straight to the tab.
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let fail_pane = |pane: &str| -> Result<(), String> {
            calls.borrow_mut().push(format!("pane close {pane} failed"));
            Err("boom".to_string())
        };
        let ok_pane = |pane: &str| -> Result<(), String> {
            calls.borrow_mut().push(format!("pane close {pane} ok"));
            Ok(())
        };
        let close_tab = |tab: &str| -> Result<(), String> {
            calls.borrow_mut().push(format!("tab close {tab}"));
            Ok(())
        };
        close_pane_with(Some("pane-7"), Some("tab-7"), &fail_pane, &close_tab);
        assert_eq!(
            calls.borrow().clone(),
            ["pane close pane-7 failed", "tab close tab-7"]
        );
        calls.borrow_mut().clear();
        close_pane_with(Some("pane-7"), Some("tab-7"), &ok_pane, &close_tab);
        assert_eq!(calls.borrow().clone(), ["pane close pane-7 ok"]);
        calls.borrow_mut().clear();
        close_pane_with(None, Some("tab-7"), &fail_pane, &close_tab);
        assert_eq!(calls.borrow().clone(), ["tab close tab-7"]);
    }

    // --- Focus return to the caller tab ------------------------------------

    /// A revdiff run with annotations on disk and both caller values
    /// present, so the outcome is `Deliver`.
    fn deliver_run<'a>(
        dir: &'a TempDir,
        out: &'a Path,
        caller_tab: Option<&'a str>,
    ) -> PaneRun<'a> {
        std::fs::write(out, "## f:1 (+)\nnote\n").unwrap();
        PaneRun {
            tool: tool::by_id("revdiff").unwrap(),
            bin: "",
            dir: dir.path(),
            out_path: out,
            launched_at: SystemTime::now(),
            caller_pane: Some("caller-1"),
            caller_agent: Some("claude"),
            caller_tab,
        }
    }

    /// A `ChildResult` carrying just an exit code.
    fn rc_child(rc: i32) -> ChildResult {
        ChildResult {
            code: Some(rc),
            stdout: String::new(),
        }
    }

    #[test]
    fn delivered_run_refocuses_the_caller_tab() {
        // Scenario "Delivered run refocuses the caller tab": before the pane
        // close attempt the process runs `tab focus <caller tab>` best-effort
        // and exits 0.
        let dir = TempDir::new("focus-deliver");
        let out = dir.path().join("1.out");
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let run = deliver_run(&dir, &out, Some("w3X:t9"));
        let teardown = Teardown {
            deliver: &|target, _| {
                calls.borrow_mut().push(format!("agent prompt {target}"));
                Ok(())
            },
            close: &|| calls.borrow_mut().push("close pane".to_string()),
            focus: &|tab| {
                calls.borrow_mut().push(format!("tab focus {tab}"));
                Ok(())
            },
            warn: &|_| panic!("no warning on a clean focus"),
            hold: &|| panic!("a delivered run never holds"),
        };
        collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap();
        assert_eq!(
            calls.borrow().clone(),
            [
                "agent prompt caller-1".to_string(),
                "tab focus w3X:t9".to_string(),
                "close pane".to_string(),
            ]
        );
        assert!(!out.exists(), "delivery removes the output file");
    }

    #[test]
    fn no_annotations_run_refocuses_the_caller_tab() {
        // Scenario "No-annotations run refocuses the caller tab": the same
        // focus-then-close sequence runs and the process exits 0.
        let dir = TempDir::new("focus-noann");
        let out = dir.path().join("1.out");
        std::fs::write(&out, "").unwrap();
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let run = PaneRun {
            tool: tool::by_id("revdiff").unwrap(),
            bin: "",
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: Some("caller-1"),
            caller_agent: Some("claude"),
            caller_tab: Some("w3X:t9"),
        };
        let teardown = Teardown {
            deliver: &|_, _| panic!("nothing to deliver"),
            close: &|| calls.borrow_mut().push("close pane".to_string()),
            focus: &|tab| {
                calls.borrow_mut().push(format!("tab focus {tab}"));
                Ok(())
            },
            warn: &|_| panic!("no warning on a clean focus"),
            hold: &|| panic!("a no-annotations run never holds"),
        };
        collect_and_finish_with(&run, &rc_child(0), &teardown).unwrap();
        assert_eq!(
            calls.borrow().clone(),
            ["tab focus w3X:t9".to_string(), "close pane".to_string()]
        );
        assert!(!out.exists(), "no annotations removes the output file");
    }

    #[test]
    fn hold_and_error_paths_never_refocus() {
        // Scenario "Hold and error paths never refocus": `NoAgent`,
        // `Failed(rc)`, and a delivery failure make no focus call — the
        // focus seam panics if it ever runs.
        let dir = TempDir::new("focus-hold");
        let out = dir.path().join("1.out");

        // NoAgent: annotations exist but no agent — the pane holds, and no
        // teardown call runs at all.
        std::fs::write(&out, "## f:1 (+)\n").unwrap();
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let mut run = deliver_run(&dir, &out, Some("w3X:t9"));
        run.caller_agent = None;
        let teardown = Teardown {
            deliver: &|_, _| panic!("no delivery without an agent"),
            close: &|| panic!("the hold path never closes"),
            focus: &|_| panic!("the hold path never refocuses"),
            warn: &|_| panic!("no warning expected"),
            hold: &|| calls.borrow_mut().push("hold".to_string()),
        };
        collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap();
        assert_eq!(calls.borrow().clone(), ["hold".to_string()]);
        assert!(out.exists(), "no agent keeps the output file");

        // Failed(rc): the collection errors before any teardown call.
        std::fs::write(&out, "").unwrap();
        let run = PaneRun {
            tool: tool::by_id("tuicr").unwrap(),
            bin: "",
            dir: dir.path(),
            out_path: &out,
            launched_at: SystemTime::now(),
            caller_pane: Some("caller-1"),
            caller_agent: Some("claude"),
            caller_tab: Some("w3X:t9"),
        };
        let teardown = Teardown {
            deliver: &|_, _| panic!("a failed tool never delivers"),
            close: &|| panic!("the error path never closes"),
            focus: &|_| panic!("the error path never refocuses"),
            warn: &|_| panic!("no warning expected"),
            hold: &|| panic!("main's error path holds, not the collection"),
        };
        let err = collect_and_finish_with(&run, &rc_child(1), &teardown).unwrap_err();
        assert!(err.contains("tuicr failed with exit code 1"), "{err}");
        assert!(out.exists(), "a failed run keeps the output file");

        // Delivery failure: the error names the target and the kept file,
        // and no teardown call runs.
        std::fs::write(&out, "## f:1 (+)\n").unwrap();
        let run = deliver_run(&dir, &out, Some("w3X:t9"));
        let teardown = Teardown {
            deliver: &|_, _| Err("stub delivery failure".to_string()),
            close: &|| panic!("a failed delivery never closes"),
            focus: &|_| panic!("a failed delivery never refocuses"),
            warn: &|_| panic!("no warning expected"),
            hold: &|| panic!("main's error path holds, not the collection"),
        };
        let err = collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap_err();
        assert!(
            err.contains("failed to send annotations to agent pane caller-1"),
            "{err}"
        );
        assert!(out.exists(), "the failed delivery keeps the file");
    }

    #[test]
    fn focus_failure_warns_without_changing_the_outcome() {
        // Scenario "Focus failure warns without changing the outcome":
        // exactly one warning line, the exit code stays 0, and the
        // output-file handling is already complete.
        let dir = TempDir::new("focus-fail");
        let out = dir.path().join("1.out");
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let run = deliver_run(&dir, &out, Some("w3X:t9"));
        let teardown = Teardown {
            deliver: &|target, _| {
                calls.borrow_mut().push(format!("agent prompt {target}"));
                Ok(())
            },
            close: &|| calls.borrow_mut().push("close pane".to_string()),
            focus: &|_| Err("herdr focus failed".to_string()),
            warn: &|msg| calls.borrow_mut().push(format!("warning: {msg}")),
            hold: &|| panic!("the exit code stays 0"),
        };
        collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap();
        assert_eq!(
            calls.borrow().clone(),
            [
                "agent prompt caller-1".to_string(),
                "warning: failed to focus caller tab w3X:t9: herdr focus failed".to_string(),
                "close pane".to_string(),
            ]
        );
        assert_eq!(
            calls
                .borrow()
                .iter()
                .filter(|c| c.starts_with("warning: "))
                .count(),
            1,
            "exactly one warning line"
        );
        assert!(!out.exists(), "the file handling already completed");
    }

    #[test]
    fn missing_caller_tab_is_silently_skipped() {
        // Scenario "Missing caller tab is silently skipped": a clean ending
        // with no forwarded tab makes no focus call and prints no warning.
        let dir = TempDir::new("focus-missing");
        let out = dir.path().join("1.out");
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let run = deliver_run(&dir, &out, None);
        let teardown = Teardown {
            deliver: &|target, _| {
                calls.borrow_mut().push(format!("agent prompt {target}"));
                Ok(())
            },
            close: &|| calls.borrow_mut().push("close pane".to_string()),
            focus: &|_| panic!("no focus call without a caller tab"),
            warn: &|_| panic!("a missing caller tab is silent"),
            hold: &|| panic!("a delivered run never holds"),
        };
        collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap();
        assert_eq!(
            calls.borrow().clone(),
            [
                "agent prompt caller-1".to_string(),
                "close pane".to_string()
            ]
        );
        assert!(!out.exists(), "delivery removes the output file");
    }

    // --- Action flow -------------------------------------------------------

    /// A caller identity with every optional value set.
    fn full_identity() -> CallerIdentity {
        CallerIdentity {
            workspace: Some("ws-1".to_string()),
            pane: Some("caller-1".to_string()),
            agent: Some("claude".to_string()),
            tab: Some("caller-tab-1".to_string()),
        }
    }

    #[test]
    fn configured_installed_tool_opens_the_review_pane_directly() {
        // Scenario "Configured installed tool opens the review pane
        // directly": a configured tool whose executable is detected on PATH
        // opens the review pane — no popup is shown.
        let dir = TempDir::new("action-direct");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let opened: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                ..Config::default()
            },
            open: &|config, dir, tool_id, workspace, pane, agent, tab| {
                opened.borrow_mut().push(format!(
                    "{:?}|{dir}|{tool_id}|{workspace:?}|{pane:?}|{agent:?}|{tab:?}",
                    config.placement
                ));
                // An unparsable reply: the open already succeeded, the log
                // line is skipped.
                Ok(None)
            },
            open_setup: &|| panic!("a usable configured tool must not open the setup dialog"),
            focus: &|_| panic!("no focus call on an unparsable reply"),
            log: &|_| panic!("an unparsable reply already warned and skips the log line"),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(opened.borrow().len(), 1, "{:?}", opened.borrow());
        let open = opened.borrow()[0].clone();
        assert!(open.starts_with("Tab|"), "{open}");
        assert!(open.contains("|/repo|revdiff|"), "{open}");
        assert!(open.contains("Some(\"ws-1\")"), "{open}");
        assert!(open.contains("Some(\"caller-1\")"), "{open}");
        assert!(open.contains("Some(\"claude\")"), "{open}");
        // The resolved caller tab travels on the direct open as the
        // HERDR_REVIEWS_CALLER_TAB pair (argv shape pinned by the herdr
        // tests).
        assert!(open.contains("Some(\"caller-tab-1\")"), "{open}");
    }

    #[test]
    fn unset_tool_opens_the_setup_dialog() {
        // Scenario "Unset tool opens the setup dialog": the tool key is
        // unset, so the setup popup opens and no review pane does.
        let dir = TempDir::new("action-unset");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let setup_opens: RefCell<usize> = RefCell::new(0);
        let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config::default(),
            open: &|_, _, _, _, _, _, _| panic!("an unset tool must not open the review pane"),
            open_setup: &|| {
                *setup_opens.borrow_mut() += 1;
                Ok(())
            },
            focus: &|_| panic!("no focus call on the setup path"),
            log: &|msg| logged.borrow_mut().push(msg.to_string()),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(*setup_opens.borrow(), 1);
        assert_eq!(
            logged.borrow().clone(),
            ["herdr-review: opened review tool setup".to_string()]
        );
    }

    #[test]
    fn configured_missing_tool_opens_the_setup_dialog() {
        // Scenario "Configured missing tool opens the setup dialog": the
        // configured id is valid but no executable with its name is on PATH,
        // so the setup popup opens and no review pane does.
        let dir = TempDir::new("action-missing");
        // Only revdiff is installed; the config names lumen.
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let setup_opens: RefCell<usize> = RefCell::new(0);
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("lumen").unwrap()),
                ..Config::default()
            },
            open: &|_, _, _, _, _, _, _| {
                panic!("a missing configured tool must not open the review pane")
            },
            open_setup: &|| {
                *setup_opens.borrow_mut() += 1;
                Ok(())
            },
            focus: &|_| panic!("no focus call on the setup path"),
            log: &|_| {},
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(*setup_opens.borrow(), 1);
    }

    #[test]
    fn invalid_tool_value_opens_the_setup_dialog_after_a_warning() {
        // Scenario "Invalid tool value opens the setup dialog after a
        // warning": the config layer warns and treats the key as unset (the
        // warning itself is pinned by the config tests), so the action sees
        // no tool and opens the setup popup.
        let dir = TempDir::new("action-invalid");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let setup_opens: RefCell<usize> = RefCell::new(0);
        let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            // `tool = "diffy"` degrades to exactly this config.
            load_config: &|| Config::default(),
            open: &|_, _, _, _, _, _, _| {
                panic!("an invalid tool value must not open the review pane")
            },
            open_setup: &|| {
                *setup_opens.borrow_mut() += 1;
                Ok(())
            },
            focus: &|_| panic!("no focus call on the setup path"),
            log: &|msg| logged.borrow_mut().push(msg.to_string()),
            warn: &|_| panic!("the warning comes from the config layer, not the flow"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(*setup_opens.borrow(), 1);
        assert_eq!(
            logged.borrow().clone(),
            ["herdr-review: opened review tool setup".to_string()]
        );
    }

    #[test]
    fn action_success_opens_the_popup_and_exits() {
        // Scenario "Action success opens the popup and exits": the
        // setup-popup open exits 0, the log line names the setup, and no
        // focus call or hold follows — the flow returns success.
        let setup_opens: RefCell<usize> = RefCell::new(0);
        let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config::default(),
            open: &|_, _, _, _, _, _, _| panic!("the setup path opens no review pane"),
            open_setup: &|| {
                *setup_opens.borrow_mut() += 1;
                Ok(())
            },
            focus: &|_| panic!("no focus call on the setup path"),
            log: &|msg| logged.borrow_mut().push(msg.to_string()),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(&full_identity(), "/repo", "", &deps).unwrap();
        assert_eq!(*setup_opens.borrow(), 1);
        assert_eq!(
            logged.borrow().clone(),
            ["herdr-review: opened review tool setup".to_string()]
        );
    }

    #[test]
    fn pane_opens_with_a_tab() {
        // Scenario "Pane opens with a tab": the direct open's parsed ids log
        // the pane and tab, and a best-effort tab focus runs — a focus
        // failure does not affect the outcome.
        let dir = TempDir::new("action-tab");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let focused: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                ..Config::default()
            },
            open: &|_, _, _, _, _, _, _| {
                Ok(Some(herdr::PaneIds {
                    pane_id: "pane-8".to_string(),
                    tab_id: Some("tab-8".to_string()),
                }))
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|tab| {
                focused.borrow_mut().push(tab.to_string());
                Ok(())
            },
            log: &|msg| logged.borrow_mut().push(msg.to_string()),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(focused.borrow().clone(), ["tab-8".to_string()]);
        assert_eq!(
            logged.borrow().clone(),
            ["herdr-review: opened review pane pane-8 in tab tab-8".to_string()]
        );
        // The focus re-assert is best-effort: a failure changes nothing.
        focused.borrow_mut().clear();
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                ..Config::default()
            },
            open: &|_, _, _, _, _, _, _| {
                Ok(Some(herdr::PaneIds {
                    pane_id: "pane-8".to_string(),
                    tab_id: Some("tab-8".to_string()),
                }))
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|tab| {
                focused.borrow_mut().push(tab.to_string());
                Err("rendering race".to_string())
            },
            log: &|_| {},
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(focused.borrow().clone(), ["tab-8".to_string()]);
    }

    #[test]
    fn pane_without_a_tab_makes_no_focus_call() {
        // Scenario "Overlay-style reply without a tab": a valid pane id with
        // an absent tab id proceeds with the tab treated as absent — the log
        // line names only the pane, and no tab-focus call runs.
        let dir = TempDir::new("action-no-tab");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                ..Config::default()
            },
            open: &|_, _, _, _, _, _, _| {
                Ok(Some(herdr::PaneIds {
                    pane_id: "pane-8".to_string(),
                    tab_id: None,
                }))
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|_| panic!("no focus call without a tab id"),
            log: &|msg| logged.borrow_mut().push(msg.to_string()),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(
            logged.borrow().clone(),
            ["herdr-review: opened review pane pane-8".to_string()]
        );
    }

    #[test]
    fn split_or_zoomed_without_a_caller_pane_falls_back_to_tab() {
        // Scenario "Split or zoomed without a caller pane falls back to tab":
        // the exact warning names the degraded placement, and the open
        // receives the tab shape — no direction, no target.
        let dir = TempDir::new("action-fallback");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let mut identity = full_identity();
        identity.pane = None;
        for (configured, name) in [(Placement::Split, "split"), (Placement::Zoomed, "zoomed")] {
            let warned: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let opened: RefCell<Vec<Placement>> = RefCell::new(Vec::new());
            let deps = ActionDeps {
                load_config: &|| Config {
                    tool: Some(tool::by_id("revdiff").unwrap()),
                    placement: configured,
                    direction: Direction::Down,
                },
                open: &|config, _, _, _, _, _, _| {
                    opened.borrow_mut().push(config.placement);
                    Ok(None)
                },
                open_setup: &|| panic!("the direct open must not open the setup dialog"),
                focus: &|_| panic!("no focus call on an unparsable reply"),
                log: &|_| panic!("an unparsable reply already warned and skips the log line"),
                warn: &|msg| warned.borrow_mut().push(msg.to_string()),
            };
            run_action_with(&identity, "/repo", &dir.path().to_string_lossy(), &deps).unwrap();
            assert_eq!(warned.borrow().len(), 1, "{:?}", warned.borrow());
            assert_eq!(
                warned.borrow()[0],
                format!("placement \"{name}\" needs a caller pane; using \"tab\"")
            );
            assert_eq!(opened.borrow().clone(), [Placement::Tab]);
        }
    }

    #[test]
    fn split_with_a_caller_pane_stays_split() {
        // The degrade fires only without a caller pane: with one, the
        // configured split placement reaches the open unchanged and no
        // warning is printed.
        let dir = TempDir::new("action-split-kept");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let opened: RefCell<Vec<Placement>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                placement: Placement::Split,
                direction: Direction::Down,
            },
            open: &|config, _, _, _, _, _, _| {
                opened.borrow_mut().push(config.placement);
                Ok(None)
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|_| panic!("no focus call on an unparsable reply"),
            log: &|_| panic!("an unparsable reply already warned and skips the log line"),
            warn: &|_| panic!("no warning with a caller pane"),
        };
        run_action_with(
            &full_identity(),
            "/repo",
            &dir.path().to_string_lossy(),
            &deps,
        )
        .unwrap();
        assert_eq!(opened.borrow().clone(), [Placement::Split]);
    }

    #[test]
    fn pane_id_taken_from_the_environment() {
        // Scenario "Pane id taken from the environment": the context JSON
        // has no non-blank `focused_pane_id` and `HERDR_PANE_ID` is set, so
        // the action uses that value as the caller pane identifier — it
        // reaches the review-pane open, whose argv carries it as
        // `--target-pane` and the `--env HERDR_REVIEWS_CALLER_PANE` pair
        // (pinned by the herdr-layer argv tests).
        let ctx = context::Context {
            dir: "/repo".to_string(),
            workspace_id: Some("ws-1".to_string()),
            focused_pane_id: None,
            focused_pane_agent: Some("claude".to_string()),
            tab_id: None,
        };
        let identity = CallerIdentity::resolve(&ctx, None, Some("p-from-env"), None);
        assert_eq!(identity.pane.as_deref(), Some("p-from-env"));
        assert_eq!(identity.tab, None);
        let dir = TempDir::new("action-pane-env");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let opened: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                placement: Placement::Split,
                direction: Direction::Down,
            },
            open: &|config, dir, tool_id, workspace, pane, agent, tab| {
                opened.borrow_mut().push(format!(
                    "{:?}|{dir}|{tool_id}|{workspace:?}|{pane:?}|{agent:?}|{tab:?}",
                    config.placement
                ));
                Ok(None)
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|_| panic!("no focus call on an unparsable reply"),
            log: &|_| panic!("an unparsable reply already warned and skips the log line"),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(&identity, "/repo", &dir.path().to_string_lossy(), &deps).unwrap();
        let open = opened.borrow()[0].clone();
        assert!(open.starts_with("Split|"), "{open}");
        assert!(open.contains("Some(\"p-from-env\")"), "{open}");
        assert!(open.contains("Some(\"claude\")"), "{open}");
    }

    #[test]
    fn agent_without_context_entry_is_simply_absent() {
        // Scenario "Agent without context entry is simply absent": the
        // context JSON has no non-blank `focused_pane_agent`, so the action
        // treats the caller agent as absent even if any environment variable
        // names an agent — the identity resolution reads the agent from the
        // context only, and the open omits the agent pair.
        let ctx = context::Context {
            dir: "/repo".to_string(),
            workspace_id: Some("ws-1".to_string()),
            focused_pane_id: Some("caller-1".to_string()),
            focused_pane_agent: None,
            tab_id: None,
        };
        let identity = CallerIdentity::resolve(&ctx, Some("ws-env"), Some("pane-env"), None);
        assert_eq!(identity.agent, None);
        assert_eq!(identity.workspace.as_deref(), Some("ws-1"));
        assert_eq!(identity.pane.as_deref(), Some("caller-1"));
        assert_eq!(identity.tab, None);
        let dir = TempDir::new("action-no-agent");
        let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
        let opened: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let deps = ActionDeps {
            load_config: &|| Config {
                tool: Some(tool::by_id("revdiff").unwrap()),
                ..Config::default()
            },
            open: &|_, dir, tool_id, workspace, pane, agent, tab| {
                opened.borrow_mut().push(format!(
                    "{dir}|{tool_id}|{workspace:?}|{pane:?}|{agent:?}|{tab:?}"
                ));
                Ok(None)
            },
            open_setup: &|| panic!("the direct open must not open the setup dialog"),
            focus: &|_| panic!("no focus call on an unparsable reply"),
            log: &|_| panic!("an unparsable reply already warned and skips the log line"),
            warn: &|_| panic!("no warning expected"),
        };
        run_action_with(&identity, "/repo", &dir.path().to_string_lossy(), &deps).unwrap();
        let open = opened.borrow()[0].clone();
        assert!(open.ends_with("|None"), "the agent pair is absent: {open}");
        assert!(!open.contains("claude"), "{open}");
    }

    #[test]
    fn caller_tab_resolves_from_the_context_with_the_env_fallback() {
        // The caller tab resolution mirrors the caller pane: the context
        // `tab_id` wins, `HERDR_TAB_ID` is the fallback, and blank counts as
        // absent (the pair is then omitted from the review-pane open).
        let ctx = context::Context {
            dir: "/repo".to_string(),
            workspace_id: None,
            focused_pane_id: None,
            focused_pane_agent: None,
            tab_id: Some("tab-ctx".to_string()),
        };
        let identity = CallerIdentity::resolve(&ctx, None, None, Some("tab-env"));
        assert_eq!(identity.tab.as_deref(), Some("tab-ctx"));
        let ctx = context::Context {
            dir: "/repo".to_string(),
            workspace_id: None,
            focused_pane_id: None,
            focused_pane_agent: None,
            tab_id: None,
        };
        let identity = CallerIdentity::resolve(&ctx, None, None, Some("tab-env"));
        assert_eq!(identity.tab.as_deref(), Some("tab-env"));
        let identity = CallerIdentity::resolve(&ctx, None, None, None);
        assert_eq!(identity.tab, None);
        // Blank env values never reach here: `run_action` reads the fallback
        // through `env_non_blank`, which trims and drops them.
    }
}
