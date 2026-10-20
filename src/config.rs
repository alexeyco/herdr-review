//! Placement and tool settings read from the plugin's `config.toml`.
//!
//! Invalid or unavailable settings warn and fall back to defaults. The file
//! has flat `placement`, `direction`, and `tool` keys; unknown keys are ignored.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::adapters::{herdr, tool};

/// Environment variable carrying the plugin config directory.
const CONFIG_DIR_ENV: &str = "HERDR_PLUGIN_CONFIG_DIR";

/// Where the review pane opens; passed verbatim as the `--placement` value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Placement {
    /// A new tab (the default and the refused-popup fallback).
    #[default]
    Tab,
    /// A split pane, the only placement that takes a `--direction` flag.
    Split,
    /// An overlay pane.
    Overlay,
    /// Zoomed over the caller's tab.
    Zoomed,
}

impl Placement {
    /// The argv value passed verbatim to `plugin pane open --placement`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Placement::Tab => "tab",
            Placement::Split => "split",
            Placement::Overlay => "overlay",
            Placement::Zoomed => "zoomed",
        }
    }
}

/// Which way a split pane opens. `Auto` lets herdr choose: no `--direction`
/// flag is passed at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Direction {
    /// To the right of the caller (the default).
    #[default]
    Right,
    /// Below the caller.
    Down,
    /// Let herdr choose; the flag is omitted.
    Auto,
}

impl Direction {
    /// The `--direction` argv value, `None` for `auto`.
    pub(crate) fn argv_value(self) -> Option<&'static str> {
        match self {
            Direction::Right => Some("right"),
            Direction::Down => Some("down"),
            Direction::Auto => None,
        }
    }
}

/// The flat-key settings. Defaults: `placement = "tab"`,
/// `direction = "right"`, `tool` unset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Config {
    /// The `placement` key.
    pub(crate) placement: Placement,
    /// The `direction` key; only applied when placement is `split`.
    pub(crate) direction: Direction,
    /// The `tool` key: a valid catalog id names the review tool the action
    /// checks against `PATH`; unset routes the launch to the setup dialog.
    pub(crate) tool: Option<tool::ReviewTool>,
}

/// Reads the config once per action launch. Never fails: every degradation
/// prints one `herdr-review: warning:` line and returns defaults.
pub(crate) fn load() -> Config {
    let (config, warnings) = load_with(resolve_config_dir());
    print_warnings(&warnings);
    config
}

/// Resolves the config directory without reading the file — the setup
/// dialog's only environment interaction, feeding its config-file path
/// line. A non-blank `HERDR_PLUGIN_CONFIG_DIR` wins outright; otherwise
/// `herdr plugin config-dir alexeyco.review` answers. Absent when neither
/// resolves: the dialog then prints that command as its hint. Unlike
/// [`load`], a resolution failure is silent — the dialog is informational
/// only.
pub(crate) fn resolve_dir() -> Option<PathBuf> {
    resolve_config_dir().ok()
}

/// Maps a resolved (or failed) directory to the settings plus warnings.
fn load_with(dir: Result<PathBuf, String>) -> (Config, Vec<String>) {
    match dir {
        Ok(dir) => load_from_dir(&dir),
        Err(reason) => (
            Config::default(),
            vec![format!(
                "config directory unavailable: {reason}; using default settings"
            )],
        ),
    }
}

/// Prints one warning line per degradation.
fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("herdr-review: warning: {warning}");
    }
}

/// Resolves the config directory: a non-blank `HERDR_PLUGIN_CONFIG_DIR` wins
/// outright and no callback is made; otherwise `herdr plugin config-dir
/// alexeyco.review` answers. The `Err` reason maps to the single "config
/// directory unavailable" warning plus defaults.
fn resolve_config_dir() -> Result<PathBuf, String> {
    resolve_config_dir_with(
        std::env::var(CONFIG_DIR_ENV).ok().as_deref(),
        &herdr::config_dir,
    )
}

/// [`resolve_config_dir`] with the env value and the CLI callback injected, so
/// the resolution matrix is testable without the process environment.
fn resolve_config_dir_with(
    raw_env: Option<&str>,
    cli: &dyn Fn() -> Result<String, String>,
) -> Result<PathBuf, String> {
    if let Some(dir) = raw_env.map(str::trim).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    cli().map(PathBuf::from)
}

