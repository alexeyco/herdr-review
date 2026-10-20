//! Read-only git probes and review-scope selection.
//!
//! Detection runs in the pane process, in the pane's working directory (the
//! repository), before the review tool starts. Every probe is read-only: the
//! index and the working tree are never mutated.

use std::path::Path;
use std::process::{Command, Output};

use crate::adapters::tool::ReviewTool;

/// Which changes a review covers for the resolved directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Staged, unstaged, and/or untracked changes exist.
    Uncommitted,
    /// A clean tree with an attached HEAD and a base ref to review against.
    VsBase(String),
    /// Nothing reviewable: not a repo, no commits, a detached HEAD, no base,
    /// or a failed probe.
    NothingToReview,
}

impl Scope {
    /// The argv fragments this scope contributes for `tool_id`, in the order
    /// they are passed to the child. This is the per-tool fragment table:
    ///
    /// | Scope            | revdiff               | tuicr                | lumen                 |
    /// | ---------------- | --------------------- | -------------------- | --------------------- |
    /// | `Uncommitted`    | `--untracked`         | `-w`                 | (none)                |
    /// | `VsBase(<base>)` | `<base> HEAD`         | `-r <base>..HEAD`    | `<base>...HEAD`       |
    ///
    /// For revdiff the `Uncommitted` fragment is a flag (the spawn builder
    /// places it before the fixed trailing flags) while `VsBase`'s are the
    /// two positional refs (placed after them); the other tools consume one
    /// plain list.
    pub(crate) fn fragments(&self, tool: ReviewTool) -> Vec<String> {
        match (self, tool) {
            (Scope::Uncommitted, ReviewTool::Revdiff) => vec!["--untracked".to_string()],
            (Scope::Uncommitted, ReviewTool::Tuicr) => vec!["-w".to_string()],
            (Scope::Uncommitted, ReviewTool::Lumen) => Vec::new(),
            (Scope::VsBase(base), ReviewTool::Revdiff) => vec![base.clone(), "HEAD".to_string()],
            (Scope::VsBase(base), ReviewTool::Tuicr) => {
                vec!["-r".to_string(), format!("{base}..HEAD")]
            }
            (Scope::VsBase(base), ReviewTool::Lumen) => vec![format!("{base}...HEAD")],
            (Scope::NothingToReview, _) => Vec::new(),
        }
    }
}

/// Read-only git state used to pick a review scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitStatus {
    pub(crate) repo: bool,
    pub(crate) has_head: bool,
    pub(crate) unstaged: bool,
    pub(crate) staged: bool,
    pub(crate) untracked: bool,
    /// `git symbolic-ref -q HEAD` failed: HEAD is detached (probed only on a
    /// clean tree).
    pub(crate) detached: bool,
    /// First base from the `origin/HEAD` → `origin/main` → `origin/master`
    /// chain that verified (probed only on a clean tree).
    pub(crate) base: Option<String>,
}

/// Read-only access to one repository.
pub(crate) struct GitRepository<'a> {
    dir: &'a Path,
}

impl<'a> GitRepository<'a> {
    pub(crate) fn new(dir: &'a Path) -> Self {
        Self { dir }
    }

    pub(crate) fn status(&self) -> GitStatus {
        match try_probe(self.dir) {
            Ok(status) => status,
            Err(e) => {
                eprintln!(
                    "herdr-review: git probe failed ({}): {}; nothing to review",
                    e.cmd, e.err
                );
                GitStatus::default()
            }
        }
    }

    pub(crate) fn review_scope(&self) -> Scope {
        select_scope(&self.status())
    }
}

impl crate::features::review::ports::ReviewRepository for GitRepository<'_> {
    fn scope(&self) -> Scope {
        self.review_scope()
    }
}

