//! tuicr session adapter: collecting review comments after the TUI exits.
//!
//! tuicr has no `--output` file and no annotations exit code (codes 0/1/2
//! only); its review sessions persist on disk. The pane records a launch
//! timestamp before spawning the TUI, then collects this run's comments in
//! order: predict the session slug
//! `[<owner>/]<repo>@<sanitized-branch>/worktree/<7-char-head>` from read-only
//! git probes and ask `tuicr review comments --session <slug>`; when that
//! fails or names no session, fall back to `tuicr review list --repo .`,
//! select the newest session updated after the launch timestamp, and fetch
//! its comments. Both paths failing (spawn error or invalid JSON) is the
//! adapter's error detail; an empty comment list is a clean result.
//!
//! tuicr emits JSON by default for every `review` subcommand — there is no
//! `--json` flag (0.27.0 rejects it), so no flag is passed.

use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use serde_json::Value;

use crate::convert::{self, Comment};

/// Collects this run's tuicr comments. `run_tuicr` and `run_git` execute one
/// command in the review directory and return its stdout (production spawns
/// the resolved binary / git; tests script replies).
pub(crate) fn collect_comments(
    dir: &Path,
    launched_at: SystemTime,
    run_tuicr: &dyn Fn(&[&str]) -> Result<String, String>,
    run_git: &dyn Fn(&[&str]) -> Result<String, String>,
) -> Result<Vec<Comment>, String> {
    let slug = predict_slug(dir, run_git);
    // A non-empty answer for the predicted slug is done in one call.
    match fetch_comments(run_tuicr, &slug) {
        Ok(comments) if !comments.is_empty() => Ok(comments),
        predicted => fallback_lookup(run_tuicr, launched_at, predicted.err()),
    }
}

/// The list-fallback half: `review list --repo .` (newest first), the newest
/// session updated after the launch timestamp, then its comments. `first` is
/// the predicted-slug failure detail, folded into the error on the way out.
fn fallback_lookup(
    run_tuicr: &dyn Fn(&[&str]) -> Result<String, String>,
    launched_at: SystemTime,
    first: Option<String>,
) -> Result<Vec<Comment>, String> {
    let list = match run_tuicr(&["review", "list", "--repo", "."]) {
        Ok(list) => list,
        Err(e) => {
            return Err(detail(
                first,
                format!("tuicr review list --repo . failed: {e}"),
            ));
        }
    };
    let sessions = match parse_sessions(&list) {
        Ok(sessions) => sessions,
        Err(e) => return Err(detail(first, format!("invalid session list: {e}"))),
    };
    // The list is newest-first, so the first session updated after the launch
    // is the newest one this run touched.
    let Some(session) = sessions
        .into_iter()
        .find(|s| s.updated_at.map(|t| t > launched_at).unwrap_or(false))
    else {
        // No session was created or updated by this run: the user quit
        // without a session, which is a clean empty result.
        return Ok(Vec::new());
    };
    match fetch_comments(run_tuicr, &session.slug) {
        Ok(comments) => Ok(comments),
        Err(e) => Err(detail(
            first,
            format!(
                "tuicr review comments --session {} failed: {e}",
                session.slug
            ),
        )),
    }
}

/// The adapter error detail: the fallback failure, prefixed with the
/// predicted-slug failure when there was one.
fn detail(first: Option<String>, last: String) -> String {
    match first {
        Some(first) => format!("{first}; {last}"),
        None => last,
    }
}

/// Runs `tuicr review comments --session <slug>` and parses the JSON list.
fn fetch_comments(
    run: &dyn Fn(&[&str]) -> Result<String, String>,
    slug: &str,
) -> Result<Vec<Comment>, String> {
    let json = run(&["review", "comments", "--session", slug])?;
    convert::parse_comments(&json)
}

