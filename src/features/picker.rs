//! Borderless popup for choosing between installed review tools.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::adapters::{herdr, tool::ReviewTool};
use crate::settings::{Config, Direction, Placement};

const TOOL_IDS_ENV: &str = "HERDR_REVIEWS_PICKER_TOOL_IDS";
const STATE_DIR_ENV: &str = "HERDR_PLUGIN_STATE_DIR";
const CHOICE_FILE: &str = "last-review-tool";
const FOOTER: &str = "↑/↓ move   enter select   esc cancel";

struct Picker {
    tools: Vec<ReviewTool>,
    selected: usize,
}

pub(crate) fn run() -> Result<(), String> {
    let tools = parse_tools(&std::env::var(TOOL_IDS_ENV).unwrap_or_default())?;
    if tools.len() < 2 {
        return Err(format!(
            "{TOOL_IDS_ENV} must contain at least two installed tools"
        ));
    }
    let mut picker = Picker {
        selected: restore_choice(&tools).unwrap_or(0),
        tools,
    };
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("herdr-review: tool selection unavailable without an interactive terminal");
        return Ok(());
    }

    let mut terminal = ratatui::try_init()
        .map_err(|error| format!("cannot initialize tool picker terminal: {error}"))?;
    let result = (|| -> Result<Option<ReviewTool>, String> {
        loop {
            terminal
                .draw(|frame| picker.render(frame))
                .map_err(|error| format!("cannot draw tool picker: {error}"))?;
            if !event::poll(Duration::from_millis(100))
                .map_err(|error| format!("cannot read tool picker input: {error}"))?
            {
                continue;
            }
            let Event::Key(key) =
                event::read().map_err(|error| format!("cannot read tool picker input: {error}"))?
            else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up => {
                    picker.selected = if picker.selected == 0 {
                        picker.tools.len() - 1
                    } else {
                        picker.selected - 1
                    }
                }
                KeyCode::Down => picker.selected = (picker.selected + 1) % picker.tools.len(),
                KeyCode::Enter => return Ok(Some(picker.tools[picker.selected])),
                KeyCode::Esc => return Ok(None),
                _ => {}
            }
        }
    })();
    let restored = ratatui::try_restore()
        .map_err(|error| format!("cannot restore tool picker terminal: {error}"));
    let chosen = match (result, restored) {
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Ok(chosen), Ok(())) => chosen,
    };
    let Some(tool) = chosen else {
        return Ok(());
    };
    save_choice_then(&choice_path(), tool, || launch(tool))
}

fn parse_tools(raw: &str) -> Result<Vec<ReviewTool>, String> {
    let mut tools = Vec::new();
    for id in raw.split(',').map(str::trim).filter(|id| !id.is_empty()) {
        let tool = ReviewTool::from_id(id)
            .ok_or_else(|| format!("unknown installed review tool id: {id}"))?;
        if !tools.contains(&tool) {
            tools.push(tool);
        }
    }
    if tools.is_empty() {
        return Err(format!("{TOOL_IDS_ENV} is missing or empty"));
    }
    Ok(tools)
}

impl Picker {
    fn render(&self, frame: &mut Frame<'_>) {
        let [list, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        let lines = self
            .tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                let marker = if index == self.selected { "› " } else { "  " };
                let style = if index == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let row = format!(
                    "{marker}{:<width$}",
                    tool.id(),
                    width = frame.area().width.saturating_sub(2) as usize
                );
                Line::from(Span::styled(row, style))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                FOOTER,
                Style::default().add_modifier(Modifier::DIM),
            ))),
            footer,
        );
    }
}

