//! herdr theme palette resolution for the setup dialog's frame renderer.
//!
//! Before the dialog draws its first frame it resolves a [`Palette`] from the
//! herdr config: `[theme] name` is read from `$HERDR_CONFIG_PATH` when set,
//! else from `~/.config/herdr/config.toml`, and parsed with the `toml` crate.
//! A name matching one of herdr's 18 built-in themes selects that theme's
//! embedded truecolor palette; `terminal`, a missing or unreadable config,
//! invalid TOML, or an unknown name select the ANSI-16 fallback palette. The
//! palette is resolved once per dialog run and never re-read while the dialog
//! is open. `[theme.custom]` overrides and `auto_switch` are out of scope and
//! ignored.
//!
//! The built-in table below carries only the four tokens the dialog paints
//! (`accent`, `panel_bg`, `text`, `overlay1`), transcribed from herdr
//! [`THEME_SOURCE_VERSION`]'s `src/app/state.rs` (`struct Palette` and its
//! per-theme constructors) against the name list in that version's
//! `src/config/theme.rs` (`THEME_NAMES`). Ported from the sibling
//! herdr-open-in plugin's `src/theme.rs`, the source of truth.

use std::path::{Path, PathBuf};

/// herdr version the embedded [`BUILTIN_PALETTES`] table was transcribed from:
/// sourced from herdr 0.9.5, `src/app/state.rs` (tag `v0.9.5`).
/// Documentation-only: no production path consumes the version string, so it
/// stays visible to tests and readers without tripping `dead_code` in the bin.
#[allow(dead_code)]
pub(crate) const THEME_SOURCE_VERSION: &str = "0.9.5";

/// herdr's default theme name (Catppuccin Mocha), also the fallback palette.
const TERMINAL_THEME_NAME: &str = "terminal";

/// Environment variable pointing at the herdr config file; when unset the
/// dialog reads `~/.config/herdr/config.toml`.
const CONFIG_PATH_ENV: &str = "HERDR_CONFIG_PATH";

/// One color token of a [`Palette`], in the form the renderer needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenColor {
    /// Truecolor; emits `38;2;r;g;b` / `48;2;r;g;b` sequences.
    Rgb(u8, u8, u8),
    /// ANSI-16 color given by its foreground SGR code (`30..=37` base,
    /// `90..=97` bright); the background code is the foreground code + 10.
    Ansi(u8),
    /// Emit nothing; the terminal's own colors show through.
    Default,
}

/// The four tokens the dialog paints, resolved once before the first frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Selected-row background and marker color.
    pub(crate) accent: TokenColor,
    /// Panel and row background.
    pub(crate) panel_bg: TokenColor,
    /// Unselected row text.
    pub(crate) text: TokenColor,
    /// Muted help-footer text.
    pub(crate) overlay1: TokenColor,
}