/// Predicts the session slug from read-only git probes: the `origin` remote
/// URL for `[<owner>/]<repo>` (owner segment omitted when no remote exists,
/// repo name falling back to the directory basename), `git branch
/// --show-current` for the branch, and `git rev-parse --short=7 HEAD` for the
/// head. tuicr's branch sanitizer is unpublished; the identity mapping is the
/// starting guess and the list fallback covers misses.
fn predict_slug(dir: &Path, run_git: &dyn Fn(&[&str]) -> Result<String, String>) -> String {
    let origin_url = git_output(run_git, &["remote", "get-url", "origin"]);
    let branch = git_output(run_git, &["branch", "--show-current"]);
    let head = git_output(run_git, &["rev-parse", "--short=7", "HEAD"]);
    let (owner, repo) = match remote_repo_parts(&origin_url) {
        Some(parts) => parts,
        None => (
            None,
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    };
    match owner {
        Some(owner) => format!("{owner}/{repo}@{branch}/worktree/{head}"),
        None => format!("{repo}@{branch}/worktree/{head}"),
    }
}

/// One trimmed stdout line of a successful probe; failures yield an empty
/// string (a missing remote or branch is not an error here).
fn git_output(run_git: &dyn Fn(&[&str]) -> Result<String, String>, args: &[&str]) -> String {
    run_git(args)
        .map(|out| out.trim().to_string())
        .unwrap_or_default()
}

/// Splits a git remote URL into `(<owner>, <repo>)`. The owner is the path
/// segment above the repo name; a repo without one yields no owner. Handles
/// `https://host/owner/repo.git` and scp-like `git@host:owner/repo.git`.
fn remote_repo_parts(url: &str) -> Option<(Option<String>, String)> {
    let url = url.trim();
    let path = match url.split_once("://") {
        // scheme://host/owner/repo(.git)
        Some((_, rest)) => match rest.split_once('/') {
            Some((_, path)) => path,
            None => return None,
        },
        // scp-like: git@host:owner/repo(.git)
        None => match url.split_once(':') {
            Some((_, path)) => path,
            None => url,
        },
    };
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let repo = segments.pop()?.trim_end_matches(".git");
    if repo.is_empty() {
        return None;
    }
    let owner = segments.pop().map(str::to_string);
    Some((owner, repo.to_string()))
}

/// One persisted session from `tuicr review list`.
struct Session {
    slug: String,
    /// Parsed `updated_at` (RFC3339); `None` when missing or unparsable, in
    /// which case the session cannot be proven to be this run's.
    updated_at: Option<SystemTime>,
}

/// Parses the `review list` JSON array. Sessions without a usable slug are
/// skipped; an unparsable `updated_at` leaves the timestamp unknown.
fn parse_sessions(json: &str) -> Result<Vec<Session>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
    let arr = v.as_array().ok_or_else(|| "not a JSON array".to_string())?;
    let sessions = arr
        .iter()
        .filter_map(|item| {
            let slug = item.get("slug")?.as_str()?.trim().to_string();
            if slug.is_empty() {
                return None;
            }
            let updated_at = item
                .get("updated_at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339);
            Some(Session { slug, updated_at })
        })
        .collect();
    Ok(sessions)
}

/// Parses an RFC3339 timestamp (`2026-05-22T17:20:00Z`, optional fractional
/// seconds, `Z` or `±HH:MM` offset) into a `SystemTime`. tuicr emits RFC3339
/// strings for every timestamp.
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let digits = |from: usize, to: usize| -> Option<i64> {
        let text = s.get(from..to)?;
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    };
    let year = digits(0, 4)?;
    if bytes[4] != b'-' {
        return None;
    }
    let month = digits(5, 7)?;
    if bytes[7] != b'-' {
        return None;
    }
    let day = digits(8, 10)?;
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    let hour = digits(11, 13)?;
    if bytes[13] != b':' {
        return None;
    }
    let minute = digits(14, 16)?;
    if bytes[16] != b':' {
        return None;
    }
    let second = digits(17, 19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=59).contains(&second) {
        return None;
    }
    // Optional fractional seconds, then the zone.
    let mut rest = &s[19..];
    let mut nanos = 0u32;
    if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        if end == 0 {
            return None;
        }
        let mut millis = frac[..end.min(3)].to_string();
        while millis.len() < 3 {
            millis.push('0');
        }
        nanos = millis.parse::<u32>().ok()? * 1_000_000;
        rest = &frac[end..];
    }
    let offset_seconds = match rest {
        "Z" | "z" | "" => 0,
        z if z.len() == 6 => {
            let sign = match z.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if z.as_bytes()[3] != b':' {
                return None;
            }
            let oh: i64 = z.get(1..3)?.parse().ok()?;
            let om: i64 = z.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    let days = days_from_civil(year, month as u32, day as u32);
    let epoch_seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset_seconds;
    if epoch_seconds < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(epoch_seconds as u64, nanos))
}

/// Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Runs the resolved tuicr binary in the review directory with captured
/// output (a non-interactive query run after the TUI exited).
pub(crate) fn exec_tuicr(bin: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            format!(
                "failed to run {bin} {} in {}: {e}",
                args.join(" "),
                dir.display()
            )
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{bin} {} exited with {}: {}",
            args.join(" "),
            output.status,
            snippet(&output.stderr)
        ))
    }
}

/// Runs a read-only git probe in the review directory with captured output.
pub(crate) fn exec_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            format!(
                "failed to run git {} in {}: {e}",
                args.join(" "),
                dir.display()
            )
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {} exited with {}: {}",
            args.join(" "),
            output.status,
            snippet(&output.stderr)
        ))
    }
}

