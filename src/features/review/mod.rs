//! Review orchestration.
//!
//! The binary is tri-mode. Action mode parses the plugin context, resolves
//! the caller identity, reads `config.toml` once, and opens the review pane
//! directly when exactly one supported tool is installed; multiple tools
//! open the picker popup, and no installed tools open the read-only setup popup —
//! then exits. Setup mode (the popup
//! pane, see [`crate::features::setup`]) never opens a pane itself. Review-pane mode
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::adapters::git::{GitRepository, Scope};
use crate::adapters::herdr;
use crate::adapters::tool::{self, ReviewTool};
use crate::adapters::tuicr;
use crate::settings;

pub(crate) mod annotations;
mod context;
mod outcome;
pub(crate) mod ports;
mod process;

use self::annotations as convert;

use outcome::{Outcome, decide, prompt_text};
use ports::{
    OpenReview, OpenToolPicker, ReviewActionHost, ReviewDeliveryHost, ReviewRepository,
    ReviewToolRunner,
};
use process::{ChildProcess, ChildResult, child_argv};
#[cfg(test)]
use process::{captures_stdout, revdiff_argv, spawn_child};

mod action;
#[cfg(test)]
mod tests;

pub(crate) use action::run_action;
#[cfg(test)]
use action::{ActionDeps, CallerIdentity, run_action_with};

/// Printed before blocking on stdin so an auto-closing plugin pane stays open
/// on error and no-agent paths.
const HOLD_PROMPT: &str = "herdr-review: press Enter to close";
/// Kept review outputs older than this are pruned at pane start.
const REVIEW_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Cap on the delivered agent prompt, well under ARG_MAX (1 MiB macOS / 2 MiB
/// Linux): annotations travel as a single argv element.
const MAX_PROMPT_BYTES: usize = 65536;

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

/// Everything the collection stage needs about one review run.
struct ReviewSession<'a> {
    tool: ReviewTool,
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

type PaneRun<'a> = ReviewSession<'a>;

impl ReviewSession<'_> {
    fn collect_and_finish(&self, child: &ChildResult) -> Result<(), String> {
        self.collect_and_finish_with(child, &real_teardown())
    }

    fn collect_and_finish_with(
        &self,
        child: &ChildResult,
        teardown: &dyn ReviewDeliveryHost,
    ) -> Result<(), String> {
        let tool = self.tool;
        let Some(rc) = child.code else {
            if tool == ReviewTool::Lumen {
                keep_raw(self.out_path, &child.stdout);
            }
            return Err(format!(
                "{} terminated without an exit code (killed?); output kept at {}",
                tool.id(),
                self.out_path.display()
            ));
        };
        match tool {
            ReviewTool::Revdiff => {
                let output = read_output(rc, self.out_path)?;
                let annotations = rc == 10 || (rc == 0 && !output.trim().is_empty());
                finish(self, rc, annotations, &output, teardown)
            }
            ReviewTool::Tuicr => {
                if rc != 0 {
                    return finish(self, rc, false, "", teardown);
                }
                let comments = tuicr::TuicrClient::new(self.bin, self.dir)
                    .collect_comments(self.launched_at)
                    .map_err(|detail| {
                        format!(
                            "tuicr session lookup failed: {detail}; output kept at {}",
                            self.out_path.display()
                        )
                    })?;
                let canonical = convert::tuicr_to_canonical(&comments);
                write_output(self.out_path, &canonical)?;
                finish(self, rc, !comments.is_empty(), &canonical, teardown)
            }
            ReviewTool::Lumen => {
                if rc != 0 {
                    keep_raw(self.out_path, &child.stdout);
                    return finish(self, rc, false, "", teardown);
                }
                let (canonical, annotations) = collect_lumen_output(&child.stdout, self.out_path)?;
                finish(self, rc, annotations, &canonical, teardown)
            }
        }
    }
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
    let repository = GitRepository::new(&dir);
    ReviewPane {
        repository: &repository,
        runner: &ChildProcess,
    }
    .run(&dir, tool)
}

struct ReviewPane<'a> {
    repository: &'a dyn ReviewRepository,
    runner: &'a dyn ReviewToolRunner,
}