fn state_dir() -> PathBuf {
    std::env::var(STATE_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn choice_path() -> PathBuf {
    state_dir().join(CHOICE_FILE)
}

fn restore_choice(tools: &[ReviewTool]) -> Option<usize> {
    restore_choice_from(&choice_path(), tools)
}

fn restore_choice_from(path: &std::path::Path, tools: &[ReviewTool]) -> Option<usize> {
    let id = std::fs::read_to_string(path).ok()?;
    tools.iter().position(|tool| tool.id() == id.trim())
}

fn save_choice_to(path: &std::path::Path, tool: ReviewTool) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_ok() {
        let _ = std::fs::write(path, tool.id());
    }
}

fn save_choice_then<T>(path: &std::path::Path, tool: ReviewTool, launch: impl FnOnce() -> T) -> T {
    save_choice_to(path, tool);
    launch()
}

fn launch(tool: ReviewTool) -> Result<(), String> {
    let dir = env_non_blank("HERDR_REVIEWS_DIR").ok_or("HERDR_REVIEWS_DIR is missing")?;
    let mut config = Config {
        placement: match env_non_blank("HERDR_REVIEWS_PLACEMENT").as_deref() {
            Some("split") => Placement::Split,
            Some("overlay") => Placement::Overlay,
            Some("zoomed") => Placement::Zoomed,
            Some("tab") | None => Placement::Tab,
            Some(other) => return Err(format!("invalid review placement: {other}")),
        },
        direction: match env_non_blank("HERDR_REVIEWS_DIRECTION").as_deref() {
            Some("down") => Direction::Down,
            Some("auto") => Direction::Auto,
            Some("right") | None => Direction::Right,
            Some(other) => return Err(format!("invalid review direction: {other}")),
        },
    };
    let workspace = env_non_blank("HERDR_REVIEWS_WORKSPACE");
    let caller_pane = env_non_blank("HERDR_REVIEWS_CALLER_PANE");
    let caller_agent = env_non_blank("HERDR_REVIEWS_CALLER_AGENT");
    let caller_tab = env_non_blank("HERDR_REVIEWS_CALLER_TAB");
    if matches!(config.placement, Placement::Split | Placement::Zoomed) && caller_pane.is_none() {
        eprintln!(
            "herdr-review: warning: {} placement requires a caller pane; using tab",
            config.placement.as_str()
        );
        config.placement = Placement::Tab;
    }
    let pane = herdr::open_review_pane(
        &config,
        workspace.as_deref(),
        &dir,
        tool.id(),
        caller_pane.as_deref(),
        caller_agent.as_deref(),
        caller_tab.as_deref(),
    )?;
    if let Some(pane) = pane {
        eprintln!(
            "herdr-review: opened {} review pane {}",
            tool.id(),
            pane.pane_id
        );
        if config.placement == Placement::Tab
            && let Some(tab_id) = pane.tab_id.as_deref()
            && let Err(error) = herdr::schedule_picker_focus(tab_id)
        {
            eprintln!("herdr-review: warning: cannot schedule tab focus after picker: {error}");
        }
    }
    Ok(())
}

/// Runs in a detached helper after the picker selected a tool. It closes the
/// modal popup first, then reasserts focus on the newly opened tab.
pub(crate) fn focus_after_popup() -> Result<(), String> {
    let tab_id = env_non_blank("HERDR_REVIEWS_PICKER_FOCUS_TAB")
        .ok_or("HERDR_REVIEWS_PICKER_FOCUS_TAB is missing")?;
    close_popup_then_focus(&tab_id, herdr::close_popup, herdr::tab_focus)
}

fn close_popup_then_focus(
    tab_id: &str,
    mut close_popup: impl FnMut() -> Result<(), String>,
    focus_tab: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut close_error = None;
    for _ in 0..20 {
        match close_popup() {
            Ok(()) => {
                close_error = None;
                break;
            }
            Err(error) => {
                close_error = Some(error);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    if let Some(error) = close_error {
        eprintln!("herdr-review: warning: cannot close picker popup before focusing tab: {error}");
    }
    focus_tab(tab_id)
}

fn env_non_blank(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const INSTALLED: [ReviewTool; 2] = [ReviewTool::Lumen, ReviewTool::Revdiff];

    #[test]
    fn selected_row_highlight_fills_the_popup_width() {
        let backend = TestBackend::new(36, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        let picker = Picker {
            tools: INSTALLED.to_vec(),
            selected: 1,
        };

        terminal.draw(|frame| picker.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();

        for x in 0..36 {
            assert!(
                buffer[(x, 1)]
                    .style()
                    .add_modifier
                    .contains(Modifier::REVERSED),
                "selected row cell {x} is not highlighted"
            );
            assert!(
                !buffer[(x, 0)]
                    .style()
                    .add_modifier
                    .contains(Modifier::REVERSED),
                "unselected row cell {x} is highlighted"
            );
        }
        assert!(buffer[(0, 1)].symbol().starts_with('›'));
    }

    #[test]
    fn footer_fits_the_declared_picker_width() {
        assert_eq!(FOOTER.chars().count(), 36);
        let backend = TestBackend::new(36, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        let picker = Picker {
            tools: INSTALLED.to_vec(),
            selected: 0,
        };

        terminal.draw(|frame| picker.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let footer = (0..36).map(|x| buffer[(x, 6)].symbol()).collect::<String>();

        assert!(footer.starts_with(FOOTER));
    }

    #[test]
    fn restores_last_choice_only_when_it_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHOICE_FILE);

        std::fs::write(&path, "revdiff").unwrap();
        assert_eq!(restore_choice_from(&path, &INSTALLED), Some(1));

        std::fs::write(&path, "tuicr").unwrap();
        assert_eq!(restore_choice_from(&path, &INSTALLED), None);
    }

    #[test]
    fn missing_unreadable_and_unknown_choices_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing").join(CHOICE_FILE);
        assert_eq!(restore_choice_from(&missing, &INSTALLED), None);

        let directory = dir.path().join("choice-directory");
        std::fs::create_dir(&directory).unwrap();
        assert_eq!(restore_choice_from(&directory, &INSTALLED), None);

        let unknown = dir.path().join("unknown");
        std::fs::write(&unknown, "not-a-tool").unwrap();
        assert_eq!(restore_choice_from(&unknown, &INSTALLED), None);
    }

    #[test]
    fn state_write_failure_does_not_skip_launch() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("state-dir-is-a-file");
        std::fs::write(&blocker, "not a directory").unwrap();
        let choice_path = blocker.join(CHOICE_FILE);
        let mut launched = false;

        save_choice_then(&choice_path, ReviewTool::Revdiff, || {
            launched = true;
        });

        assert!(launched);
        assert!(!choice_path.exists());
    }

    #[test]
    fn focus_is_reasserted_only_after_popup_close() {
        let calls = std::cell::RefCell::new(Vec::new());

        close_popup_then_focus(
            "workspace:tab-2",
            || {
                calls.borrow_mut().push("popup.close");
                Ok(())
            },
            |tab_id| {
                assert_eq!(tab_id, "workspace:tab-2");
                calls.borrow_mut().push("tab.focus");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["popup.close", "tab.focus"]);
    }
}
