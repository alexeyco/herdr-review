use super::*;
use crate::settings::{Config, Direction, Placement};
use ports::{
    MockReviewActionHost, MockReviewDeliveryHost, MockReviewRepository, MockReviewToolRunner,
};
use std::cell::RefCell;

/// A canonicalized scratch directory, removed when the test ends.
struct TempDir(tempfile::TempDir);

impl TempDir {
    fn new(tag: &str) -> Self {
        Self(
            tempfile::Builder::new()
                .prefix(&format!("herdr-review-review-test-{tag}-"))
                .tempdir()
                .unwrap(),
        )
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    /// Writes an executable stub and returns its path.
    fn stub(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.path().join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
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
    assert_eq!(
        decide(ReviewTool::Revdiff, 10, true, true),
        Outcome::Deliver
    );
}

#[test]
fn annotations_without_an_agent() {
    // Scenario "Annotations without an agent".
    for tool in ReviewTool::ALL {
        assert_eq!(decide(tool, 0, true, false), Outcome::NoAgent);
        assert_eq!(decide(tool, 0, true, true), Outcome::Deliver);
    }
    assert_eq!(
        decide(ReviewTool::Revdiff, 10, true, false),
        Outcome::NoAgent
    );
}

#[test]
fn clean_exit_with_blank_output_is_no_annotations() {
    // Scenario "Clean exit with blank output is no-annotations" for all
    // three tools.
    for tool in ReviewTool::ALL {
        assert_eq!(decide(tool, 0, false, true), Outcome::NoAnnotations);
        assert_eq!(decide(tool, 0, false, false), Outcome::NoAnnotations);
    }
}

#[test]
fn any_other_exit_code_fails_regardless_of_output() {
    // Scenario "Any other exit code fails regardless of output": revdiff
    // outside {0, 10}, tuicr/lumen nonzero — even with annotations.
    assert_eq!(
        decide(ReviewTool::Revdiff, 1, true, true),
        Outcome::Failed(1)
    );
    assert_eq!(
        decide(ReviewTool::Revdiff, 2, false, false),
        Outcome::Failed(2)
    );
    assert_eq!(decide(ReviewTool::Tuicr, 1, true, true), Outcome::Failed(1));
    assert_eq!(decide(ReviewTool::Lumen, 2, true, true), Outcome::Failed(2));
}

#[test]
fn tuicr_with_comments_delivers() {
    // Scenario "tuicr with comments delivers".
    assert_eq!(decide(ReviewTool::Tuicr, 0, true, true), Outcome::Deliver);
}

#[test]
fn lumen_export_delivers() {
    // Scenario "lumen export delivers".
    assert_eq!(decide(ReviewTool::Lumen, 0, true, true), Outcome::Deliver);
}

// --- Prompt construction ----------------------------------------------

#[test]
fn header_names_the_tool_that_ran() {
    // Scenario "Header names the tool that ran".
    assert_eq!(
        prompt_text(ReviewTool::Tuicr, "## a.rs:1 (+)\nnote\n"),
        "Review annotations from tuicr:\n\n## a.rs:1 (+)\nnote"
    );
    assert_eq!(
        prompt_text(ReviewTool::Lumen, "  body  "),
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
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: false,
                unstaged: true,
                untracked: false
            }
        ),
        [
            "/bin/revdiff",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations"
        ]
    );
    assert_eq!(
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: true,
                unstaged: false,
                untracked: false
            }
        ),
        [
            "/bin/revdiff",
            "--staged",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations"
        ]
    );
    assert_eq!(
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: true,
                unstaged: true,
                untracked: false
            }
        ),
        [
            "/bin/revdiff",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations",
            "HEAD"
        ]
    );
    assert_eq!(
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: true,
                unstaged: false,
                untracked: true
            }
        ),
        [
            "/bin/revdiff",
            "--untracked",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations",
            "HEAD"
        ]
    );
    assert_eq!(
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: true,
                unstaged: true,
                untracked: true
            }
        ),
        [
            "/bin/revdiff",
            "--untracked",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations",
            "HEAD"
        ]
    );
    assert_eq!(
        revdiff_argv("/bin/revdiff", "/s/1.out", &Scope::AllFiles),
        [
            "/bin/revdiff",
            "--all-files",
            "--output",
            "/s/1.out",
            "--exit-code-on-annotations"
        ]
    );
    assert_eq!(
        revdiff_argv(
            "/bin/revdiff",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: false,
                unstaged: false,
                untracked: true
            }
        ),
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
        child_argv(
            ReviewTool::Tuicr,
            "/bin/tuicr",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: false,
                unstaged: true,
                untracked: false
            }
        ),
        ["/bin/tuicr", "-w"]
    );
    assert_eq!(
        child_argv(
            ReviewTool::Tuicr,
            "/bin/tuicr",
            "/s/1.out",
            &Scope::VsBase("origin/main".to_string())
        ),
        ["/bin/tuicr", "-r", "origin/main..HEAD"]
    );
    assert_eq!(
        child_argv(
            ReviewTool::Lumen,
            "/bin/lumen",
            "/s/1.out",
            &Scope::Uncommitted {
                staged: false,
                unstaged: true,
                untracked: false
            }
        ),
        ["/bin/lumen", "diff"]
    );
    assert_eq!(
        child_argv(
            ReviewTool::Lumen,
            "/bin/lumen",
            "/s/1.out",
            &Scope::VsBase("origin/main".to_string())
        ),
        ["/bin/lumen", "diff", "origin/main...HEAD"]
    );
}

