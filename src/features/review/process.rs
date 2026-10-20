use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::adapters::git::Scope;
use crate::adapters::tool::ReviewTool;
use anyhow::Context;

use super::ports::ReviewToolRunner;

pub(super) fn child_argv(
    tool: ReviewTool,
    bin: &str,
    out_path: &str,
    scope: &Scope,
) -> Vec<String> {
    match tool {
        ReviewTool::Revdiff => revdiff_argv(bin, out_path, scope),
        ReviewTool::Tuicr => {
            let mut argv = vec![bin.to_string()];
            argv.extend(scope.fragments(tool));
            argv
        }
        ReviewTool::Lumen => {
            let mut argv = vec![bin.to_string(), "diff".to_string()];
            argv.extend(scope.fragments(tool));
            argv
        }
    }
}

pub(super) fn revdiff_argv(bin: &str, out_path: &str, scope: &Scope) -> Vec<String> {
    let fragments = scope.fragments(ReviewTool::Revdiff);
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

pub(super) fn captures_stdout(tool: ReviewTool) -> bool {
    tool == ReviewTool::Lumen
}

#[derive(Debug, Default)]
pub(crate) struct ChildResult {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
}

pub(super) struct ChildProcess;

impl ReviewToolRunner for ChildProcess {
    fn run(&self, tool: ReviewTool, argv: &[String], dir: &Path) -> Result<ChildResult, String> {
        spawn_child(tool, argv, dir)
    }
}

pub(super) fn spawn_child(
    tool: ReviewTool,
    argv: &[String],
    dir: &Path,
) -> Result<ChildResult, String> {
    spawn_child_inner(tool, argv, dir).map_err(|error| format!("{error:#}"))
}

fn spawn_child_inner(tool: ReviewTool, argv: &[String], dir: &Path) -> anyhow::Result<ChildResult> {
    #[cfg(unix)]
    const ETXTBSY: i32 = 26;
    #[cfg(not(unix))]
    const ETXTBSY: i32 = -1;
    const ETXTBSY_RETRIES: u32 = 5;

    let piped = captures_stdout(tool);
    let mut child = {
        let mut attempt = 0;
        loop {
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]).current_dir(dir);
            if piped {
                cmd.stdout(Stdio::piped());
            }
            match cmd.spawn() {
                Ok(child) => break child,
                // Linux can briefly reject exec while the binary is being written.
                Err(e) if e.raw_os_error() == Some(ETXTBSY) && attempt < ETXTBSY_RETRIES => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to run {} in {}", argv[0], dir.display())
                    });
                }
            }
        }
    };
    if piped {
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed to wait for {}", argv[0]))?;
        Ok(ChildResult {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    } else {
        let status = child
            .wait()
            .with_context(|| format!("failed to wait for {}", argv[0]))?;
        Ok(ChildResult {
            code: status.code(),
            stdout: String::new(),
        })
    }
}
