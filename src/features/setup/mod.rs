//! Read-only setup dialog shown when no review tool is available.

use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::adapters::tool::ReviewTool;

pub(crate) mod theme;

const CONFIG_FILE_LINE: &str = "Config file:";
const ADD_TOOLS_LINE: &str = "Add one of these tools to config:";
const MORE_INFO_LINE: &str = "For more information:";
const REPO_URL: &str = "https://github.com/alexeyco/herdr-review";
const CONFIG_DIR_HINT: &str = "herdr plugin config-dir alexeyco.review";
const FOOTER: &str = "enter/q/esc close";

pub(crate) struct SetupDialog {
    config_line: String,
    palette: theme::Palette,
}

impl SetupDialog {
    fn new(config_dir: Option<&Path>) -> Self {
        Self {
            config_line: config_dir
                .map(|dir| dir.join("config.toml").display().to_string())
                .unwrap_or_else(|| CONFIG_DIR_HINT.to_string()),
            palette: theme::resolve_palette(),
        }
    }

    pub(crate) fn run() -> Result<(), String> {
        let dialog = Self::new(crate::config::resolve_dir().as_deref());
        if !std::io::stdin().is_terminal() {
            return dialog.run_plain();
        }
        let mut terminal = ratatui::try_init()
            .map_err(|error| format!("cannot initialize setup terminal: {error}"))?;
        let outcome = (|| -> Result<(), String> {
            loop {
                terminal
                    .draw(|frame| dialog.render(frame))
                    .map_err(|error| format!("cannot draw the setup dialog: {error}"))?;
                if event::poll(Duration::from_millis(100))
                    .map_err(|error| format!("cannot read the terminal: {error}"))?
                    && is_key_event(
                        &event::read()
                            .map_err(|error| format!("cannot read the terminal: {error}"))?,
                    )
                {
                    break Ok(());
                }
            }
        })();
        let restored = ratatui::try_restore()
            .map_err(|error| format!("cannot restore the setup terminal: {error}"));
        match (outcome, restored) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn run_plain(&self) -> Result<(), String> {
        let mut output = std::io::stdout().lock();
        writeln!(output, "  {CONFIG_FILE_LINE}\n  {}", self.config_line)
            .and_then(|()| writeln!(output, "\n  {ADD_TOOLS_LINE}"))
            .and_then(|()| {
                for tool in ReviewTool::ALL {
                    writeln!(output, "  {:<7}  {}", tool.id(), tool.install_hint())?;
                }
                writeln!(output, "\n  {MORE_INFO_LINE}\n  {REPO_URL}\n\n  {FOOTER}")
            })
            .map_err(|error| format!("cannot draw the setup dialog: {error}"))?;
        let mut key = [0; 1];
        let _ = std::io::stdin()
            .lock()
            .read(&mut key)
            .map_err(|error| format!("cannot read the terminal: {error}"))?;
        Ok(())
    }

    fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette;
        let base = Style::default()
            .fg(to_color(palette.text))
            .bg(to_color(palette.panel_bg));
        frame.render_widget(Block::default().style(base), area);

        let [content, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        frame.render_widget(
            Paragraph::new(self.lines())
                .style(base)
                .wrap(Wrap { trim: false }),
            content,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    FOOTER,
                    Style::default()
                        .fg(to_color(palette.overlay1))
                        .bg(to_color(palette.panel_bg)),
                ),
            ]))
            .style(base),
            footer,
        );
    }

    fn lines(&self) -> Vec<Line<'_>> {
        let mut lines = vec![
            Line::from(""),
            line(CONFIG_FILE_LINE),
            line(&self.config_line),
            Line::from(""),
        ];
        lines.push(line(ADD_TOOLS_LINE));
        let name_width = ReviewTool::ALL
            .iter()
            .map(|tool| tool.id().len())
            .max()
            .unwrap_or_default();
        for tool in ReviewTool::ALL {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(format!("{:<name_width$}  ", tool.id())),
                Span::styled(
                    tool.install_hint(),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]));
        }
        lines.extend([
            Line::from(""),
            line(MORE_INFO_LINE),
            line(REPO_URL),
            Line::from(""),
        ]);
        lines
    }
}

fn line(text: &str) -> Line<'_> {
    Line::from(vec![Span::raw("  "), Span::raw(text)])
}

fn to_color(color: theme::TokenColor) -> Color {
    match color {
        theme::TokenColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
        theme::TokenColor::Ansi(code) => match code {
            30 => Color::Black,
            31 => Color::Red,
            32 => Color::Green,
            33 => Color::Yellow,
            34 => Color::Blue,
            35 => Color::Magenta,
            36 => Color::Cyan,
            37 => Color::Gray,
            90 => Color::DarkGray,
            91 => Color::LightRed,
            92 => Color::LightGreen,
            93 => Color::LightYellow,
            94 => Color::LightBlue,
            95 => Color::LightMagenta,
            96 => Color::LightCyan,
            97 => Color::White,
            _ => Color::Reset,
        },
        theme::TokenColor::Default => Color::Reset,
    }
}

fn is_key_event(event: &Event) -> bool {
    matches!(event, Event::Key(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn dialog() -> SetupDialog {
        let mut dialog = SetupDialog::new(Some(Path::new("/cfg/review")));
        dialog.palette = theme::Palette {
            accent: theme::TokenColor::Rgb(137, 180, 250),
            panel_bg: theme::TokenColor::Rgb(24, 24, 37),
            text: theme::TokenColor::Rgb(205, 214, 244),
            overlay1: theme::TokenColor::Rgb(127, 132, 156),
        };
        dialog
    }

    fn draw(width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| dialog().render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn dialog_shows_config_tools_and_repository() {
        let screen = draw(80, 15);
        for expected in [
            "Config file:",
            "/cfg/review/config.toml",
            "Add one of these tools to config:",
            "revdiff",
            "tuicr",
            "lumen",
            "https://github.com/alexeyco/herdr-review",
            FOOTER,
        ] {
            assert!(
                screen.contains(expected),
                "missing {expected:?} in {screen:?}"
            );
        }
    }

    #[test]
    fn dialog_draws_footer_at_bottom_and_handles_small_terminal() {
        let screen = draw(80, 15);
        assert!(screen.rfind(FOOTER).unwrap() > screen.rfind("For more information:").unwrap());
        assert!(!draw(20, 6).is_empty());
    }

    #[test]
    fn any_key_event_closes_the_dialog() {
        assert!(is_key_event(&Event::Key(
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('x'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            )
        )));
        assert!(!is_key_event(&Event::Resize(80, 24)));
    }
}
