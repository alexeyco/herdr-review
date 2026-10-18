//! Minimal client over the herdr CLI (`HERDR_BIN_PATH`).
//!
//! All herdr calls go through the binary at `HERDR_BIN_PATH` — never a bare
//! `herdr`, because the plugin process may run with a different `PATH`. The
//! two `plugin pane open` shapes this plugin uses are the setup popup (exit
//! status only, reply body ignored — a popup reply carries no pane id) and
//! the review pane (reply parsed best-effort: exit status 0 already means the
//! pane exists, so an unparsable reply is a warning, not an error).

use std::process::Command;

use serde_json::Value;

use crate::config::{Config, Placement};

/// The plugin's manifest id — the `--plugin` value of every pane open and
/// the key of the `plugin config-dir` callback.
const PLUGIN_ID: &str = "alexeyco.review";

/// The successful reply of a review-pane open, parsed best-effort.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PaneIds {
    /// `result.plugin_pane.pane.pane_id`, non-blank.
    pub(crate) pane_id: String,
    /// `result.plugin_pane.pane.tab_id`, absent for overlay-style panes.
    pub(crate) tab_id: Option<String>,
}

/// Runs the herdr binary with `args`, returning stdout on success.
fn exec(args: &[&str]) -> Result<String, String> {
    exec_with_bin(&herdr_bin()?, args)
}

/// [`exec`] against an explicit binary, so tests drive the CLI failure modes
/// without mutating the process environment.
fn exec_with_bin(bin: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {} {}: {e}", bin, args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "{} {} failed ({}): {}",
            bin,
            args.join(" "),
            output.status,
            snippet(&stderr)
        ))
    }
}

fn herdr_bin() -> Result<String, String> {
    herdr_bin_from(std::env::var("HERDR_BIN_PATH").ok().as_deref())
}

/// [`herdr_bin`] against an already-read raw env value.
fn herdr_bin_from(raw: Option<&str>) -> Result<String, String> {
    match raw.map(str::trim) {
        Some(v) if !v.is_empty() => Ok(v.to_string()),
        _ => Err(
            "herdr did not provide HERDR_BIN_PATH (plugin API v1 contract); cannot call back into herdr"
                .to_string(),
        ),
    }
}

/// Fails early when `HERDR_BIN_PATH` is missing. Pane mode calls this before
/// starting the review tool so a bad environment is reported before the TUI
/// opens.
pub(crate) fn require_bin() -> Result<(), String> {
    herdr_bin().map(|_| ())
}

/// Opens the read-only setup popup: the exit status alone decides success —
/// a popup reply carries no pane id, so the body is not parsed at all.
pub(crate) fn open_setup_popup() -> Result<(), String> {
    let args = setup_pane_open_args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec(&refs).map(|_| ())
}