/// Trims and truncates an error snippet to 200 characters.
fn snippet(s: &[u8]) -> String {
    let text = String::from_utf8_lossy(s).trim().to_string();
    match text.char_indices().nth(200) {
        Some((i, _)) => text[..i].to_string(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The comments fixture the fake tuicr returns for the right slug.
    const COMMENTS: &str = r#"[{"path":"src/main.rs","start_line":42,"end_line":42,"side":"new","content":"Handle the empty case here."}]"#;

    /// A launch timestamp safely before every fixture session's update time.
    fn launched_before(update: &str) -> SystemTime {
        parse_rfc3339(update).unwrap() - Duration::from_secs(3600)
    }

    /// A tuicr runner that answers `review comments --session <slug>` calls
    /// from `replies` and `review list` from `list`, recording every argv.
    struct FakeTuicr {
        replies: Vec<(String, String)>,
        list: Result<String, String>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl FakeTuicr {
        fn new(replies: Vec<(String, String)>, list: Result<String, String>) -> Self {
            Self {
                replies,
                list,
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn run(&self, args: &[&str]) -> Result<String, String> {
            self.calls.borrow_mut().push(args.join(" "));
            if args[..2] == ["review", "list"] {
                return self.list.clone();
            }
            let slug = args.iter().rev().find(|a| !a.is_empty()).copied();
            let key = format!("review comments --session {}", slug.unwrap_or_default());
            for (command, reply) in &self.replies {
                if *command == key {
                    return Ok(reply.clone());
                }
            }
            Err(format!(
                "session '{}' was not found",
                slug.unwrap_or_default()
            ))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    /// A git runner answering the three slug probes.
    fn fake_git(
        origin: Option<&str>,
        branch: &str,
        head: &str,
    ) -> impl Fn(&[&str]) -> Result<String, String> {
        move |args: &[&str]| {
            let joined = args.join(" ");
            match joined.as_str() {
                "remote get-url origin" => match origin {
                    Some(url) => Ok(format!("{url}\n")),
                    None => Err("no remote".to_string()),
                },
                "branch --show-current" => Ok(format!("{branch}\n")),
                "rev-parse --short=7 HEAD" => Ok(format!("{head}\n")),
                _ => panic!("unexpected git probe: {joined}"),
            }
        }
    }

    #[test]
    fn predicted_slug_answers_directly() {
        // Scenario "Predicted slug answers directly": the slug is built from
        // the origin URL, branch, and 7-char head; no `review list` call.
        let tuicr = FakeTuicr::new(
            vec![(
                "review comments --session alexeyco/herdr-review@main/worktree/abc1234".to_string(),
                COMMENTS.to_string(),
            )],
            Err("must not be called".to_string()),
        );
        let git = fake_git(
            Some("git@github.com:alexeyco/herdr-review.git"),
            "main",
            "abc1234",
        );
        let comments = collect_comments(
            Path::new("/repo"),
            launched_before("2026-10-17T12:00:00Z"),
            &|args| tuicr.run(args),
            &git,
        )
        .unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].path, "src/main.rs");
        assert_eq!(
            tuicr.calls(),
            ["review comments --session alexeyco/herdr-review@main/worktree/abc1234"]
        );
    }

    #[test]
    fn fallback_list_picks_the_session_created_by_this_run() {
        // Scenario "Fallback list picks the session created by this run": the
        // predicted slug misses; the list returns sessions of which exactly
        // one was updated after the launch timestamp, and its comments are
        // fetched.
        let tuicr = FakeTuicr::new(
            vec![(
                "review comments --session owner/repo@feature/worktree/def2345".to_string(),
                COMMENTS.to_string(),
            )],
            Ok(
                r#"[
                    {"slug":"owner/repo@feature/worktree/abc1234","comment_count":1,"updated_at":"2026-10-10T00:00:00Z"},
                    {"slug":"owner/repo@feature/worktree/def2345","comment_count":1,"updated_at":"2026-10-17T12:00:00Z"}
                ]"#
                .to_string(),
            ),
        );
        let git = fake_git(None, "feature", "def2345");
        let launched = launched_before("2026-10-17T12:00:00Z");
        let comments =
            collect_comments(Path::new("repo"), launched, &|args| tuicr.run(args), &git).unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(
            tuicr.calls(),
            [
                "review comments --session repo@feature/worktree/def2345",
                "review list --repo .",
                "review comments --session owner/repo@feature/worktree/def2345",
            ]
        );
    }

    #[test]
    fn adapter_failure_keeps_evidence() {
        // Scenario "Adapter failure keeps evidence": both the predicted-slug
        // call and the list fallback fail, so the error detail names both.
        let tuicr = FakeTuicr::new(Vec::new(), Err("spawn error".to_string()));
        let git = fake_git(Some("https://github.com/o/r.git"), "main", "abc1234");
        let err = collect_comments(
            Path::new("/repo"),
            SystemTime::now(),
            &|args| tuicr.run(args),
            &git,
        )
        .unwrap_err();
        assert!(
            err.contains("session 'o/r@main/worktree/abc1234' was not found"),
            "{err}"
        );
        assert!(
            err.contains("tuicr review list --repo . failed: spawn error"),
            "{err}"
        );
        // The review layer prefixes this with "tuicr session lookup failed:".
        assert!(!err.starts_with("tuicr session lookup failed"), "{err}");
    }

    #[test]
    fn invalid_list_json_is_an_adapter_failure() {
        let tuicr = FakeTuicr::new(Vec::new(), Ok("{not json".to_string()));
        let git = fake_git(None, "main", "abc1234");
        let err = collect_comments(
            Path::new("/repo"),
            SystemTime::now(),
            &|args| tuicr.run(args),
            &git,
        )
        .unwrap_err();
        assert!(err.contains("invalid session list"), "{err}");
    }

    #[test]
    fn no_session_from_this_run_is_a_clean_empty_result() {
        // The user quit without creating a session: the list holds only old
        // sessions, so the result is empty (no annotations), not an error.
        let tuicr = FakeTuicr::new(
            Vec::new(),
            Ok(
                r#"[{"slug":"owner/repo@feature/worktree/abc1234","comment_count":1,"updated_at":"2020-01-01T00:00:00Z"}]"#
                    .to_string(),
            ),
        );
        let git = fake_git(None, "feature", "def2345");
        let comments = collect_comments(
            Path::new("repo"),
            SystemTime::now(),
            &|args| tuicr.run(args),
            &git,
        )
        .unwrap();
        assert!(comments.is_empty());
    }

    #[test]
    fn nonzero_tuicr_exit_skips_the_adapter() {
        // Scenario "Nonzero tuicr exit skips the adapter": the review layer
        // never calls the adapter on a nonzero TUI exit (pinned there); here
        // the adapter reports a failed list call, not a silent success.
        let tuicr = FakeTuicr::new(Vec::new(), Err("exit status: 1".to_string()));
        let git = fake_git(None, "main", "abc1234");
        assert!(
            collect_comments(
                Path::new("/repo"),
                SystemTime::now(),
                &|args| { tuicr.run(args) },
                &git
            )
            .is_err()
        );
    }

    #[test]
    fn slug_prediction_covers_the_remote_url_shapes() {
        for (url, expected) in [
            (
                Some("https://github.com/alexeyco/herdr-review.git"),
                "alexeyco/herdr-review@main/worktree/abc1234",
            ),
            (
                Some("git@github.com:alexeyco/herdr-review.git"),
                "alexeyco/herdr-review@main/worktree/abc1234",
            ),
            (
                Some("ssh://git@example.com:2222/owner/repo.git"),
                "owner/repo@main/worktree/abc1234",
            ),
            (
                Some("https://example.com/plain-repo.git"),
                "plain-repo@main/worktree/abc1234",
            ),
            (None, "myrepo@main/worktree/abc1234"),
        ] {
            let git = fake_git(url, "main", "abc1234");
            let slug = predict_slug(Path::new("/x/myrepo"), &git);
            assert_eq!(slug, expected, "url {url:?}");
        }
    }

    #[test]
    fn remote_repo_parts_rejects_unusable_urls() {
        assert_eq!(remote_repo_parts("https://host"), None);
        assert_eq!(remote_repo_parts("https://host/"), None);
        assert_eq!(remote_repo_parts("https://host/.git"), None);
    }

    #[test]
    fn rfc3339_parses_utc_offsets_and_fractions() {
        let t = parse_rfc3339("1970-01-01T00:00:00Z").unwrap();
        assert_eq!(t, SystemTime::UNIX_EPOCH);
        // An offset shifts the epoch instant.
        let t = parse_rfc3339("1970-01-01T02:00:00+02:00").unwrap();
        assert_eq!(t, SystemTime::UNIX_EPOCH);
        // Fractional seconds parse to nanoseconds.
        let t = parse_rfc3339("1970-01-01T00:00:01.5Z").unwrap();
        assert_eq!(
            t.duration_since(SystemTime::UNIX_EPOCH).unwrap(),
            Duration::new(1, 500_000_000)
        );
        assert_eq!(
            parse_rfc3339("2026-05-22T17:20:00Z").unwrap(),
            parse_rfc3339("2026-05-22 17:20:00z").unwrap()
        );
    }

    #[test]
    fn rfc3339_rejects_malformed_timestamps() {
        for s in [
            "",
            "not-a-time",
            "2026-05-22",
            "2026-13-01T00:00:00Z",
            "2026-05-22T25:00:00Z",
            "2026-05-22T17:20:00",
            "2026-05-22T17:20:00+2:00",
            "1969-12-31T23:59:59Z",
        ] {
            assert!(parse_rfc3339(s).is_none(), "{s}");
        }
    }
}