/// Picks the review scope from a git state. Pure: every branch is unit-tested.
pub(crate) fn select_scope(s: &GitStatus) -> Scope {
    if !s.repo || !s.has_head {
        return Scope::NothingToReview;
    }
    if s.unstaged || s.staged || s.untracked {
        return Scope::Uncommitted;
    }
    match (s.detached, &s.base) {
        (false, Some(base)) => Scope::VsBase(base.clone()),
        _ => Scope::NothingToReview,
    }
}

/// Probes `dir` read-only and returns its git state. On any unexpected git
/// failure (spawn error, or an exit code outside the documented meanings) one
/// warning is written to stderr and a non-repo status is returned, which
/// selects the nothing-to-review hold path.
#[cfg(test)]
fn probe(dir: &Path) -> GitStatus {
    GitRepository::new(dir).status()
}

/// A failed probe: the command that ran and what went wrong.
#[derive(Debug)]
struct ProbeError {
    cmd: String,
    err: String,
}

impl ProbeError {
    fn new(cmd: String, err: impl Into<String>) -> Self {
        Self {
            cmd,
            err: err.into(),
        }
    }
}

fn try_probe(dir: &Path) -> Result<GitStatus, ProbeError> {
    Prober {
        run: &|args| run_git(dir, args),
    }
    .status()
}

/// Runs one git command (argv after `git`) in `dir`; the error is the spawn
/// failure message.
fn run_git(dir: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())
}

/// The probe sequence over an injected command runner. The runner receives
/// the argv after `git` and returns the captured output or a spawn error;
/// production runs real `git` in the probe directory, tests script replies so
/// the command order itself can be asserted.
struct Prober<'a> {
    run: &'a dyn Fn(&[&str]) -> Result<Output, String>,
}

impl Prober<'_> {
    fn exec(&self, args: &[&str]) -> Result<Output, ProbeError> {
        (self.run)(args).map_err(|e| ProbeError::new(command(args), e))
    }

    /// The probe sequence: a non-repo stops at the first command, a repo
    /// without HEAD at the second, a dirty tree after the three change
    /// probes, and only a clean tree reaches the detached check and the base
    /// chain.
    fn status(&self) -> Result<GitStatus, ProbeError> {
        if !self.is_repo()? {
            return Ok(GitStatus::default());
        }
        if !self.has_rev("HEAD")? {
            return Ok(GitStatus {
                repo: true,
                ..GitStatus::default()
            });
        }
        let unstaged = self.diff_present(&[])?;
        let staged = self.diff_present(&["--cached"])?;
        let untracked = self.has_untracked()?;
        if unstaged || staged || untracked {
            return Ok(GitStatus {
                repo: true,
                has_head: true,
                unstaged,
                staged,
                untracked,
                ..GitStatus::default()
            });
        }
        Ok(GitStatus {
            repo: true,
            has_head: true,
            detached: self.is_detached()?,
            base: self.resolve_base()?,
            ..GitStatus::default()
        })
    }

    /// `rev-parse --is-inside-work-tree` exits 0 and prints `true` inside a
    /// work tree; any other outcome is the expected "not a repository"
    /// signal, not an error.
    fn is_repo(&self) -> Result<bool, ProbeError> {
        let args = ["rev-parse", "--is-inside-work-tree"];
        let out = self.exec(&args)?;
        Ok(out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
    }

    /// `rev-parse -q --verify <rev>`: exit 0 means the revision exists.
    fn has_rev(&self, rev: &str) -> Result<bool, ProbeError> {
        let args = ["rev-parse", "-q", "--verify", rev];
        let out = self.exec(&args)?;
        Ok(out.status.success())
    }

    /// A quiet diff exits 0 with no changes and 1 with changes; anything else
    /// is unexpected.
    fn diff_present(&self, extra: &[&str]) -> Result<bool, ProbeError> {
        let mut args = vec!["diff"];
        args.extend_from_slice(extra);
        args.push("--quiet");
        let out = self.exec(&args)?;
        match out.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(unexpected(&args, &out)),
        }
    }

    /// `ls-files --others --exclude-standard` lists untracked files on
    /// stdout.
    fn has_untracked(&self) -> Result<bool, ProbeError> {
        let args = ["ls-files", "--others", "--exclude-standard"];
        let out = self.exec(&args)?;
        if !out.status.success() {
            return Err(unexpected(&args, &out));
        }
        Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
    }

    /// `symbolic-ref -q HEAD` exits nonzero on a detached HEAD.
    fn is_detached(&self) -> Result<bool, ProbeError> {
        let args = ["symbolic-ref", "-q", "HEAD"];
        let out = self.exec(&args)?;
        match out.status.code() {
            Some(0) => Ok(false),
            Some(_) => Ok(true),
            None => Err(unexpected(&args, &out)),
        }
    }

    /// First base from the chain `origin/HEAD` → `origin/main` →
    /// `origin/master` that verifies; `None` when none do.
    fn resolve_base(&self) -> Result<Option<String>, ProbeError> {
        for base in ["origin/HEAD", "origin/main", "origin/master"] {
            if self.has_rev(base)? {
                return Ok(Some(base.to_string()));
            }
        }
        Ok(None)
    }
}