/// All 18 herdr built-in theme names with its tokens, transcribed
/// from herdr [`THEME_SOURCE_VERSION`]'s `src/app/state.rs`; each entry cites
/// the constructor it was sourced from. The `"terminal"` entry is faithful to
/// that version but unreachable through [`resolve_palette`]: the spec makes
/// the terminal fallback check take precedence over this table.
pub(crate) const BUILTIN_PALETTES: &[(&str, Palette); 18] = &[
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::catppuccin
    // (#89b4fa / #181825 / #cdd6f4 / #7f849c)
    (
        "catppuccin",
        Palette {
            accent: TokenColor::Rgb(137, 180, 250),
            panel_bg: TokenColor::Rgb(24, 24, 37),
            text: TokenColor::Rgb(205, 214, 244),
            overlay1: TokenColor::Rgb(127, 132, 156),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::catppuccin_latte
    // (#1e66f5 / #eff1f5 / #4c4f69 / #8c8fa1)
    (
        "catppuccin-latte",
        Palette {
            accent: TokenColor::Rgb(30, 102, 245),
            panel_bg: TokenColor::Rgb(239, 241, 245),
            text: TokenColor::Rgb(76, 79, 105),
            overlay1: TokenColor::Rgb(140, 143, 161),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::terminal
    // (Color::Blue / Reset / Reset / White — the 16-color theme); the
    // resolver's terminal precedence check means this entry is never chosen.
    (
        "terminal",
        Palette {
            accent: TokenColor::Ansi(34),
            panel_bg: TokenColor::Default,
            text: TokenColor::Default,
            overlay1: TokenColor::Ansi(97),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::tokyo_night
    // (#7aa2f7 / #1a1b26 / #c0caf5 / #697196)
    (
        "tokyo-night",
        Palette {
            accent: TokenColor::Rgb(122, 162, 247),
            panel_bg: TokenColor::Rgb(26, 27, 38),
            text: TokenColor::Rgb(192, 202, 245),
            overlay1: TokenColor::Rgb(105, 113, 150),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::tokyo_night_day
    // (#2e7de9 / #e1e2e7 / #3760bf / #68709a)
    (
        "tokyo-night-day",
        Palette {
            accent: TokenColor::Rgb(46, 125, 233),
            panel_bg: TokenColor::Rgb(225, 226, 231),
            text: TokenColor::Rgb(55, 96, 191),
            overlay1: TokenColor::Rgb(104, 112, 154),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::dracula
    // (#bd93f9 / #282a36 / #f8f8f2 / #828cb4)
    (
        "dracula",
        Palette {
            accent: TokenColor::Rgb(189, 147, 249),
            panel_bg: TokenColor::Rgb(40, 42, 54),
            text: TokenColor::Rgb(248, 248, 242),
            overlay1: TokenColor::Rgb(130, 140, 180),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::nord
    // (#88c0d0 / #2e3440 / #eceff4 / #646e82)
    (
        "nord",
        Palette {
            accent: TokenColor::Rgb(136, 192, 208),
            panel_bg: TokenColor::Rgb(46, 52, 64),
            text: TokenColor::Rgb(236, 239, 244),
            overlay1: TokenColor::Rgb(100, 110, 130),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::gruvbox
    // (#d79921 / #282828 / #ebdbb2 / #a89984)
    (
        "gruvbox",
        Palette {
            accent: TokenColor::Rgb(215, 153, 33),
            panel_bg: TokenColor::Rgb(40, 40, 40),
            text: TokenColor::Rgb(235, 219, 178),
            overlay1: TokenColor::Rgb(168, 153, 132),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::gruvbox_light
    // (#076678 / #fbf1c7 / #3c3836 / #7c6f64)
    (
        "gruvbox-light",
        Palette {
            accent: TokenColor::Rgb(7, 102, 120),
            panel_bg: TokenColor::Rgb(251, 241, 199),
            text: TokenColor::Rgb(60, 56, 54),
            overlay1: TokenColor::Rgb(124, 111, 100),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::one_dark
    // (#61afef / #282c34 / #abb2bf / #737a87)
    (
        "one-dark",
        Palette {
            accent: TokenColor::Rgb(97, 175, 239),
            panel_bg: TokenColor::Rgb(40, 44, 52),
            text: TokenColor::Rgb(171, 178, 191),
            overlay1: TokenColor::Rgb(115, 122, 135),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::one_light
    // (#4078f2 / #fafafa / #383a42 / #686b77)
    (
        "one-light",
        Palette {
            accent: TokenColor::Rgb(64, 120, 242),
            panel_bg: TokenColor::Rgb(250, 250, 250),
            text: TokenColor::Rgb(56, 58, 66),
            overlay1: TokenColor::Rgb(104, 107, 119),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::solarized
    // (#268bd2 / #002b36 / #93a1a1 / #657b83)
    (
        "solarized",
        Palette {
            accent: TokenColor::Rgb(38, 139, 210),
            panel_bg: TokenColor::Rgb(0, 43, 54),
            text: TokenColor::Rgb(147, 161, 161),
            overlay1: TokenColor::Rgb(101, 123, 131),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::solarized_light
    // (#268bd2 / #fdf6e3 / #657b83 / #586e75)
    (
        "solarized-light",
        Palette {
            accent: TokenColor::Rgb(38, 139, 210),
            panel_bg: TokenColor::Rgb(253, 246, 227),
            text: TokenColor::Rgb(101, 123, 131),
            overlay1: TokenColor::Rgb(88, 110, 117),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::kanagawa
    // (#7e9cd8 / #1f1f28 / #dcd7ba / #87867d)
    (
        "kanagawa",
        Palette {
            accent: TokenColor::Rgb(126, 156, 216),
            panel_bg: TokenColor::Rgb(31, 31, 40),
            text: TokenColor::Rgb(220, 215, 186),
            overlay1: TokenColor::Rgb(135, 134, 125),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::kanagawa_lotus
    // (#4d699b / #f2ecbc / #545464 / #8a8980)
    (
        "kanagawa-lotus",
        Palette {
            accent: TokenColor::Rgb(77, 105, 155),
            panel_bg: TokenColor::Rgb(242, 236, 188),
            text: TokenColor::Rgb(84, 84, 100),
            overlay1: TokenColor::Rgb(138, 137, 128),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::rose_pine
    // (#c4a7e7 / #191724 / #e0def4 / #908caa)
    (
        "rose-pine",
        Palette {
            accent: TokenColor::Rgb(196, 167, 231),
            panel_bg: TokenColor::Rgb(25, 23, 36),
            text: TokenColor::Rgb(224, 222, 244),
            overlay1: TokenColor::Rgb(144, 140, 170),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::rose_pine_dawn
    // (#907aa9 / #faf4ed / #464261 / #797593)
    (
        "rose-pine-dawn",
        Palette {
            accent: TokenColor::Rgb(144, 122, 169),
            panel_bg: TokenColor::Rgb(250, 244, 237),
            text: TokenColor::Rgb(70, 66, 97),
            overlay1: TokenColor::Rgb(121, 117, 147),
        },
    ),
    // sourced from herdr 0.9.5 src/app/state.rs: Palette::vesper
    // (#ffc799 / #1a1a1a / #ffffff / #7e7e7e)
    (
        "vesper",
        Palette {
            accent: TokenColor::Rgb(255, 199, 153),
            panel_bg: TokenColor::Rgb(26, 26, 26),
            text: TokenColor::Rgb(255, 255, 255),
            overlay1: TokenColor::Rgb(126, 126, 126),
        },
    ),
];

/// The ANSI-16 palette used for `terminal`, missing/unreadable configs,
/// invalid TOML, and unknown names: `accent` is ANSI blue, `overlay1` is
/// ANSI bright black (dark gray), and `text`/`panel_bg` use terminal defaults.
pub(crate) fn fallback_palette() -> Palette {
    Palette {
        accent: TokenColor::Ansi(34),
        panel_bg: TokenColor::Default,
        text: TokenColor::Default,
        overlay1: TokenColor::Ansi(90),
    }
}

/// Resolves the palette from the environment: `$HERDR_CONFIG_PATH`
/// when set and non-blank, else `~/.config/herdr/config.toml`. The dialog
/// calls this exactly once, before its first frame.
pub(crate) fn resolve_palette() -> Palette {
    palette_at(env_path(CONFIG_PATH_ENV).as_deref(), home_dir().as_deref())
}

/// Chooses the config path: an explicit `$HERDR_CONFIG_PATH`-style path wins
/// over the `<home>/.config/herdr/config.toml` default; no path at all is the
/// fallback. Split from [`resolve_palette`] so tests drive it directly instead
/// of mutating the process-global environment.
fn palette_at(config_path: Option<&Path>, home: Option<&Path>) -> Palette {
    let path = match config_path {
        Some(path) => path.to_path_buf(),
        None => match home {
            Some(home) => home.join(".config/herdr/config.toml"),
            None => return fallback_palette(),
        },
    };
    palette_from_file(&path)
}

/// Reads and parses one config file; any failure selects the fallback.
fn palette_from_file(path: &Path) -> Palette {
    let Ok(text) = std::fs::read_to_string(path) else {
        return fallback_palette();
    };
    let Some(name) = theme_name_from_toml(&text) else {
        return fallback_palette();
    };
    // The fallback check takes precedence over the built-in table, even
    // though "terminal" is one of the 18 built-in names (verified against
    // herdr 0.9.5 src/config/theme.rs THEME_NAMES).
    if name == TERMINAL_THEME_NAME {
        return fallback_palette();
    }
    BUILTIN_PALETTES
        .iter()
        .find(|(builtin, _)| *builtin == name)
        .map_or_else(fallback_palette, |(_, palette)| *palette)
}

/// Extracts `[theme] name` from TOML text; absent, non-string, or unparseable
/// all yield `None`. Parsed as a plain [`toml::Value`] and indexed — no serde
/// structs needed for one string, and `[theme.custom]` is never consulted.
fn theme_name_from_toml(text: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(text).ok()?;
    value
        .get("theme")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// Reads an environment variable as a path; unset, blank, and non-UTF-8
/// values are absent.
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The user's home directory, from `HOME` (Unix) or `USERPROFILE` (Windows);
/// `None` when neither is set.
fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|name| env_path(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp file that deletes itself, with a unique name per test.
    struct TempFile(PathBuf);

    impl TempFile {
        fn with_content(tag: &str, content: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "herdr-review-theme-test-{}-{tag}-{id}.toml",
                std::process::id()
            ));
            std::fs::write(&path, content).expect("temp fixture written");
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A temp directory tree that deletes itself recursively.
    struct TempDir {
        path: PathBuf,
        _temp: tempfile::TempDir,
    }

    impl TempDir {
        /// Creates `<tmp>/herdr-review-theme-test-<pid>-<tag>` containing a
        /// `.config/herdr/config.toml` with `content`.
        fn with_herdr_config(tag: &str, content: &str) -> Self {
            let temp = tempfile::Builder::new()
                .prefix(&format!("herdr-review-theme-test-{tag}-"))
                .tempdir()
                .unwrap();
            let dir = temp.path().to_path_buf();
            let config_dir = dir.join(".config/herdr");
            std::fs::create_dir_all(&config_dir).expect("temp home created");
            std::fs::write(config_dir.join("config.toml"), content).expect("temp config written");
            Self {
                path: dir,
                _temp: temp,
            }
        }
    }

    fn palette_of(name: &str) -> Palette {
        BUILTIN_PALETTES
            .iter()
            .find(|(builtin, _)| *builtin == name)
            .unwrap_or_else(|| panic!("{name} is not in the built-in table"))
            .1
    }

    #[test]
    fn built_in_table_carries_the_18_verified_names() {
        // The name list verified against herdr 0.9.5 src/config/theme.rs
        // THEME_NAMES, sorted.
        let mut names: Vec<&str> = BUILTIN_PALETTES.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "catppuccin",
                "catppuccin-latte",
                "dracula",
                "gruvbox",
                "gruvbox-light",
                "kanagawa",
                "kanagawa-lotus",
                "nord",
                "one-dark",
                "one-light",
                "rose-pine",
                "rose-pine-dawn",
                "solarized",
                "solarized-light",
                "terminal",
                "tokyo-night",
                "tokyo-night-day",
                "vesper",
            ]
        );
    }

    #[test]
    fn built_in_theme_honored() {
        // A real-world config shape: other tables and keys around [theme].
        let config = TempFile::with_content(
            "builtin",
            "\
keybinds = []

[theme]
name = \"rose-pine-dawn\"
auto_switch = false

[editor]
mouse = true
",
        );
        let palette = palette_at(Some(&config.0), None);
        assert_eq!(palette, palette_of("rose-pine-dawn"));
        assert_eq!(palette.accent, TokenColor::Rgb(144, 122, 169));
    }

    #[test]
    fn herdr_config_path_override_wins_over_the_home_default() {
        let override_file = TempFile::with_content("override", "[theme]\nname = \"tokyo-night\"\n");
        let home = TempDir::with_herdr_config("other", "[theme]\nname = \"vesper\"\n");
        // The explicit path takes precedence over the home default...
        let palette = palette_at(Some(&override_file.0), Some(&home.path));
        assert_eq!(palette, palette_of("tokyo-night"));
        // ...while without it the home default is read.
        let palette = palette_at(None, Some(&home.path));
        assert_eq!(palette, palette_of("vesper"));
    }

    #[test]
    fn resolve_palette_reads_the_herdr_config_path_env() {
        // Process-global env mutation stays serialized: no other test in this
        // module touches the environment, only this one, under this lock.
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = TempFile::with_content("env", "[theme]\nname = \"nord\"\n");
        // SAFETY: the only concurrent env readers are tests in this binary,
        // and none of them read HERDR_CONFIG_PATH or HOME.
        unsafe { std::env::set_var(CONFIG_PATH_ENV, &config.0) };
        let palette = resolve_palette();
        unsafe { std::env::remove_var(CONFIG_PATH_ENV) };
        assert_eq!(palette, palette_of("nord"));
    }

    #[test]
    fn missing_config_file_falls_back() {
        let missing = std::env::temp_dir().join("herdr-review-theme-test-does-not-exist.toml");
        assert!(!missing.exists());
        assert_eq!(palette_at(Some(&missing), None), fallback_palette());
        // No path and no home is the fallback as well.
        assert_eq!(palette_at(None, None), fallback_palette());
    }

    #[test]
    fn invalid_toml_falls_back() {
        let config = TempFile::with_content("invalid", "[theme\nname = unclosed");
        assert_eq!(palette_at(Some(&config.0), None), fallback_palette());
    }

    #[test]
    fn theme_table_or_name_absent_falls_back() {
        let no_theme = TempFile::with_content("notheme", "[editor]\nmouse = true\n");
        assert_eq!(palette_at(Some(&no_theme.0), None), fallback_palette());
        let no_name = TempFile::with_content("noname", "[theme]\nauto_switch = true\n");
        assert_eq!(palette_at(Some(&no_name.0), None), fallback_palette());
        // A non-string name is the same failure.
        let wrong_type = TempFile::with_content("wrongtype", "[theme]\nname = 3\n");
        assert_eq!(palette_at(Some(&wrong_type.0), None), fallback_palette());
    }

    #[test]
    fn unknown_name_falls_back() {
        let config = TempFile::with_content("unknown", "[theme]\nname = \"catppucin\"\n");
        assert_eq!(palette_at(Some(&config.0), None), fallback_palette());
    }

    #[test]
    fn terminal_name_falls_back_even_though_it_is_a_built_in() {
        // The precedence clause: "terminal" is verified to be among the 18
        // built-in names, yet it must select the ANSI-16 fallback.
        assert!(
            BUILTIN_PALETTES
                .iter()
                .any(|(name, _)| *name == TERMINAL_THEME_NAME)
        );
        let config = TempFile::with_content("terminal", "[theme]\nname = \"terminal\"\n");
        assert_eq!(palette_at(Some(&config.0), None), fallback_palette());
    }

    #[test]
    fn fallback_palette_uses_the_pinned_ansi_16_tokens() {
        let palette = fallback_palette();
        assert_eq!(palette.accent, TokenColor::Ansi(34));
        assert_eq!(palette.overlay1, TokenColor::Ansi(90));
        assert_eq!(palette.text, TokenColor::Default);
        assert_eq!(palette.panel_bg, TokenColor::Default);
        assert_eq!(palette.accent, TokenColor::Ansi(34));
        assert_eq!(palette.overlay1, TokenColor::Ansi(90));
        assert_eq!(palette.text, TokenColor::Default);
        assert_eq!(palette.panel_bg, TokenColor::Default);
    }
}