impl ReviewPane<'_> {
    fn run(&self, dir: &Path, tool: ReviewTool) -> Result<(), String> {
        let reports = state_dir().join("reviews");
        self.run_in(dir, tool, &reports, || tool::resolve(tool))
    }

    fn run_in(
        &self,
        dir: &Path,
        tool: ReviewTool,
        reports: &Path,
        resolve_bin: impl FnOnce() -> Result<String, String>,
    ) -> Result<(), String> {
        let scope = self.repository.scope().for_tool(tool);
        match &scope {
            // A failed probe already printed its warning; either way no tool runs
            // and no output file is created — hold the pane and exit cleanly.
            Scope::NothingToReview | Scope::CleanCommitted { .. } => {
                println!("herdr-review: nothing to review");
                hold_pane();
                return Ok(());
            }
            Scope::Uncommitted { .. } => println!("herdr-review: reviewing uncommitted changes"),
            Scope::AllFiles => println!("herdr-review: reviewing all files"),
            Scope::VsBase(base) => println!("herdr-review: reviewing vs {base}"),
        }

        ensure_state_dir(reports)?;
        prune_old_reviews(reports, SystemTime::now());
        let out_path = reports.join(state_file_name(std::process::id(), unix_millis()?));
        // revdiff writes the file itself; for tuicr and lumen the plugin owns the
        // content, so an empty file exists before the spawn and failure paths
        // keep evidence on disk.
        if tool != ReviewTool::Revdiff {
            write_output(&out_path, "")?;
        }
        let bin = resolve_bin()?;
        let argv = child_argv(tool, &bin, &out_path.to_string_lossy(), &scope);
        let caller_pane = env_non_blank("HERDR_REVIEWS_CALLER_PANE");
        let caller_agent = env_non_blank("HERDR_REVIEWS_CALLER_AGENT");
        let caller_tab = env_non_blank("HERDR_REVIEWS_CALLER_TAB");
        // The launch timestamp precedes the spawn so the tuicr adapter can tell
        // this run's sessions from older ones.
        let run = ReviewSession {
            tool,
            bin: &bin,
            dir,
            out_path: &out_path,
            launched_at: SystemTime::now(),
            caller_pane: caller_pane.as_deref(),
            caller_agent: caller_agent.as_deref(),
            caller_tab: caller_tab.as_deref(),
        };
        let child = self.runner.run(tool, &argv, dir)?;
        run.collect_and_finish(&child)
    }
}

/// Per-tool collection and outcome: reads revdiff's output file, runs the
/// tuicr session adapter, converts lumen's captured stdout, and maps each to
/// the delivery decision.
#[cfg(test)]
fn collect_and_finish(run: &PaneRun, child: &ChildResult) -> Result<(), String> {
    run.collect_and_finish(child)
}

/// [`collect_and_finish`] with the process-bound herdr calls injected, so
/// the clean-ending teardown is testable without the environment.
#[cfg(test)]
fn collect_and_finish_with(
    run: &PaneRun,
    child: &ChildResult,
    teardown: &dyn ReviewDeliveryHost,
) -> Result<(), String> {
    run.collect_and_finish_with(child, teardown)
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
fn collect_lumen_output(capture: &str, out_path: &Path) -> Result<(String, bool), String> {
    let stripped = convert::strip_terminal_escapes(capture);
    if stripped.trim().is_empty() {
        return Ok((String::new(), false));
    }
    match convert::lumen_to_canonical_stripped(&stripped) {
        Some(canonical) => {
            write_output(out_path, &canonical)?;
            Ok((canonical, true))
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
    teardown: &dyn ReviewDeliveryHost,
) -> Result<(), String> {
    let tool = run.tool;
    let has_agent = run.caller_pane.is_some() && run.caller_agent.is_some();
    match decide(tool, rc, annotations, has_agent) {
        Outcome::Deliver => {
            // `decide` returns Deliver only when both caller pane and agent are
            // present, so the pane is always Some here; avoid expect() anyway.
            let Some(target) = run.caller_pane else {
                return Err("internal: deliver outcome without a caller pane".to_string());
            };
            let prompt = cap_prompt(&prompt_text(tool, output), MAX_PROMPT_BYTES, run.out_path);
            match teardown.deliver(target, &prompt) {
                Ok(()) => {
                    eprintln!("herdr-review: sent review annotations to agent pane {target}");
                    remove(run.out_path);
                    // Focus BEFORE the close: the daemon kills a pane process
                    // at the moment it closes its own pane, so a post-close
                    // focus call never runs (live herdr 0.9.5 finding).
                    refocus_caller_tab(run.caller_tab, teardown);
                    teardown.close();
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
            teardown.close();
            Ok(())
        }
        Outcome::NoAgent => {
            eprintln!(
                "herdr-review: no agent in focused pane; annotations kept at {}",
                run.out_path.display()
            );
            teardown.hold();
            Ok(())
        }
        Outcome::Failed(code) => Err(format!(
            "{} failed with exit code {code}; output kept at {}",
            tool.id(),
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

impl ReviewDeliveryHost for Teardown<'_> {
    fn deliver(&self, pane_id: &str, prompt: &str) -> Result<(), String> {
        (self.deliver)(pane_id, prompt)
    }

    fn close(&self) {
        (self.close)();
    }

    fn focus(&self, tab_id: &str) -> Result<(), String> {
        (self.focus)(tab_id)
    }

    fn warn(&self, message: &str) {
        (self.warn)(message);
    }

    fn hold(&self) {
        (self.hold)();
    }
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
fn refocus_caller_tab(tab: Option<&str>, host: &dyn ReviewDeliveryHost) {
    if let Some(tab) = tab
        && let Err(e) = host.focus(tab)
    {
        host.warn(&format!("failed to focus caller tab {tab}: {e}"));
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