fn unexpected(args: &[&str], out: &Output) -> ProbeError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    ProbeError::new(command(args), format!("{}: {}", out.status, stderr.trim()))
}

fn command(args: &[&str]) -> String {
    format!("git {}", args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonicalized scratch directory, removed when the test ends.
    struct TempDir(tempfile::TempDir);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = tempfile::Builder::new()
                .prefix(&format!("herdr-review-git-test-{tag}-"))
                .tempdir()
                .unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            self.0.path()
        }
    }

    /// Runs git in `dir` with an inline identity so no global git config is
    /// required. Panics on a non-zero exit with git's stderr.
    fn git(dir: &Path, args: &[&str]) {
        let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@example.com"];
        full.extend_from_slice(args);
        let out = Command::new("git")
            .args(&full)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_and_commit(dir: &Path, file: &str, contents: &str) {
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join(file), contents).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", "base"]);
    }

    /// Adds an `origin` remote and an `origin/main` ref pointing at HEAD.
    fn add_origin_main(dir: &Path) {
        git(
            dir,
            &[
                "remote",
                "add",
                "origin",
                "https://example.com/herdr-review.git",
            ],
        );
        git(dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    }

    /// A second commit so HEAD is ahead of `origin/main` with a clean tree.
    fn second_commit(dir: &Path) {
        std::fs::write(dir.join("a.txt"), "b\n").unwrap();
        git(dir, &["add", "a.txt"]);
        git(dir, &["commit", "-q", "-m", "second"]);
    }

    /// A repository with one commit and a clean tree.
    fn repo_with_head() -> GitStatus {
        GitStatus {
            repo: true,
            has_head: true,
            ..GitStatus::default()
        }
    }

    #[test]
    fn dirty_tree_selects_uncommitted() {
        // Unstaged, staged, untracked, and every mix all review as
        // uncommitted changes, including a staged-only tree.
        for s in [
            GitStatus {
                unstaged: true,
                ..repo_with_head()
            },
            GitStatus {
                staged: true,
                ..repo_with_head()
            },
            GitStatus {
                untracked: true,
                ..repo_with_head()
            },
            GitStatus {
                unstaged: true,
                staged: true,
                ..repo_with_head()
            },
            GitStatus {
                staged: true,
                untracked: true,
                ..repo_with_head()
            },
            GitStatus {
                unstaged: true,
                staged: true,
                untracked: true,
                ..repo_with_head()
            },
        ] {
            assert_eq!(select_scope(&s), Scope::Uncommitted);
        }
    }

    #[test]
    fn clean_tree_with_base_selects_vs_base() {
        let s = GitStatus {
            base: Some("origin/main".to_string()),
            ..repo_with_head()
        };
        assert_eq!(select_scope(&s), Scope::VsBase("origin/main".to_string()));
    }

    #[test]
    fn clean_tree_without_base_selects_nothing_to_review() {
        assert_eq!(select_scope(&repo_with_head()), Scope::NothingToReview);
    }

    #[test]
    fn detached_head_on_clean_tree_selects_nothing_to_review() {
        // Detached wins even when a base branch exists.
        let s = GitStatus {
            detached: true,
            base: Some("origin/main".to_string()),
            ..repo_with_head()
        };
        assert_eq!(select_scope(&s), Scope::NothingToReview);
    }

    #[test]
    fn non_repo_selects_nothing_to_review() {
        assert_eq!(select_scope(&GitStatus::default()), Scope::NothingToReview);
    }

    #[test]
    fn repo_without_commits_selects_nothing_to_review() {
        // No HEAD means nothing to review, even with untracked files present.
        let s = GitStatus {
            repo: true,
            untracked: true,
            ..GitStatus::default()
        };
        assert_eq!(select_scope(&s), Scope::NothingToReview);
    }

    #[test]
    fn uncommitted_fragments_per_tool() {
        // Scenario "Uncommitted fragments per tool".
        assert_eq!(
            Scope::Uncommitted.fragments(ReviewTool::Revdiff),
            ["--untracked"]
        );
        assert_eq!(Scope::Uncommitted.fragments(ReviewTool::Tuicr), ["-w"]);
        assert!(Scope::Uncommitted.fragments(ReviewTool::Lumen).is_empty());
        assert!(
            Scope::NothingToReview
                .fragments(ReviewTool::Revdiff)
                .is_empty()
        );
    }

    #[test]
    fn vs_base_fragments_per_tool() {
        // Scenario "Vs-base fragments per tool".
        let scope = Scope::VsBase("origin/main".to_string());
        assert_eq!(
            scope.fragments(ReviewTool::Revdiff),
            ["origin/main", "HEAD"]
        );
        assert_eq!(
            scope.fragments(ReviewTool::Tuicr),
            ["-r", "origin/main..HEAD"]
        );
        assert_eq!(scope.fragments(ReviewTool::Lumen), ["origin/main...HEAD"]);
    }

    #[test]
    fn revdiff_uses_two_positional_refs_not_a_range_argument() {
        // Scenario "revdiff uses two positional refs, not a range argument":
        // the fragment is `<base> HEAD` as two positionals, never a single
        // `base..HEAD` argument.
        let fragments = Scope::VsBase("origin/main".to_string()).fragments(ReviewTool::Revdiff);
        assert_eq!(fragments, ["origin/main", "HEAD"]);
        assert!(!fragments.iter().any(|f| f.contains("..")), "{fragments:?}");
    }

    #[test]
    fn probe_non_repo_dir_selects_nothing_to_review() {
        let dir = TempDir::new("nonrepo");
        let status = probe(dir.path());
        assert_eq!(status, GitStatus::default());
        assert!(!status.repo);
        assert_eq!(select_scope(&status), Scope::NothingToReview);
    }

    #[test]
    fn probe_repo_without_commits_selects_nothing_to_review() {
        let dir = TempDir::new("nocommits");
        git(dir.path(), &["init", "-q"]);
        // Untracked files do not help: HEAD does not exist.
        std::fs::write(dir.path().join("new.txt"), "n\n").unwrap();
        let status = probe(dir.path());
        assert_eq!(
            status,
            GitStatus {
                repo: true,
                ..GitStatus::default()
            }
        );
        assert_eq!(select_scope(&status), Scope::NothingToReview);
    }

    #[test]
    fn probe_clean_tree_with_origin_main_selects_vs_base() {
        let dir = TempDir::new("origin-main");
        init_and_commit(dir.path(), "a.txt", "a\n");
        add_origin_main(dir.path());
        second_commit(dir.path());
        let status = probe(dir.path());
        assert_eq!(
            status,
            GitStatus {
                repo: true,
                has_head: true,
                base: Some("origin/main".to_string()),
                ..GitStatus::default()
            }
        );
        assert_eq!(
            select_scope(&status),
            Scope::VsBase("origin/main".to_string())
        );
    }

    #[test]
    fn probe_clean_tree_without_base_selects_nothing_to_review() {
        let dir = TempDir::new("no-base");
        init_and_commit(dir.path(), "a.txt", "a\n");
        second_commit(dir.path());
        let status = probe(dir.path());
        assert_eq!(status, repo_with_head());
        assert_eq!(select_scope(&status), Scope::NothingToReview);
    }

    #[test]
    fn probe_detached_head_selects_nothing_to_review() {
        let dir = TempDir::new("detached");
        init_and_commit(dir.path(), "a.txt", "a\n");
        add_origin_main(dir.path());
        second_commit(dir.path());
        git(dir.path(), &["checkout", "-q", "--detach"]);
        let status = probe(dir.path());
        // The base still resolves, but a detached HEAD holds.
        assert!(status.detached);
        assert_eq!(status.base.as_deref(), Some("origin/main"));
        assert_eq!(select_scope(&status), Scope::NothingToReview);
    }

    #[test]
    fn probe_dirty_tree_skips_the_base_chain() {
        let dir = TempDir::new("dirty-origin");
        init_and_commit(dir.path(), "a.txt", "a\n");
        add_origin_main(dir.path());
        second_commit(dir.path());
        // An unstaged change: had the base chain run, base would resolve.
        std::fs::write(dir.path().join("a.txt"), "c\n").unwrap();
        let status = probe(dir.path());
        assert_eq!(
            status,
            GitStatus {
                repo: true,
                has_head: true,
                unstaged: true,
                ..GitStatus::default()
            }
        );
        assert_eq!(select_scope(&status), Scope::Uncommitted);
    }

    #[test]
    fn probe_staged_only_selects_uncommitted() {
        let dir = TempDir::new("staged");
        init_and_commit(dir.path(), "base.txt", "b\n");
        std::fs::write(dir.path().join("added.txt"), "n\n").unwrap();
        git(dir.path(), &["add", "added.txt"]);
        let status = probe(dir.path());
        assert_eq!(
            status,
            GitStatus {
                repo: true,
                has_head: true,
                staged: true,
                ..GitStatus::default()
            }
        );
        assert_eq!(select_scope(&status), Scope::Uncommitted);
    }

    #[test]
    fn probe_untracked_only_selects_uncommitted() {
        let dir = TempDir::new("untracked");
        init_and_commit(dir.path(), "base.txt", "b\n");
        std::fs::write(dir.path().join("new.txt"), "n\n").unwrap();
        let status = probe(dir.path());
        assert_eq!(
            status,
            GitStatus {
                repo: true,
                has_head: true,
                untracked: true,
                ..GitStatus::default()
            }
        );
        assert_eq!(select_scope(&status), Scope::Uncommitted);
    }

    #[test]
    fn probe_failure_degrades_to_nothing_to_review() {
        // A directory that cannot be entered makes every probe command fail
        // to spawn; probe() warns once and returns the default status, which
        // holds instead of spawning a tool.
        let status = probe(Path::new("/nonexistent-dir-for-herdr-review-test"));
        assert_eq!(status, GitStatus::default());
        assert_eq!(select_scope(&status), Scope::NothingToReview);
    }

    /// Scripted probe-sequence tests: the runner records every argv and
    /// replays canned results, so these pin both the selected status and
    /// exactly which commands ran. Gated on unix, which can construct
    /// `ExitStatus` values.
    #[cfg(unix)]
    mod scripted {
        use super::*;
        use std::cell::{Cell, RefCell};
        use std::process::ExitStatus;

        const IS_REPO: &str = "rev-parse --is-inside-work-tree";
        const VERIFY_HEAD: &str = "rev-parse -q --verify HEAD";
        const DIFF: &str = "diff --quiet";
        const DIFF_CACHED: &str = "diff --cached --quiet";
        const LS_OTHERS: &str = "ls-files --others --exclude-standard";
        const SYMBOLIC_REF: &str = "symbolic-ref -q HEAD";
        const VERIFY_ORIGIN_HEAD: &str = "rev-parse -q --verify origin/HEAD";
        const VERIFY_ORIGIN_MAIN: &str = "rev-parse -q --verify origin/main";
        const VERIFY_ORIGIN_MASTER: &str = "rev-parse -q --verify origin/master";

        /// Replays one scripted result per call and records the argv.
        struct ScriptedGit {
            calls: RefCell<Vec<String>>,
            script: Vec<Result<Output, String>>,
            step: Cell<usize>,
        }

        impl ScriptedGit {
            fn new(script: Vec<Result<Output, String>>) -> Self {
                Self {
                    calls: RefCell::new(Vec::new()),
                    script,
                    step: Cell::new(0),
                }
            }

            fn run(&self, args: &[&str]) -> Result<Output, String> {
                self.calls.borrow_mut().push(args.join(" "));
                let step = self.step.get();
                self.step.set(step + 1);
                self.script[step].clone()
            }

            fn calls(&self) -> Vec<String> {
                self.calls.borrow().clone()
            }
        }

        /// A probe reply with the given exit code and stdout, empty stderr.
        fn reply(code: i32, stdout: &str) -> Result<Output, String> {
            Ok(Output {
                status: exit_status(code),
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }

        fn exit_status(code: i32) -> ExitStatus {
            use std::os::unix::process::ExitStatusExt;
            // Wait-status encoding on unix: the exit code lives in the high
            // byte.
            ExitStatusExt::from_raw(code << 8)
        }

        fn probe_with(sg: &ScriptedGit) -> Result<GitStatus, ProbeError> {
            Prober {
                run: &|args| sg.run(args),
            }
            .status()
        }

        #[test]
        fn non_repo_stops_after_the_first_command() {
            // Both "not a repository" signals: a failed exit, and success
            // with output other than `true`.
            for first in [reply(1, ""), reply(0, "false")] {
                let sg = ScriptedGit::new(vec![first]);
                let status = probe_with(&sg).unwrap();
                assert_eq!(status, GitStatus::default());
                assert_eq!(sg.calls(), [IS_REPO]);
            }
        }

        #[test]
        fn missing_head_stops_after_the_second_command() {
            let sg = ScriptedGit::new(vec![reply(0, "true"), reply(1, "")]);
            let status = probe_with(&sg).unwrap();
            assert_eq!(
                status,
                GitStatus {
                    repo: true,
                    ..GitStatus::default()
                }
            );
            assert_eq!(sg.calls(), [IS_REPO, VERIFY_HEAD]);
        }

        #[test]
        fn clean_tree_probes_everything_and_resolves_origin_head() {
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(0, ""),
                reply(0, ""),
                reply(0, ""),
                reply(0, "refs/heads/main\n"),
                reply(0, "sha2\n"), // origin/HEAD is the first hit
            ]);
            let status = probe_with(&sg).unwrap();
            assert_eq!(
                status,
                GitStatus {
                    repo: true,
                    has_head: true,
                    base: Some("origin/HEAD".to_string()),
                    ..GitStatus::default()
                }
            );
            assert_eq!(
                sg.calls(),
                [
                    IS_REPO,
                    VERIFY_HEAD,
                    DIFF,
                    DIFF_CACHED,
                    LS_OTHERS,
                    SYMBOLIC_REF,
                    VERIFY_ORIGIN_HEAD,
                ]
            );
        }

        #[test]
        fn dirty_tree_skips_the_detached_and_base_probes() {
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(1, ""),          // unstaged
                reply(1, ""),          // staged
                reply(0, "new.txt\n"), // untracked
            ]);
            let status = probe_with(&sg).unwrap();
            assert_eq!(
                status,
                GitStatus {
                    repo: true,
                    has_head: true,
                    unstaged: true,
                    staged: true,
                    untracked: true,
                    ..GitStatus::default()
                }
            );
            assert_eq!(
                sg.calls(),
                [IS_REPO, VERIFY_HEAD, DIFF, DIFF_CACHED, LS_OTHERS]
            );
            assert_eq!(select_scope(&status), Scope::Uncommitted);
        }

        #[test]
        fn base_chain_stops_at_the_first_hit() {
            // origin/HEAD does not verify, origin/main does: the probe never
            // asks about origin/master.
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(0, ""),
                reply(0, ""),
                reply(0, ""),
                reply(0, "refs/heads/main\n"),
                reply(1, ""),
                reply(0, "sha2\n"),
            ]);
            let status = probe_with(&sg).unwrap();
            assert_eq!(
                sg.calls(),
                [
                    IS_REPO,
                    VERIFY_HEAD,
                    DIFF,
                    DIFF_CACHED,
                    LS_OTHERS,
                    SYMBOLIC_REF,
                    VERIFY_ORIGIN_HEAD,
                    VERIFY_ORIGIN_MAIN,
                ]
            );
            assert_eq!(
                select_scope(&status),
                Scope::VsBase("origin/main".to_string())
            );
        }

        #[test]
        fn base_chain_exhausted_is_no_base() {
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(0, ""),
                reply(0, ""),
                reply(0, ""),
                reply(0, "refs/heads/main\n"),
                reply(1, ""),
                reply(1, ""),
                reply(1, ""),
            ]);
            let status = probe_with(&sg).unwrap();
            assert_eq!(status.base, None);
            assert_eq!(
                sg.calls(),
                [
                    IS_REPO,
                    VERIFY_HEAD,
                    DIFF,
                    DIFF_CACHED,
                    LS_OTHERS,
                    SYMBOLIC_REF,
                    VERIFY_ORIGIN_HEAD,
                    VERIFY_ORIGIN_MAIN,
                    VERIFY_ORIGIN_MASTER,
                ]
            );
            assert_eq!(select_scope(&status), Scope::NothingToReview);
        }

        #[test]
        fn detached_head_is_recorded_on_a_clean_tree() {
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(0, ""),
                reply(0, ""),
                reply(0, ""),
                reply(1, ""),       // symbolic-ref fails: detached
                reply(0, "sha2\n"), // the base still resolves
            ]);
            let status = probe_with(&sg).unwrap();
            assert!(status.detached);
            assert_eq!(status.base.as_deref(), Some("origin/HEAD"));
            assert_eq!(select_scope(&status), Scope::NothingToReview);
        }

        #[test]
        fn unexpected_diff_exit_code_is_a_probe_failure() {
            let sg = ScriptedGit::new(vec![
                reply(0, "true"),
                reply(0, "sha1\n"),
                reply(2, ""), // undocumented quiet-diff exit code
            ]);
            let err = probe_with(&sg).unwrap_err();
            assert_eq!(err.cmd, "git diff --quiet");
            assert!(err.err.contains("exit status: 2"), "{}", err.err);
            assert_eq!(sg.calls(), [IS_REPO, VERIFY_HEAD, DIFF]);
        }

        #[test]
        fn spawn_failure_is_a_probe_failure() {
            let sg = ScriptedGit::new(vec![Err(
                "No such file or directory (os error 2)".to_string()
            )]);
            let err = probe_with(&sg).unwrap_err();
            assert_eq!(err.cmd, "git rev-parse --is-inside-work-tree");
            assert!(err.err.contains("No such file or directory"), "{}", err.err);
            assert_eq!(sg.calls(), [IS_REPO]);
        }
    }
}