/// Reads `<dir>/config.toml` once. A file that does not exist is defaults with
/// one warning; an existing-but-unreadable file degrades like corrupt content.
fn load_from_dir(dir: &Path) -> (Config, Vec<String>) {
    let path = dir.join("config.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (
                Config::default(),
                vec![format!(
                    "no config.toml at {}; using default settings",
                    path.display()
                )],
            );
        }
        Err(e) => {
            return (
                Config::default(),
                vec![format!(
                    "cannot read config.toml at {}: {e}; using default settings",
                    path.display()
                )],
            );
        }
    };
    parse_config(&content)
}

/// Two-phase parsing: first the whole file as a flat TOML table — unparseable
/// content is all defaults with one warning — then per-key validation so a bad
/// value only ever falls back to that key's own default while the other keys
/// keep their effect. Unknown keys are ignored.
fn parse_config(content: &str) -> (Config, Vec<String>) {
    let raw: RawConfig = match toml::from_str(content) {
        Ok(table) => table,
        Err(e) => {
            return (
                Config::default(),
                vec![format!("invalid config.toml: {e}; using default settings")],
            );
        }
    };
    let mut warnings = Vec::new();
    let mut config = Config::default();
    if let Some(value) = raw.placement {
        match value.as_str() {
            Some("tab") => config.placement = Placement::Tab,
            Some("split") => config.placement = Placement::Split,
            Some("overlay") => config.placement = Placement::Overlay,
            Some("zoomed") => config.placement = Placement::Zoomed,
            Some("popup") => warnings.push(POPUP_REFUSED.to_string()),
            _ => warnings.push(invalid_placement_warning(&value)),
        }
    }
    if let Some(value) = raw.direction {
        match value.as_str() {
            Some("right") => config.direction = Direction::Right,
            Some("down") => config.direction = Direction::Down,
            Some("auto") => config.direction = Direction::Auto,
            _ => {
                config.direction = Direction::Auto;
                warnings.push(invalid_direction_warning(&value));
            }
        }
    }
    if let Some(value) = raw.tool {
        match value.as_str() {
            Some(id) if id.trim().is_empty() => {}
            Some(id) => match tool::by_id(id.trim()) {
                Some(tool) => config.tool = Some(tool),
                None => warnings.push(invalid_tool_warning(&value)),
            },
            None => warnings.push(invalid_tool_warning(&value)),
        }
    }
    (config, warnings)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    placement: Option<toml::Value>,
    direction: Option<toml::Value>,
    tool: Option<toml::Value>,
}

/// The exact refusal text for `placement = "popup"`: the review pane is a
/// workspace pane and `popup` is not among the placements the herdr CLI
/// documents for plugin panes.
const POPUP_REFUSED: &str =
    "placement \"popup\" is not supported for the review pane; using \"tab\"";

/// Warning for an invalid or non-string `placement` value: the value shown is
/// quoted for strings and generic for other TOML types.
fn invalid_placement_warning(value: &toml::Value) -> String {
    let shown = match value.as_str() {
        Some(s) => format!("\"{s}\""),
        None => "non-string value".to_string(),
    };
    format!(
        "invalid placement in config.toml: {shown}; \
         expected one of tab|split|overlay|zoomed, using default placement tab"
    )
}

/// Warning for an invalid or non-string `direction` value: the key is treated
/// as `auto` for that launch, so no `--direction` flag is passed.
fn invalid_direction_warning(value: &toml::Value) -> String {
    let shown = match value.as_str() {
        Some(s) => format!("\"{s}\""),
        None => "non-string value".to_string(),
    };
    format!(
        "invalid direction in config.toml: {shown}; \
         expected one of right|down|auto, treating it as auto"
    )
}