#[test]
fn only_lumen_stdout_is_captured() {
    assert!(!captures_stdout(ReviewTool::Tuicr));
    assert!(!captures_stdout(ReviewTool::Revdiff));
    assert!(captures_stdout(ReviewTool::Lumen));
}

#[test]
fn spawn_failure() {
    // Scenario "Spawn failure": the binary vanished between resolution
    // and spawn.
    let dir = TempDir::new("spawn-fail");
    let argv = vec!["/nonexistent-tool-bin".to_string(), "-w".to_string()];
    let err = spawn_child(ReviewTool::Tuicr, &argv, dir.path()).unwrap_err();
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
    spawn_child(ReviewTool::Revdiff, &argv, dir.path()).unwrap();
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
        ReviewTool::Tuicr,
        &bin.to_string_lossy(),
        "/s/1.out",
        &Scope::Uncommitted {
            staged: false,
            unstaged: true,
            untracked: false,
        },
    );
    let child = spawn_child(ReviewTool::Tuicr, &argv, dir.path()).unwrap();
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
        ReviewTool::Lumen,
        &bin.to_string_lossy(),
        "/s/1.out",
        &Scope::Uncommitted {
            staged: false,
            unstaged: true,
            untracked: false,
        },
    );
    let child = spawn_child(ReviewTool::Lumen, &argv, dir.path()).unwrap();
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
    let child = spawn_child(ReviewTool::Revdiff, &argv, dir.path()).unwrap();
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
    let (canonical, annotations) = collect_lumen_output(capture, &out).unwrap();
    assert!(annotations);
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
    assert_eq!(
        collect_lumen_output("", &out).unwrap(),
        (String::new(), false)
    );
    assert_eq!(
        collect_lumen_output("  \n", &out).unwrap(),
        (String::new(), false)
    );
    // The exact live byte sequence lumen emits at TUI startup (spike 1.2):
    // OSC `ESC]11;?BEL` + CSI `ESC[5n`.
    let probes = "\u{1b}]11;?\u{7}\u{1b}[5n";
    assert_eq!(probes.as_bytes(), b"\x1b]11;?\x07\x1b[5n");
    assert_eq!(
        collect_lumen_output(probes, &out).unwrap(),
        (String::new(), false)
    );
    assert_eq!(
        collect_lumen_output(&format!("{probes}  \n"), &out).unwrap(),
        (String::new(), false)
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
        ReviewTool::Lumen,
        &bin.to_string_lossy(),
        "/s/1.out",
        &Scope::Uncommitted {
            staged: false,
            unstaged: true,
            untracked: false,
        },
    );
    let child = spawn_child(ReviewTool::Lumen, &argv, dir.path()).unwrap();
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
        ReviewTool::Tuicr,
        &bin.to_string_lossy(),
        "/s/1.out",
        &Scope::Uncommitted {
            staged: false,
            unstaged: true,
            untracked: false,
        },
    );
    let child = spawn_child(ReviewTool::Tuicr, &argv, dir.path()).unwrap();
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
fn deliver_run<'a>(dir: &'a TempDir, out: &'a Path, caller_tab: Option<&'a str>) -> PaneRun<'a> {
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
fn pane_uses_repository_and_runner_ports() {
    let dir = TempDir::new("pane-mocks");
    let reports = dir.path().join("reviews");
    let mut repository = MockReviewRepository::new();
    repository
        .expect_scope()
        .times(1)
        .return_const(Scope::Uncommitted {
            staged: false,
            unstaged: true,
            untracked: false,
        });
    let mut runner = MockReviewToolRunner::new();
    runner
        .expect_run()
        .withf(|tool, argv, workdir| {
            *tool == ReviewTool::Tuicr
                && argv.first().is_some_and(|bin| bin == "tuicr-bin")
                && argv.contains(&"-w".to_string())
                && workdir == Path::new("/repo")
        })
        .times(1)
        .returning(|_, _, _| {
            Ok(ChildResult {
                code: Some(1),
                stdout: String::new(),
            })
        });
    let pane = ReviewPane {
        repository: &repository,
        runner: &runner,
    };
    let error = pane
        .run_in(Path::new("/repo"), ReviewTool::Tuicr, &reports, || {
            Ok("tuicr-bin".to_string())
        })
        .unwrap_err();
    assert!(error.contains("tuicr failed with exit code 1"), "{error}");
    assert_eq!(std::fs::read_dir(reports).unwrap().count(), 1);
}

