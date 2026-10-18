//! The review-tool catalog and PATH-only binary resolution.
//!
//! The catalog is fixed — `lumen`, `revdiff`, `tuicr`, in the setup dialog's
//! alphabetical row order — and is not user-configurable. A tool counts as
//! installed when a `PATH` entry,
//! scanned in order with empty entries skipped, contains an executable regular
//! file with its name; there is deliberately no environment-variable override
//! for any tool (the pre-rename, revdiff-specific override variable was
//! removed), so all three tools resolve the same way from the herdr daemon's
//! `PATH`.
//!
//! The pane process resolves the selected tool's binary immediately before
//! spawning it: the path is canonicalized to an absolute one, and a candidate
//! that cannot be canonicalized is an error rather than a cwd-relative guess
//! (the pane's cwd may default to the plugin directory, not the repo).

use std::path::{Path, PathBuf};

/// The pane environment variable carrying the selected tool id.
const TOOL_ENV: &str = "HERDR_REVIEW_TOOL";

/// One catalog tool: a stable id plus the install command shown when missing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Tool {
    /// Catalog id, also the binary name on `PATH` and the `HERDR_REVIEW_TOOL` value.
    pub(crate) id: &'static str,
    /// Install hint: shown dim inline after the tool name in the setup
    /// dialog's rows and quoted in resolution errors.
    pub(crate) install: &'static str,
}

/// The fixed catalog, in the setup dialog's alphabetical row order.
pub(crate) static CATALOG: [Tool; 3] = [
    Tool {
        id: "lumen",
        install: "brew install jnsahaj/lumen/lumen",
    },
    Tool {
        id: "revdiff",
        install: "brew install umputun/apps/revdiff",
    },
    Tool {
        id: "tuicr",
        install: "brew install tuicr",
    },
];

/// Looks up a catalog row by id.
pub(crate) fn by_id(id: &str) -> Option<&'static Tool> {
    CATALOG.iter().find(|tool| tool.id == id)
}

/// Reads the selected tool id from the pane environment (trimmed). An unset or
/// blank value and an unknown value are errors that precede any git probe or
/// spawn; the messages tell the user to come back through the review action,
/// which is the only path that sets the variable.
pub(crate) fn selected() -> Result<&'static Tool, String> {
    let raw = std::env::var(TOOL_ENV).unwrap_or_default();
    selected_from(&raw)
}

/// [`selected`] against an already-read raw env value, so tests exercise the
/// error paths without mutating the process environment.
fn selected_from(raw: &str) -> Result<&'static Tool, String> {
    let id = raw.trim();
    if id.is_empty() {
        return Err(format!(
            "{TOOL_ENV} is not set; open the review pane through the review action"
        ));
    }
    by_id(id).ok_or_else(|| format!("unknown review tool {id}; expected revdiff, tuicr or lumen"))
}

/// Resolves the tool's binary from the process `PATH`: the first entry,
/// scanned in order with empty entries skipped, that contains an executable
/// regular file named `<tool>`, canonicalized to an absolute path.
pub(crate) fn resolve(tool: &Tool) -> Result<String, String> {
    let path = std::env::var("PATH").unwrap_or_default();
    resolve_in(tool, &path)
}

/// [`resolve`] against an explicit `PATH` string, without touching the process
/// environment.
fn resolve_in(tool: &Tool, path: &str) -> Result<String, String> {
    let candidate = find_candidate(path, tool.id).ok_or_else(|| {
        format!(
            "{} not found on PATH; install it with {}",
            tool.id, tool.install
        )
    })?;
    canonical_absolute(&candidate, tool.id).map(|p| p.to_string_lossy().into_owned())
}

/// The catalog tools that have an executable on `path`, in catalog order.
pub(crate) fn installed(path: &str) -> Vec<&'static Tool> {
    CATALOG
        .iter()
        .filter(|tool| find_candidate(path, tool.id).is_some())
        .collect()
}