/// Warning for an invalid or non-string `tool` value: the key counts as unset
/// for that launch, routing it to the setup dialog.
fn invalid_tool_warning(value: &toml::Value) -> String {
    let shown = match value.as_str() {
        Some(s) => format!("\"{s}\""),
        None => "non-string value".to_string(),
    };
    format!(
        "invalid tool in config.toml: {shown}; \
         expected one of revdiff, tuicr, lumen; treating it as unset"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory removed when the test ends.
    struct TempDir(tempfile::TempDir);

    impl TempDir {
        fn new(tag: &str) -> Self {
            Self(
                tempfile::Builder::new()
                    .prefix(&format!("herdr-review-config-test-{tag}-"))
                    .tempdir()
                    .unwrap(),
            )
        }

        fn path(&self) -> &Path {
            self.0.path()
        }

        fn write_config(&self, content: &str) {
            std::fs::write(self.0.path().join("config.toml"), content).unwrap();
        }
    }

    #[test]
    fn environment_directory_wins() {
        // Scenario "Environment directory wins": the env value resolves the
        // directory and no config-dir CLI call is made (a panicking callback
        // would fail the test).
        let dir = TempDir::new("env-wins");
        dir.write_config("placement = \"overlay\"\n");
        let resolved = resolve_config_dir_with(Some(&dir.path().to_string_lossy()), &|| {
            panic!("the config-dir CLI call must not run when the env var is set")
        })
        .unwrap();
        let (config, warnings) = load_from_dir(&resolved);
        assert_eq!(config.placement, Placement::Overlay);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn cli_fallback_resolves_the_directory() {
        // Scenario "CLI fallback resolves the directory": with the env var
        // unset or blank, the CLI callback's directory is used.
        let dir = TempDir::new("cli-fallback");
        dir.write_config("placement = \"split\"\ndirection = \"down\"\n");
        for raw in [None, Some("   ")] {
            let resolved =
                resolve_config_dir_with(raw, &|| Ok(dir.path().to_string_lossy().into_owned()))
                    .unwrap();
            let (config, warnings) = load_from_dir(&resolved);
            assert_eq!(config.placement, Placement::Split);
            assert_eq!(config.direction, Direction::Down);
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    #[test]
    fn no_config_directory_available_warns_and_uses_defaults() {
        // Scenario "No config directory available warns and uses defaults":
        // the tool is unset, so the launch continues to the setup dialog.
        let (config, warnings) = load_with(Err("herdr did not provide HERDR_BIN_PATH".to_string()));
        assert_eq!(config, Config::default());
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{:?}", warnings);
        assert!(
            warnings[0].contains("config directory unavailable: herdr did not provide"),
            "{:?}",
            warnings
        );
        assert!(
            warnings[0].contains("using default settings"),
            "{:?}",
            warnings
        );
    }

    #[test]
    fn missing_file_uses_defaults_with_one_warning() {
        // Scenario "Missing file uses defaults": tab placement, right
        // direction, tool unset, and exactly one warning line.
        let dir = TempDir::new("missing-file");
        let (config, warnings) = load_from_dir(dir.path());
        assert_eq!(config, Config::default());
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("no config.toml"), "{warnings:?}");
    }

    #[test]
    fn corrupt_file_warns_and_uses_defaults() {
        // Scenario "Corrupt file warns and uses defaults": the warning names
        // the parse problem; the tool is unset, so the launch continues to
        // the setup dialog instead of failing.
        let dir = TempDir::new("corrupt");
        dir.write_config("placement = ");
        let (config, warnings) = load_from_dir(dir.path());
        assert_eq!(config, Config::default());
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("invalid config.toml"), "{warnings:?}");
        // A non-directory directory path also degrades without failing.
        let (config, warnings) = load_from_dir(&dir.path().join("gone"));
        assert_eq!(config, Config::default());
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn the_setup_dialogs_directory_resolution_is_silent() {
        // The setup dialog resolves the directory without reading the file
        // and without warnings: a non-blank env value wins outright (the
        // panicking callback proves it never runs), and a blank or absent
        // value falls through to the CLI callback whose result — or whose
        // failure, mapped to `None` — feeds the dialog's path line.
        let dir = TempDir::new("resolve-dir");
        assert_eq!(
            resolve_config_dir_with(Some(&dir.path().to_string_lossy()), &|| panic!(
                "the callback must not run when the env value is non-blank"
            ))
            .unwrap(),
            dir.path()
        );
        for raw in [None, Some("   ")] {
            assert_eq!(
                resolve_config_dir_with(raw, &|| Ok(dir.path().to_string_lossy().into_owned()))
                    .unwrap(),
                dir.path()
            );
        }
    }

    #[test]
    fn each_accepted_placement_maps_to_a_variant() {
        // Scenario "Each accepted placement maps to argv": every accepted
        // value parses to the variant whose `as_str` is the argv value.
        for (value, placement) in [
            ("tab", Placement::Tab),
            ("split", Placement::Split),
            ("overlay", Placement::Overlay),
            ("zoomed", Placement::Zoomed),
        ] {
            let (config, warnings) = parse_config(&format!("placement = \"{value}\""));
            assert_eq!(config.placement, placement, "value {value}");
            assert!(warnings.is_empty(), "{value}: {warnings:?}");
            assert_eq!(config.placement.as_str(), value);
        }
    }

    #[test]
    fn popup_falls_back_to_tab_with_a_warning() {
        // Scenario "Popup falls back to tab with a warning": the exact
        // refusal text, and the effective placement stays the tab default.
        let (config, warnings) = parse_config("placement = \"popup\"");
        assert_eq!(config.placement, Placement::Tab);
        assert_eq!(warnings, vec![POPUP_REFUSED.to_string()]);
        assert_eq!(
            warnings[0], "placement \"popup\" is not supported for the review pane; using \"tab\"",
            "{warnings:?}"
        );
    }

    #[test]
    fn unknown_placement_value_falls_back() {
        // Scenario "Unknown placement value falls back": a warning naming
        // `floating`, tab used.
        let (config, warnings) = parse_config("placement = \"floating\"");
        assert_eq!(config.placement, Placement::Tab);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"floating\""), "{warnings:?}");
        // A non-string value degrades the same way.
        let (config, warnings) = parse_config("placement = 42");
        assert_eq!(config.placement, Placement::Tab);
        assert!(warnings[0].contains("non-string value"), "{warnings:?}");
    }

    #[test]
    fn valid_tool_id_is_taken_as_configured() {
        // Scenario "Valid tool id is taken as configured": a catalog id
        // selects the tool the action then checks against PATH.
        for id in ["revdiff", "tuicr", "lumen"] {
            let (config, warnings) = parse_config(&format!("tool = \"{id}\""));
            assert_eq!(config.tool.map(tool::ReviewTool::id), Some(id), "{id}");
            assert!(warnings.is_empty(), "{id}: {warnings:?}");
        }
        // Whitespace around the id is trimmed like every other env-derived
        // value in the plugin.
        let (config, warnings) = parse_config("tool = \" tuicr \"");
        assert_eq!(config.tool.map(tool::ReviewTool::id), Some("tuicr"));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn invalid_tool_value_warns_and_counts_as_unset() {
        // Scenario "Invalid tool value warns and counts as unset": one
        // warning naming the value; the tool counts as unset, routing the
        // launch to the setup dialog.
        let (config, warnings) = parse_config("tool = \"diffy\"");
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"diffy\""), "{warnings:?}");
        assert!(warnings[0].contains("treating it as unset"), "{warnings:?}");
        // A non-string value degrades the same way.
        let (config, warnings) = parse_config("tool = 42");
        assert_eq!(config.tool, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("non-string value"), "{warnings:?}");
    }

    #[test]
    fn blank_tool_value_counts_as_unset() {
        // Scenario "Blank tool value counts as unset": silently, no warning.
        for content in ["tool = \"\"", "tool = \"   \"", "tool = \"\\t\""] {
            let (config, warnings) = parse_config(content);
            assert_eq!(config.tool, None, "{content}");
            assert!(warnings.is_empty(), "{content}: {warnings:?}");
        }
    }

    #[test]
    fn split_with_direction() {
        // Scenario "Split with direction".
        let (config, warnings) = parse_config("placement = \"split\"\ndirection = \"down\"\n");
        assert_eq!(config.placement, Placement::Split);
        assert_eq!(config.direction, Direction::Down);
        assert_eq!(config.direction.argv_value(), Some("down"));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn auto_omits_the_flag() {
        // Scenario "Auto omits the flag": `auto` maps to no argv value.
        let (config, warnings) = parse_config("placement = \"split\"\ndirection = \"auto\"\n");
        assert_eq!(config.direction, Direction::Auto);
        assert_eq!(config.direction.argv_value(), None);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn direction_ignored_without_split_produces_no_warning() {
        // Scenario "Direction ignored without split": a valid direction with
        // a non-split placement warns about nothing; omitting the flag is the
        // herdr layer's job, pinned by the pane-open argv tests.
        let (config, warnings) = parse_config("placement = \"tab\"\ndirection = \"down\"\n");
        assert_eq!(config.placement, Placement::Tab);
        assert_eq!(config.direction, Direction::Down);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn invalid_direction_warns_and_is_treated_as_auto() {
        // Scenario "Invalid direction warns and omits the flag": a warning
        // naming `left`, and the value maps to no argv value.
        let (config, warnings) = parse_config("placement = \"split\"\ndirection = \"left\"\n");
        assert_eq!(config.placement, Placement::Split);
        assert_eq!(config.direction, Direction::Auto);
        assert_eq!(config.direction.argv_value(), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"left\""), "{warnings:?}");
        assert!(warnings[0].contains("treating it as auto"), "{warnings:?}");
    }

    #[test]
    fn unknown_keys_are_ignored_without_warning() {
        let (config, warnings) = parse_config("answer = 42\nplacement = \"zoomed\"\n");
        assert_eq!(config.placement, Placement::Zoomed);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn empty_file_is_silent_defaults() {
        let (config, warnings) = parse_config("");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn defaults_are_tab_right_and_no_tool() {
        assert_eq!(
            Config::default(),
            Config {
                placement: Placement::Tab,
                direction: Direction::Right,
                tool: None,
            }
        );
    }
}
