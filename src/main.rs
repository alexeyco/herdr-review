//! herdr plugin that reviews the current workspace with a review tool.
//!
//! The binary is tri-mode, dispatched from the plugin environment:
//! - action mode (`HERDR_PLUGIN_ACTION_ID` set, entrypoint unset) resolves the
//!   repository from the plugin context, reads `config.toml` once, and opens
//!   the review pane directly when the configured tool is usable — otherwise
//!   it opens the read-only setup popup — then exits immediately.
//! - setup mode (`HERDR_PLUGIN_ENTRYPOINT_ID=setup`) runs inside that popup:
//!   it shows the one universal layout — the config file path, the fixed
//!   catalog rows with their install commands, and the repository URL —
//!   and closes on any key.
//! - review-pane mode (`HERDR_PLUGIN_ENTRYPOINT_ID=review`) runs inside the
//!   review pane: it spawns the chosen tool on the pane tty, collects its
//!   annotations, and delivers them to the agent pane that requested the
//!   review.
//!
//! Plugin docs: https://herdr.dev/docs/plugins/

mod config;
mod context;
mod convert;
mod git;
mod herdr;
mod review;
mod setup;
mod theme;
mod tool;
mod tuicr;

fn main() {
    if let Err(e) = run() {
        eprintln!("herdr-review: {e}");
        // Plugin panes auto-close when this process exits, so pane-mode errors
        // must block until the user has read the message.
        if env_set("HERDR_PLUGIN_ENTRYPOINT_ID") {
            review::hold_pane();
        }
        std::process::exit(1);
    }
}

/// Which entry point the process should run.
#[derive(Debug, PartialEq, Eq)]
enum RunMode {
    /// `HERDR_PLUGIN_ENTRYPOINT_ID=review`: the review pane.
    Review,
    /// `HERDR_PLUGIN_ENTRYPOINT_ID=setup`: the read-only setup dialog popup.
    Setup,
    /// Only `HERDR_PLUGIN_ACTION_ID` is set: the action.
    Action,
    /// Neither is set: invoked outside herdr.
    Outside,
}

/// Picks the mode from the entry-point environment. The entrypoint is checked
/// first, so pane modes win when both variables are set, matching herdr's
/// action-spawns-pane nesting; a blank or whitespace-only value counts as
/// unset, so herdr passing empty strings never silently selects a mode. Any
/// other non-blank value is an error naming it.
fn run_mode() -> Result<RunMode, String> {
    match env_value("HERDR_PLUGIN_ENTRYPOINT_ID") {
        Some(id) => match id.as_str() {
            "review" => Ok(RunMode::Review),
            "setup" => Ok(RunMode::Setup),
            other => Err(format!(
                "unknown plugin entrypoint {other}; expected review or setup"
            )),
        },
        None if env_set("HERDR_PLUGIN_ACTION_ID") => Ok(RunMode::Action),
        None => Ok(RunMode::Outside),
    }
}

fn run() -> Result<(), String> {
    match run_mode()? {
        RunMode::Review => review::run_pane(),
        RunMode::Setup => setup::run(),
        RunMode::Action => review::run_action(),
        RunMode::Outside => Err("invoked outside herdr plugin context".to_string()),
    }
}

fn env_set(key: &str) -> bool {
    env_value(key).is_some()
}