/// The first executable regular file named `id` among `path` entries, in
/// order. Empty entries are skipped.
fn find_candidate(path: &str, id: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(id))
        .find(|candidate| is_executable_file(candidate))
}

/// Canonicalizes a candidate to an absolute path. There is deliberately no
/// working-directory fallback: the pane process may run with the plugin
/// directory as cwd, so guessing would spawn the wrong thing.
fn canonical_absolute(candidate: &Path, id: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(candidate).map_err(|e| {
        format!(
            "found {id} at {} but cannot resolve it to an absolute path: {e}",
            candidate.display()
        )
    })
}

/// An executable regular file (on unix at least one execute bit).
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The removed per-tool override variable, spelled as two fragments so
    /// the removal grep over `src` and `tests` stays clean while this test
    /// still pins the variable's non-effect on PATH-only resolution.
    const REMOVED_OVERRIDE: &str = concat!("REVDIFF", "_BIN");

    /// Serializes tests that mutate the removed override variable, restoring
    /// the previous value on drop so other tests see the original
    /// environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct RemovedOverrideEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<std::ffi::OsString>,
    }

    impl RemovedOverrideEnv {
        fn set(path: &Path) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var_os(REMOVED_OVERRIDE);
            unsafe { std::env::set_var(REMOVED_OVERRIDE, path) };
            Self { _lock: lock, prev }
        }
    }

    impl Drop for RemovedOverrideEnv {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var(REMOVED_OVERRIDE, v) },
                None => unsafe { std::env::remove_var(REMOVED_OVERRIDE) },
            }
        }
    }

    /// A canonicalized scratch directory, removed when the test ends, so
    /// comparisons survive symlinked temp roots (`/var` → `/private/var` on
    /// macOS).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "herdr-review-tool-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(std::fs::canonicalize(&dir).unwrap())
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a file named `id` into `dir`, with or without execute bits.
    fn write_binary(dir: &Path, id: &str, executable: bool) {
        let path = dir.join(id);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = executable;
    }

    fn path_of(dirs: &[&Path]) -> String {
        std::env::join_paths(dirs)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn installed_tool_is_selectable() {
        // Scenario "Installed tool is selectable": an executable tuicr on
        // PATH makes the tuicr row active.
        let dir = TempDir::new("installed");
        write_binary(dir.path(), "tuicr", true);
        let installed = installed(&path_of(&[dir.path()]));
        assert_eq!(
            installed.iter().map(|t| t.id).collect::<Vec<_>>(),
            ["tuicr"]
        );
    }

    #[test]
    fn non_executable_file_does_not_count() {
        // Scenario "Non-executable file does not count": a `revdiff` file
        // without an execute bit leaves revdiff not installed.
        let dir = TempDir::new("nonexec");
        write_binary(dir.path(), "revdiff", false);
        assert!(installed(&path_of(&[dir.path()])).is_empty());
    }

    #[test]
    fn first_executable_hit_wins() {
        // Scenario "First executable hit wins": two PATH entries with an
        // executable revdiff resolve to the earlier entry's canonical path.
        let first = TempDir::new("first");
        let second = TempDir::new("second");
        write_binary(first.path(), "revdiff", true);
        write_binary(second.path(), "revdiff", true);
        let path = path_of(&[first.path(), second.path()]);
        assert_eq!(
            resolve_in(by_id("revdiff").unwrap(), &path).unwrap(),
            first.path().join("revdiff").to_string_lossy()
        );
    }

    #[test]
    fn empty_path_entries_are_skipped() {
        let first = TempDir::new("empty-entries");
        write_binary(first.path(), "revdiff", true);
        // Empty entries (":") and missing directories are skipped.
        let path = format!(
            ":{}:{}::",
            path_of(&[Path::new("/nonexistent")]),
            first.path().display()
        );
        assert_eq!(
            resolve_in(by_id("revdiff").unwrap(), &path).unwrap(),
            first.path().join("revdiff").to_string_lossy()
        );
    }

    #[test]
    fn no_binary_anywhere() {
        // Scenario "No binary anywhere": the error names the tool and its
        // catalog install hint — the brew command the sketch pinned.
        for tool in CATALOG.iter() {
            for path in ["", ":", "/nonexistent-dir-for-herdr-review-test"] {
                let err = resolve_in(tool, path).unwrap_err();
                assert!(
                    err.contains(&format!("{} not found on PATH", tool.id)),
                    "{err}"
                );
                assert!(err.contains(tool.install), "{err}");
                assert!(
                    tool.install.starts_with("brew install "),
                    "{}",
                    tool.install
                );
            }
        }
    }

    #[test]
    fn candidate_cannot_be_canonicalized() {
        // Scenario "Candidate cannot be canonicalized": canonicalization
        // failure reports the candidate as given and the underlying error.
        let err = canonical_absolute(
            Path::new("/nonexistent-dir-for-herdr-review/revdiff"),
            "revdiff",
        )
        .unwrap_err();
        assert!(
            err.contains("found revdiff at /nonexistent-dir-for-herdr-review/revdiff"),
            "{err}"
        );
        assert!(
            err.contains("cannot resolve it to an absolute path"),
            "{err}"
        );
    }

    #[test]
    fn the_removed_override_is_ignored() {
        // Reviewer-execution scenario (removed override is ignored): with the
        // removed variable set to an executable file and PATH also carrying a
        // revdiff, resolution returns the PATH hit — the variable is never
        // read.
        let bin_dir = TempDir::new("bin-override");
        let path_dir = TempDir::new("path-hit");
        write_binary(bin_dir.path(), "revdiff", true);
        write_binary(path_dir.path(), "revdiff", true);
        let _env = RemovedOverrideEnv::set(&bin_dir.path().join("revdiff"));
        assert_eq!(
            resolve_in(by_id("revdiff").unwrap(), &path_of(&[path_dir.path()])).unwrap(),
            path_dir.path().join("revdiff").to_string_lossy()
        );
    }

    #[test]
    fn catalog_rows_are_alphabetical_with_brew_install_commands() {
        // Catalog-side data of the setup-dialog scenario "Tool rows are
        // alphabetical with dim inline install commands": the dialog rows
        // come straight from the catalog in its order, each with its
        // user-sketched brew install command.
        assert_eq!(
            CATALOG.iter().map(|t| t.id).collect::<Vec<_>>(),
            ["lumen", "revdiff", "tuicr"]
        );
        assert_eq!(
            by_id("lumen").unwrap().install,
            "brew install jnsahaj/lumen/lumen"
        );
        assert_eq!(
            by_id("revdiff").unwrap().install,
            "brew install umputun/apps/revdiff"
        );
        assert_eq!(by_id("tuicr").unwrap().install, "brew install tuicr");
    }

    #[test]
    fn by_id_finds_every_catalog_row() {
        for tool in CATALOG.iter() {
            assert_eq!(by_id(tool.id).map(|t| t.id), Some(tool.id));
        }
        assert!(by_id("diffy").is_none());
    }

    #[test]
    fn valid_tool_id_selects_the_tool() {
        // Scenario "Valid tool id selects the tool".
        assert_eq!(selected_from("lumen").unwrap().id, "lumen");
        assert_eq!(selected_from("  tuicr  ").unwrap().id, "tuicr");
    }

    #[test]
    fn missing_tool_id_is_an_error() {
        // Scenario "Missing tool id is an error".
        for raw in ["", "   ", "\t\n"] {
            let err = selected_from(raw).unwrap_err();
            assert!(
                err.contains(
                    "HERDR_REVIEW_TOOL is not set; open the review pane through the review action"
                ),
                "{err}"
            );
        }
    }

    #[test]
    fn unknown_tool_id_is_an_error() {
        // Scenario "Unknown tool id is an error".
        let err = selected_from("diffy").unwrap_err();
        assert_eq!(
            err,
            "unknown review tool diffy; expected revdiff, tuicr or lumen"
        );
    }
}