#[test]
fn delivered_run_refocuses_the_caller_tab() {
    // Scenario "Delivered run refocuses the caller tab": before the pane
    // close attempt the process runs `tab focus <caller tab>` best-effort
    // and exits 0.
    let dir = TempDir::new("focus-deliver");
    let out = dir.path().join("1.out");
    let mut sequence = mockall::Sequence::new();
    let mut teardown = MockReviewDeliveryHost::new();
    teardown
        .expect_deliver()
        .withf(|pane, prompt| {
            pane == "caller-1" && prompt.starts_with("Review annotations from revdiff:")
        })
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _| Ok(()));
    teardown
        .expect_focus()
        .withf(|tab| tab == "w3X:t9")
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(()));
    teardown
        .expect_close()
        .times(1)
        .in_sequence(&mut sequence)
        .return_const(());
    teardown.expect_warn().times(0);
    teardown.expect_hold().times(0);
    let run = deliver_run(&dir, &out, Some("w3X:t9"));
    collect_and_finish_with(&run, &rc_child(10), &teardown).unwrap();
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
fn exactly_one_installed_tool_opens_the_review_pane_directly() {
    // Scenario "Exactly one catalog tool is installed": the installed
    // tool opens directly — no popup is shown.
    let dir = TempDir::new("action-direct");
    let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
    let mut deps = MockReviewActionHost::new();
    deps.expect_load_config().return_const(Config {
        ..Config::default()
    });
    deps.expect_open_review()
        .withf(|request| {
            request.config.placement == Placement::Tab
                && request.dir == "/repo"
                && request.tool_id == "revdiff"
                && request.workspace == Some("ws-1")
                && request.caller_pane == Some("caller-1")
                && request.caller_agent == Some("claude")
                && request.caller_tab == Some("caller-tab-1")
        })
        .times(1)
        .returning(|_| Ok(None));
    deps.expect_open_setup().times(0);
    deps.expect_focus().times(0);
    deps.expect_log().times(0);
    deps.expect_warn().times(0);
    run_action_with(
        &full_identity(),
        "/repo",
        &dir.path().to_string_lossy(),
        &deps,
    )
    .unwrap();
}

#[test]
fn no_installed_tools_open_the_setup_dialog() {
    // Scenario "No catalog tools are installed": the setup popup opens
    // and no review pane does.
    let dir = TempDir::new("action-unset");
    let setup_opens: RefCell<usize> = RefCell::new(0);
    let logged: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let deps = ActionDeps {
        load_config: &|| Config::default(),
        open: &|_, _, _, _, _, _, _| panic!("no installed tool must not open the review pane"),
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
fn an_installed_tool_opens_directly_regardless_of_obsolete_config() {
    // Exactly one installed catalog tool opens directly, regardless of config.
    let dir = TempDir::new("action-missing");
    let _bin = dir.stub("revdiff", "#!/bin/sh\nexit 0\n");
    let reviews: RefCell<usize> = RefCell::new(0);
    let deps = ActionDeps {
        load_config: &|| Config {
            ..Config::default()
        },
        open: &|_, _, _, _, _, _, _| {
            *reviews.borrow_mut() += 1;
            Ok(None)
        },
        open_setup: &|| panic!("an installed tool must not open setup"),
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
    assert_eq!(*reviews.borrow(), 1);
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