/// A trimmed, non-blank env value, or `None`.
fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    /// Serializes tests that mutate the mode environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Sets both mode variables for the duration of a test, restoring the
    /// previous values on drop.
    struct ModeEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev_entry: Option<OsString>,
        prev_action: Option<OsString>,
    }

    impl ModeEnv {
        fn set(entry: Option<&str>, action: Option<&str>) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev_entry = std::env::var_os("HERDR_PLUGIN_ENTRYPOINT_ID");
            let prev_action = std::env::var_os("HERDR_PLUGIN_ACTION_ID");
            set_var("HERDR_PLUGIN_ENTRYPOINT_ID", entry);
            set_var("HERDR_PLUGIN_ACTION_ID", action);
            Self {
                _lock: lock,
                prev_entry,
                prev_action,
            }
        }
    }

    impl Drop for ModeEnv {
        fn drop(&mut self) {
            match &self.prev_entry {
                Some(v) => unsafe { std::env::set_var("HERDR_PLUGIN_ENTRYPOINT_ID", v) },
                None => unsafe { std::env::remove_var("HERDR_PLUGIN_ENTRYPOINT_ID") },
            }
            match &self.prev_action {
                Some(v) => unsafe { std::env::set_var("HERDR_PLUGIN_ACTION_ID", v) },
                None => unsafe { std::env::remove_var("HERDR_PLUGIN_ACTION_ID") },
            }
        }
    }

    fn set_var(key: &str, value: Option<&str>) {
        match value {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn only_the_entry_point_variable_is_set() {
        // Scenario "Only the entry-point variable is set": each known
        // entrypoint selects its own pane mode.
        {
            let _env = ModeEnv::set(Some("review"), None);
            assert_eq!(run_mode().unwrap(), RunMode::Review);
        }
        {
            let _env = ModeEnv::set(Some("setup"), None);
            assert_eq!(run_mode().unwrap(), RunMode::Setup);
        }
    }

    #[test]
    fn unknown_entrypoint_is_an_error() {
        // Scenario "Unknown entrypoint is an error".
        for (entry, action) in [("editor", None), ("editor", Some("review"))] {
            let _env = ModeEnv::set(Some(entry), action);
            let err = run_mode().unwrap_err();
            assert_eq!(
                err,
                "unknown plugin entrypoint editor; expected review or setup"
            );
        }
    }

    #[test]
    fn both_mode_variables_are_set() {
        // Scenario "Both mode variables are set": the entrypoint wins.
        {
            let _env = ModeEnv::set(Some("review"), Some("review"));
            assert_eq!(run_mode().unwrap(), RunMode::Review);
        }
        {
            let _env = ModeEnv::set(Some("setup"), Some("review"));
            assert_eq!(run_mode().unwrap(), RunMode::Setup);
        }
    }

    #[test]
    fn neither_mode_variable_is_set() {
        // Scenario "Neither mode variable is set".
        let _env = ModeEnv::set(None, None);
        assert_eq!(run_mode().unwrap(), RunMode::Outside);
    }

    #[test]
    fn action_only_is_action() {
        let _env = ModeEnv::set(None, Some("review"));
        assert_eq!(run_mode().unwrap(), RunMode::Action);
    }

    #[test]
    fn whitespace_only_entry_point_with_a_set_action() {
        // Scenario "Whitespace-only entry point with a set action".
        let _env = ModeEnv::set(Some("   "), Some("review"));
        assert_eq!(run_mode().unwrap(), RunMode::Action);
    }

    #[test]
    fn both_variables_set_to_blank_values() {
        // Scenario "Both variables set to blank values".
        let _env = ModeEnv::set(Some("\t"), Some("  \n"));
        assert_eq!(run_mode().unwrap(), RunMode::Outside);
    }

    #[test]
    fn manifest_declares_paired_entrypoints_for_one_binary() {
        // Scenario "Both entry points resolve to the same binary": all three
        // entries — the action plus the review and setup panes — declare the
        // identical argv command array, and the environment dispatch selects
        // which role the launched process plays. The identity is the
        // namespaced plugin id with the `open` action (invoke command
        // `alexeyco.review.open`), and the setup pane declares its popup
        // sizing keys. The version stays in sync with `Cargo.toml` (the CI
        // job enforces it on release tags; this keeps the pair honest
        // between releases).
        let manifest: toml::Table = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
        assert_eq!(
            manifest.get("id").and_then(|v| v.as_str()),
            Some("alexeyco.review")
        );
        assert_eq!(
            manifest.get("name").and_then(|v| v.as_str()),
            Some("Review")
        );
        assert_eq!(
            manifest.get("min_herdr_version").and_then(|v| v.as_str()),
            Some("0.9.0")
        );
        assert_eq!(
            manifest.get("version").and_then(|v| v.as_str()),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let command = vec![toml::Value::from("./target/release/herdr-review")];
        let actions = manifest.get("actions").and_then(|v| v.as_array()).unwrap();
        let panes = manifest.get("panes").and_then(|v| v.as_array()).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(panes.len(), 2);
        assert_eq!(actions[0].get("id").and_then(|v| v.as_str()), Some("open"));
        assert_eq!(
            actions[0].get("contexts").and_then(|v| v.as_array()),
            Some(&vec![toml::Value::from("workspace")])
        );
        assert_eq!(
            actions[0]
                .get("command")
                .and_then(|v| v.as_array())
                .unwrap(),
            &command
        );
        for pane in panes {
            assert_eq!(
                pane.get("command").and_then(|v| v.as_array()).unwrap(),
                &command
            );
        }
        let review = &panes[0];
        let setup = &panes[1];
        assert_eq!(review.get("id").and_then(|v| v.as_str()), Some("review"));
        assert_eq!(
            review.get("placement").and_then(|v| v.as_str()),
            Some("tab")
        );
        assert_eq!(setup.get("id").and_then(|v| v.as_str()), Some("setup"));
        assert_eq!(
            setup.get("title").and_then(|v| v.as_str()),
            Some("Review tool setup")
        );
        assert_eq!(
            setup.get("placement").and_then(|v| v.as_str()),
            Some("popup")
        );
        assert_eq!(setup.get("width").and_then(|v| v.as_str()), Some("60%"));
        assert_eq!(setup.get("height").and_then(|v| v.as_integer()), Some(15));
    }
}