/// Opens the review pane after a tool is chosen. On exec success the reply is
/// parsed best-effort: a non-blank `result.plugin_pane.pane.pane_id` enables
/// the success log line and `tab_id` is optional; an unparsable reply or a
/// missing pane id logs a warning and returns `Ok(None)` — exit status 0
/// already means the pane exists.
pub(crate) fn open_review_pane(
    config: &Config,
    workspace_id: Option<&str>,
    cwd: &str,
    tool: &str,
    caller_pane: Option<&str>,
    caller_agent: Option<&str>,
    caller_tab: Option<&str>,
) -> Result<Option<PaneIds>, String> {
    let args = review_pane_open_args(
        config,
        workspace_id,
        cwd,
        tool,
        caller_pane,
        caller_agent,
        caller_tab,
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = exec(&refs)?;
    Ok(parse_review_pane_open(&out))
}

/// Queries the plugin config directory with `herdr plugin config-dir
/// alexeyco.review`. The reply is a bare path line (not a JSON envelope), so
/// nothing is parsed beyond trimming stdout: a single non-blank line is the
/// directory. A blank or multi-line reply, or a failed exec, is an `Err` the
/// config layer maps to one warning plus defaults — never a launch failure.
pub(crate) fn config_dir() -> Result<String, String> {
    let out = exec(&["plugin", "config-dir", PLUGIN_ID])?;
    parse_config_dir(&out)
}

/// Parses the bare-path `plugin config-dir` reply: trimmed stdout must be
/// exactly one non-blank line, which is the directory.
fn parse_config_dir(out: &str) -> Result<String, String> {
    let mut lines = out
        .trim()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let dir = lines
        .next()
        .ok_or_else(|| "herdr plugin config-dir returned a blank reply".to_string())?;
    if lines.next().is_some() {
        let raw = snippet(out);
        return Err(format!(
            "herdr plugin config-dir returned more than one line: {raw}"
        ));
    }
    Ok(dir.to_string())
}

/// `herdr plugin pane open` argv for the setup popup, exactly as the spec
/// pins: fixed plugin/entrypoint/placement and `--focus`. No `--workspace`
/// flag (the herdr 0.9.5 daemon rejects popup opens that target a workspace
/// with `invalid_params`, live-probed), no `--cwd`, and no `--env` pairs:
/// the dialog re-reads the config and `PATH` itself.
pub(crate) fn setup_pane_open_args() -> Vec<String> {
    let mut args = base_pane_open_args("setup", "popup");
    args.push("--focus".to_string());
    args
}

/// `herdr plugin pane open` argv for the review pane, in the exact order the
/// spec pins: fixed plugin/entrypoint and the config-driven `--placement`,
/// the conditional `--direction` (only when placement is `split` and the
/// direction is right/down), the conditional `--target-pane` (only for
/// `split`/`zoomed`, from the resolved caller pane — the daemon rejects
/// those placements targetless), the conditional `--workspace` (only when
/// placement is `tab` — the daemon rejects it on every pane-anchored
/// placement), `--cwd`, the always-present `HERDR_REVIEWS_DIR` and
/// `HERDR_REVIEW_TOOL` exports, the conditional caller pairs — pane, agent,
/// then tab (the review pane returns focus to the caller tab after a clean
/// ending) — and `--focus`.
///
/// The manifest's `placement = "tab"` for the `review` pane is a separate
/// declaration that can drift from this argv: this arg is what the action
/// actually passes at open time. The split/zoomed-to-tab degrade is not done
/// here — the action warns and degrades before the open, so a split/zoomed
/// argv reaching this builder always carries a caller pane.
pub(crate) fn review_pane_open_args(
    config: &Config,
    workspace_id: Option<&str>,
    cwd: &str,
    tool: &str,
    caller_pane: Option<&str>,
    caller_agent: Option<&str>,
    caller_tab: Option<&str>,
) -> Vec<String> {
    let mut args = base_pane_open_args("review", config.placement.as_str());
    if config.placement == Placement::Split
        && let Some(direction) = config.direction.argv_value()
    {
        args.push("--direction".to_string());
        args.push(direction.to_string());
    }
    // split/zoomed anchor on the caller pane; tab/overlay never carry the
    // flag. Probes against herdr 0.9.5: those placements reject the open
    // with `invalid_params` without a `--target-pane`.
    if matches!(config.placement, Placement::Split | Placement::Zoomed)
        && let Some(pane) = caller_pane
    {
        args.push("--target-pane".to_string());
        args.push(pane.to_string());
    }
    // The daemon rejects `--workspace` on every non-tab placement
    // (`invalid_params`); the action's split/zoomed degrade lands on the tab
    // shape, which is the one placement the pair is legal on.
    if config.placement == Placement::Tab
        && let Some(ws) = workspace_id
    {
        args.push("--workspace".to_string());
        args.push(ws.to_string());
    }
    args.push("--cwd".to_string());
    args.push(cwd.to_string());
    args.push("--env".to_string());
    args.push(format!("HERDR_REVIEWS_DIR={cwd}"));
    args.push("--env".to_string());
    args.push(format!("HERDR_REVIEW_TOOL={tool}"));
    if let Some(pane) = caller_pane {
        args.push("--env".to_string());
        args.push(format!("HERDR_REVIEWS_CALLER_PANE={pane}"));
    }
    if let Some(agent) = caller_agent {
        args.push("--env".to_string());
        args.push(format!("HERDR_REVIEWS_CALLER_AGENT={agent}"));
    }
    // The caller tab pair rides last of the caller pairs, right before
    // `--focus`: the review pane uses it to return focus after a clean
    // ending. No tab resolved ⇒ the pair is omitted.
    if let Some(tab) = caller_tab {
        args.push("--env".to_string());
        args.push(format!("HERDR_REVIEWS_CALLER_TAB={tab}"));
    }
    args.push("--focus".to_string());
    args
}

/// The fixed prefix of a pane-open argv: `plugin pane open --plugin
/// alexeyco.review --entrypoint <entrypoint> --placement <placement>`.
fn base_pane_open_args(entrypoint: &str, placement: &str) -> Vec<String> {
    [
        "plugin",
        "pane",
        "open",
        "--plugin",
        PLUGIN_ID,
        "--entrypoint",
        entrypoint,
        "--placement",
        placement,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Parses the review-pane open reply best-effort. A non-blank
/// `result.plugin_pane.pane.pane_id` and an optional `tab_id` succeed; a
/// blank tab id counts as absent (overlay-style reply). Anything else logs
/// one `herdr-review: warning:` line quoting a 200-char snippet and yields
/// `None`.
fn parse_review_pane_open(json: &str) -> Option<PaneIds> {
    let raw = snippet(json);
    let parsed = serde_json::from_str::<Value>(json).ok().and_then(|v| {
        let pane_id = v
            .pointer("/result/plugin_pane/pane/pane_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let tab_id = v
            .pointer("/result/plugin_pane/pane/tab_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Some(PaneIds {
            pane_id: pane_id.to_string(),
            tab_id,
        })
    });
    if parsed.is_none() {
        eprintln!("herdr-review: warning: cannot parse review pane open reply: {raw}");
    }
    parsed
}

/// Closes a plugin pane by id.
pub(crate) fn pane_close(pane_id: &str) -> Result<(), String> {
    exec(&["plugin", "pane", "close", pane_id]).map(|_| ())
}

/// Closes a tab by id.
pub(crate) fn tab_close(tab_id: &str) -> Result<(), String> {
    exec(&["tab", "close", tab_id]).map(|_| ())
}

/// Focuses a tab by id. Used as a best-effort re-assert after opening the
/// review pane; herdr already applies `--focus` at open.
pub(crate) fn tab_focus(tab_id: &str) -> Result<(), String> {
    exec(&["tab", "focus", tab_id]).map(|_| ())
}

/// Submits `text` to the agent hosted in `target_pane`.
pub(crate) fn agent_prompt(target_pane: &str, text: &str) -> Result<(), String> {
    exec(&["agent", "prompt", target_pane, text]).map(|_| ())
}

/// Trims and truncates a snippet for error messages.
fn snippet(s: &str) -> &str {
    let s = s.trim();
    match s.char_indices().nth(200) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Direction;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Serializes tests that mutate `HERDR_BIN_PATH`, restoring the previous
    /// value on drop.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct BinEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<OsString>,
    }

    impl BinEnv {
        fn set(path: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var_os("HERDR_BIN_PATH");
            unsafe { std::env::set_var("HERDR_BIN_PATH", path) };
            Self { _lock: lock, prev }
        }
    }

    impl Drop for BinEnv {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var("HERDR_BIN_PATH", v) },
                None => unsafe { std::env::remove_var("HERDR_BIN_PATH") },
            }
        }
    }

    /// A scratch directory removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "herdr-review-herdr-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
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

    fn config(placement: Placement, direction: Direction) -> Config {
        Config {
            placement,
            direction,
            tool: None,
        }
    }

    #[test]
    fn setup_open_argv_carries_no_forwarding() {
        // Scenario "Setup-open argv carries no forwarding": exactly the
        // pinned argv — no `--workspace`, no `--cwd`, and no `--env` pairs,
        // because the dialog resolves the config path itself and needs
        // nothing else forwarded.
        assert_eq!(
            setup_pane_open_args(),
            [
                "plugin",
                "pane",
                "open",
                "--plugin",
                "alexeyco.review",
                "--entrypoint",
                "setup",
                "--placement",
                "popup",
                "--focus",
            ]
        );
        let args = setup_pane_open_args();
        let got: Vec<&str> = args.iter().map(String::as_str).collect();
        assert!(!got.contains(&"--workspace"), "{got:?}");
        assert!(!got.contains(&"--cwd"), "{got:?}");
        assert!(!got.contains(&"--env"), "{got:?}");
        // Scenario "Setup popup never carries the caller tab": no `--env`
        // pair exists at all, so the caller tab pair never rides that open.
        assert!(
            !got.iter().any(|a| a.contains("HERDR_REVIEWS_CALLER_TAB")),
            "{got:?}"
        );
    }

    #[test]
    fn review_pane_open_argv_full_flag_order() {
        // Scenarios "Resolved directory is passed twice" (both `--cwd` and
        // `HERDR_REVIEWS_DIR`), "The tool env pair is always present", and
        // "Split and zoomed pass the caller pane as target" (split side): the
        // target pair sits after the direction pair, and no `--workspace`
        // rides the split shape. "Caller tab pair is present on the direct
        // open": the tab pair sits after the caller pane and agent pairs and
        // immediately before `--focus`.
        let args = review_pane_open_args(
            &config(Placement::Split, Direction::Down),
            Some("ws-1"),
            "/repo",
            "tuicr",
            Some("pane-9"),
            Some("claude"),
            Some("tab-9"),
        );
        let got: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "plugin",
                "pane",
                "open",
                "--plugin",
                "alexeyco.review",
                "--entrypoint",
                "review",
                "--placement",
                "split",
                "--direction",
                "down",
                "--target-pane",
                "pane-9",
                "--cwd",
                "/repo",
                "--env",
                "HERDR_REVIEWS_DIR=/repo",
                "--env",
                "HERDR_REVIEW_TOOL=tuicr",
                "--env",
                "HERDR_REVIEWS_CALLER_PANE=pane-9",
                "--env",
                "HERDR_REVIEWS_CALLER_AGENT=claude",
                "--env",
                "HERDR_REVIEWS_CALLER_TAB=tab-9",
                "--focus",
            ]
        );
    }

    #[test]
    fn review_pane_open_argv_omits_absent_pairs() {
        // Scenario "Caller tab pair is omitted when no tab resolves": with no
        // tab, no caller pane, no agent, and no workspace the argv carries
        // none of the conditional pairs and the open shape is unchanged.
        let args = review_pane_open_args(
            &config(Placement::Tab, Direction::Right),
            None,
            "/repo",
            "revdiff",
            None,
            None,
            None,
        );
        let got: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "plugin",
                "pane",
                "open",
                "--plugin",
                "alexeyco.review",
                "--entrypoint",
                "review",
                "--placement",
                "tab",
                "--cwd",
                "/repo",
                "--env",
                "HERDR_REVIEWS_DIR=/repo",
                "--env",
                "HERDR_REVIEW_TOOL=revdiff",
                "--focus",
            ]
        );
        assert!(!got.contains(&"--direction"), "{got:?}");
    }

    #[test]
    fn each_accepted_placement_maps_to_argv() {
        // Scenarios "Each accepted placement maps to argv" and
        // "The pane-open argv always carries its own placement".
        for placement in [
            Placement::Tab,
            Placement::Split,
            Placement::Overlay,
            Placement::Zoomed,
        ] {
            let args = review_pane_open_args(
                &config(placement, Direction::Auto),
                None,
                "/repo",
                "lumen",
                None,
                None,
                None,
            );
            let got: Vec<&str> = args.iter().map(String::as_str).collect();
            assert_eq!(&got[7..9], ["--placement", placement.as_str()], "{got:?}");
        }
    }

    #[test]
    fn direction_is_passed_only_for_split_placement() {
        // Scenario "Direction is passed only for split placement": the flag
        // sits immediately after `--placement split`, and no non-split
        // placement ever carries it.
        for direction in [Direction::Right, Direction::Down] {
            let args = review_pane_open_args(
                &config(Placement::Split, direction),
                None,
                "/repo",
                "lumen",
                None,
                None,
                None,
            );
            let got: Vec<&str> = args.iter().map(String::as_str).collect();
            assert_eq!(
                &got[7..11],
                [
                    "--placement",
                    "split",
                    "--direction",
                    direction.argv_value().unwrap()
                ],
                "{got:?}"
            );
        }
        for placement in [Placement::Tab, Placement::Overlay, Placement::Zoomed] {
            let args = review_pane_open_args(
                &config(placement, Direction::Down),
                None,
                "/repo",
                "lumen",
                None,
                None,
                None,
            );
            let got: Vec<&str> = args.iter().map(String::as_str).collect();
            assert!(!got.contains(&"--direction"), "{got:?}");
        }
        // `auto` omits the flag even for split.
        let args = review_pane_open_args(
            &config(Placement::Split, Direction::Auto),
            None,
            "/repo",
            "lumen",
            None,
            None,
            None,
        );
        let got: Vec<&str> = args.iter().map(String::as_str).collect();
        assert!(!got.contains(&"--direction"), "{got:?}");
    }

    #[test]
    fn split_and_zoomed_pass_the_caller_pane_as_target() {
        // Scenario "Split and zoomed pass the caller pane as target": the
        // pair sits after the placement-and-direction pairs and before
        // `--cwd`; `tab` and `overlay` pass no `--target-pane`.
        let split = review_pane_open_args(
            &config(Placement::Split, Direction::Down),
            Some("ws-1"),
            "/repo",
            "lumen",
            Some("caller-7"),
            None,
            None,
        );
        let got: Vec<&str> = split.iter().map(String::as_str).collect();
        assert_eq!(
            &got[7..13],
            [
                "--placement",
                "split",
                "--direction",
                "down",
                "--target-pane",
                "caller-7"
            ],
            "{got:?}"
        );
        assert_eq!(got[13], "--cwd", "{got:?}");
        let zoomed = review_pane_open_args(
            &config(Placement::Zoomed, Direction::Down),
            Some("ws-1"),
            "/repo",
            "lumen",
            Some("caller-7"),
            None,
            None,
        );
        let got: Vec<&str> = zoomed.iter().map(String::as_str).collect();
        assert_eq!(
            &got[7..11],
            ["--placement", "zoomed", "--target-pane", "caller-7"],
            "{got:?}"
        );
        assert_eq!(got[11], "--cwd", "{got:?}");
        for placement in [Placement::Tab, Placement::Overlay] {
            let args = review_pane_open_args(
                &config(placement, Direction::Down),
                Some("ws-1"),
                "/repo",
                "lumen",
                Some("caller-7"),
                None,
                None,
            );
            let got: Vec<&str> = args.iter().map(String::as_str).collect();
            assert!(!got.contains(&"--target-pane"), "{got:?}");
        }
    }

    #[test]
    fn the_workspace_flag_is_passed_only_for_tab_placement() {
        // Scenario "The workspace flag is passed only for tab placement": a
        // forwarded workspace id reaches the argv as `--workspace` only on
        // the tab shape; `overlay`, `split`, and `zoomed` opens carry no
        // `--workspace`.
        for placement in [
            Placement::Tab,
            Placement::Split,
            Placement::Overlay,
            Placement::Zoomed,
        ] {
            let args = review_pane_open_args(
                &config(placement, Direction::Right),
                Some("ws-1"),
                "/repo",
                "revdiff",
                Some("caller-7"),
                None,
                None,
            );
            let got: Vec<&str> = args.iter().map(String::as_str).collect();
            assert_eq!(
                got.contains(&"--workspace"),
                placement == Placement::Tab,
                "{placement:?}: {got:?}"
            );
        }
        // On the tab shape the pair sits right after the placement pair.
        let args = review_pane_open_args(
            &config(Placement::Tab, Direction::Down),
            Some("ws-1"),
            "/repo",
            "revdiff",
            Some("caller-7"),
            None,
            None,
        );
        let got: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(
            &got[7..11],
            ["--placement", "tab", "--workspace", "ws-1"],
            "{got:?}"
        );
        assert_eq!(got[11], "--cwd", "{got:?}");
    }

    #[test]
    fn overlay_style_reply_without_a_tab() {
        // Scenario "Overlay-style reply without a tab": a valid pane id with
        // an absent or blank tab id is a success with tab_id treated as
        // absent.
        let json = r#"{"result":{"plugin_pane":{"pane":{"pane_id":"pane-1"}}}}"#;
        assert_eq!(
            parse_review_pane_open(json),
            Some(PaneIds {
                pane_id: "pane-1".to_string(),
                tab_id: None
            })
        );
        let json = r#"{"result":{"plugin_pane":{"pane":{"pane_id":"pane-1","tab_id":"  "}}}}"#;
        assert_eq!(
            parse_review_pane_open(json),
            Some(PaneIds {
                pane_id: "pane-1".to_string(),
                tab_id: None
            })
        );
    }

    #[test]
    fn reply_with_pane_and_tab_parses_both() {
        let json = r#"{"result":{"plugin_pane":{"pane":{"pane_id":"pane-1","tab_id":"tab-1"}}}}"#;
        assert_eq!(
            parse_review_pane_open(json),
            Some(PaneIds {
                pane_id: "pane-1".to_string(),
                tab_id: Some("tab-1".to_string())
            })
        );
    }

    #[test]
    fn reply_without_a_usable_pane_id_warns_but_succeeds() {
        // Scenario "Reply without a usable pane id warns but succeeds": not
        // valid JSON, or valid JSON without a non-blank pane id, yields
        // `None` (never an error).
        for json in [
            "{not json",
            "",
            r#"{"result":{"type":"ok"}}"#,
            r#"{"result":{"plugin_pane":{"pane":{"tab_id":"tab-1"}}}}"#,
            r#"{"result":{"plugin_pane":{"pane":{"pane_id":"   "}}}}"#,
        ] {
            assert!(parse_review_pane_open(json).is_none(), "{json:?}");
        }
    }

    #[test]
    fn config_dir_parses_a_bare_path_line() {
        assert_eq!(
            parse_config_dir("/home/u/.config/herdr/plugins/config/alexeyco.review\n").unwrap(),
            "/home/u/.config/herdr/plugins/config/alexeyco.review"
        );
        let err = parse_config_dir("  \n\n").unwrap_err();
        assert!(err.contains("blank reply"), "{err}");
        let err = parse_config_dir("/one\ntwo\n").unwrap_err();
        assert!(err.contains("more than one line"), "{err}");
    }

    #[test]
    fn popup_open_succeeds_without_a_pane_id_in_the_reply() {
        // Scenarios "Popup open succeeds without a pane id in the reply" and
        // "Popup reply body is ignored": a stub herdr exits 0 with a body
        // that carries no pane id (or is not even JSON), and the open
        // succeeds because only the exit status is checked.
        let dir = TempDir::new("popup-ok");
        let body = r#"{"id":"cli:plugin","result":{"type":"ok"}}"#;
        {
            let bin = dir.stub(
                "herdr",
                &format!("#!/bin/sh\nprintf '%s\\n' '{body}'\nexit 0\n"),
            );
            let _env = BinEnv::set(bin.to_str().unwrap());
            assert!(open_setup_popup().is_ok());
        }
        // A body that is not JSON at all is equally ignored.
        {
            let bin = dir.stub("herdr2", "#!/bin/sh\nprintf 'not json at all'\nexit 0\n");
            let _env = BinEnv::set(bin.to_str().unwrap());
            assert!(open_setup_popup().is_ok());
        }
    }

    #[test]
    fn review_pane_open_warns_but_succeeds_on_an_unparsable_reply() {
        // The full open path: exec succeeds (exit 0), the reply is garbage,
        // so the result is Ok(None) with a warning.
        let dir = TempDir::new("review-garbage");
        let bin = dir.stub("herdr", "#!/bin/sh\nprintf '{not json'\nexit 0\n");
        let _env = BinEnv::set(bin.to_str().unwrap());
        let result = open_review_pane(
            &config(Placement::Tab, Direction::Right),
            None,
            "/repo",
            "revdiff",
            None,
            None,
            None,
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn review_pane_open_parses_the_stub_reply() {
        let dir = TempDir::new("review-parse");
        let body =
            r#"{"result":{"plugin_pane":{"pane":{"pane_id":"pane-stub","tab_id":"tab-stub"}}}}"#;
        let bin = dir.stub(
            "herdr",
            &format!("#!/bin/sh\nprintf '%s\\n' '{body}'\nexit 0\n"),
        );
        let _env = BinEnv::set(bin.to_str().unwrap());
        let result = open_review_pane(
            &config(Placement::Tab, Direction::Right),
            Some("ws-1"),
            "/repo",
            "revdiff",
            Some("pane-9"),
            None,
            Some("tab-9"),
        );
        assert_eq!(
            result.unwrap(),
            Some(PaneIds {
                pane_id: "pane-stub".to_string(),
                tab_id: Some("tab-stub".to_string())
            })
        );
    }

    #[test]
    fn config_dir_resolves_through_the_cli() {
        // Scenario "CLI fallback resolves the directory", argv side: the
        // callback is exactly `plugin config-dir alexeyco.review` and the
        // bare path line is the directory. The log assertion matches by
        // line, not by whole content: `HERDR_BIN_PATH` is process-wide, so
        // unrelated close-path calls from parallel tests may share the stub.
        let dir = TempDir::new("config-dir");
        let log = dir.0.join("calls.log");
        let bin = dir.stub(
            "herdr",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '%s\\n' /cfg/review\nexit 0\n",
                log.display()
            ),
        );
        let _env = BinEnv::set(bin.to_str().unwrap());
        assert_eq!(config_dir().unwrap(), "/cfg/review");
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(
            logged
                .lines()
                .any(|l| l == "plugin config-dir alexeyco.review"),
            "{logged}"
        );
    }

    #[test]
    fn herdr_bin_path_is_missing_or_blank() {
        // Scenario "HERDR_BIN_PATH is missing or blank".
        for raw in [None, Some(""), Some("   "), Some("\t\n")] {
            let err = herdr_bin_from(raw).unwrap_err();
            assert!(
                err.contains("herdr did not provide HERDR_BIN_PATH (plugin API v1 contract)"),
                "{err}"
            );
        }
    }

    #[test]
    fn herdr_bin_path_is_trimmed() {
        assert_eq!(
            herdr_bin_from(Some("  /usr/local/bin/herdr  ")).as_deref(),
            Ok("/usr/local/bin/herdr")
        );
    }

    #[test]
    fn a_cli_call_exits_nonzero() {
        // Scenario "A CLI call exits nonzero": the error names the binary,
        // the argv, the status, and a stderr snippet.
        let dir = TempDir::new("nonzero");
        let bin = dir.stub("herdr", "#!/bin/sh\necho 'boom' >&2\nexit 3\n");
        let err = exec_with_bin(bin.to_str().unwrap(), &["plugin", "pane", "open"]).unwrap_err();
        assert!(
            err.contains(&format!(
                "{} plugin pane open failed (exit status: 3)",
                bin.display()
            )),
            "{err}"
        );
        assert!(err.contains("boom"), "{err}");
    }

    #[test]
    fn the_herdr_binary_cannot_be_spawned() {
        // Scenario "The herdr binary cannot be spawned".
        let err = exec_with_bin("/nonexistent-herdr-binary", &["tab", "focus", "t1"]).unwrap_err();
        assert!(
            err.starts_with("failed to run /nonexistent-herdr-binary tab focus t1:"),
            "{err}"
        );
        assert!(err.contains("No such file"), "{err}");
    }

    #[test]
    fn snippet_truncates_long_output() {
        let long = "x".repeat(500);
        assert_eq!(snippet(&long).len(), 200);
    }
}
