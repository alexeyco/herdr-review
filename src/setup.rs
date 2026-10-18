//! The read-only review-tool setup dialog: a tiny raw-mode popup TUI.
//!
//! The review action opens this dialog when no usable tool is configured —
//! the `tool` key unset or blank, an invalid value degraded to unset, or a
//! valid id whose binary is not on `PATH`. The dialog shows **one universal
//! layout** for every entry case: the config file path, the three catalog
//! rows with their install commands, and the repository URL. It runs no
//! PATH detection and reads no config content — its only environment
//! interaction is resolving the config directory for the path line (the
//! `herdr plugin config-dir alexeyco.review` hint when nothing resolves).
//! It never selects a tool, never writes state, and never opens a review
//! pane — the config file is the only chooser.
//!
//! Ported from the sibling herdr-open-in plugin's dialog mechanics,
//! including the frame shape: no internal title (the popup window already
//! shows the manifest pane title), one blank line above the content, filler
//! blanks down to the help footer pinned to the drawable bottom, the
//! terminal size re-read from `stty size` before every frame (80×24
//! fallback), no trailing line break, and colors from a theme palette
//! resolved once per run (see [`crate::theme`]).
//!
//! Raw mode is entered only through an `stty` child process — no FFI, no
//! extra dependency. The saved `stty -g` settings and the cursor are restored
//! on every exit path: an RAII guard restores on drop and the lifecycle
//! runner also restores explicitly before returning, so the terminal stays
//! sane even when a later write fails.
//!
//! The key loop and the frame builder are split from the terminal plumbing so
//! tests drive them over an in-memory reader/writer pair without a terminal.
//! Raw mode uses `min 0 time 1` (VMIN=0, VTIME=1): every `read` returns
//! within 0.1 s, so on a real terminal a lone Escape is followed by a
//! timed-out read rather than a wait for a second byte — that is how a
//! closing Escape is told apart from a closed terminal.

use std::io::{ErrorKind, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::theme::Palette;
use crate::tool::{self, Tool};

/// Hides the hardware cursor while the dialog redraws.
const HIDE_CURSOR: &str = "\x1b[?25l";

/// Shows the hardware cursor again; written on every restore path.
const SHOW_CURSOR: &str = "\x1b[?25h";

/// Moves the cursor to the top-left corner before redrawing.
const HOME: &str = "\x1b[H";

/// Clears from the cursor to the end of the screen before redrawing.
const CLEAR_TAIL: &str = "\x1b[J";

/// Dim SGR wrapping the inline install command after each tool name.
const DIM: &str = "\x1b[2m";

/// Resets all SGR attributes.
const RESET: &str = "\x1b[0m";

/// The help footer: the dialog's single affordance — any closing key.
const FOOTER: &str = "enter/q/esc close";

/// The drawable area, in `(rows, cols)`, the dialog assumes when `stty size`
/// cannot run or its output cannot be parsed: the standard 80×24 terminal.
const DEFAULT_SIZE: (usize, usize) = (24, 80);

/// The left padding on every content line: two visible columns.
const LEFT_PAD: &str = "  ";

/// The content's first label line.
const CONFIG_FILE_LINE: &str = "Config file:";

/// The label line above the catalog rows.
const ADD_TOOLS_LINE: &str = "Add one of these tools to config:";

/// The label line above the repository URL.
const MORE_INFO_LINE: &str = "For more information:";

/// The repository URL, the last content line before the blank above the
/// footer.
const REPO_URL: &str = "https://github.com/alexeyco/herdr-review";

/// The value printed under `Config file:` when no config directory
/// resolves: the command that finds the directory, instead of a file path.
const CONFIG_DIR_HINT: &str = "herdr plugin config-dir alexeyco.review";

/// The dialog content: fully static apart from the config-file value.
struct Dialog {
    /// The line under `Config file:` — the resolved `<dir>/config.toml`
    /// path, or the command that finds the directory.
    config_line: String,
}

impl Dialog {
    /// Builds the one universal dialog from the resolved config directory.
    fn new(config_dir: Option<&Path>) -> Self {
        let config_line = match config_dir {
            Some(dir) => dir.join("config.toml").display().to_string(),
            None => CONFIG_DIR_HINT.to_string(),
        };
        Self { config_line }
    }
}

/// Runs the dialog until a key closes it. The pane resolves only the config
/// directory for its path line, draws the universal dialog, and waits for
/// one key. Returns `Ok(())` — exit 0 — on every closing path; only a
/// terminal failure (raw mode, drawing, restore) is an error, which the
/// pane-mode error policy holds.
pub(crate) fn run() -> Result<(), String> {
    let dialog = Dialog::new(crate::config::resolve_dir().as_deref());
    // A tty, entered below with `min 0 time 1`, reports a 0-byte read on a
    // timeout; every other reader reports end of input there. Record which
    // one this run reads from so the key loop can tell a bare Escape from a
    // closed stdin.
    let read_timeout = std::io::stdin().is_terminal();
    run_lifecycle(
        &dialog,
        Draw {
            palette: crate::theme::resolve_palette(),
            size: || terminal_size(real_stty),
        },
        real_stty,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        read_timeout,
    )
}

/// The stty invocation used for raw-mode control; a plain fn so tests can
/// substitute a recording fake.
type Stty = fn(&[&str]) -> Result<String, String>;

/// The drawing inputs shared by every frame: the theme palette, resolved
/// once before the first frame and never re-read while the dialog is open,
/// and the terminal-size source called before every redraw.
struct Draw<S> {
    palette: crate::theme::Palette,
    size: S,
}

/// The full terminal lifecycle: enter raw mode, run the loop over the given
/// io, restore. [`run`] passes the real stdin/stdout; tests pass in-memory
/// seams plus a recording stty. The reader and writer are consumed by the
/// loop, so they are released before the restore writes the show-cursor
/// sequence.
fn run_lifecycle<R: Read, W: Write, S: FnMut() -> (usize, usize)>(
    dialog: &Dialog,
    draw: Draw<S>,
    stty: Stty,
    reader: R,
    writer: W,
    read_timeout: bool,
) -> Result<(), String> {
    let mut raw = RawMode::enter(stty)?;
    let outcome = run_with(dialog, draw, reader, writer, read_timeout);
    // Explicit restore before returning so the terminal is sane even if a
    // later write failed; `RawMode::drop` is the backstop for early returns.
    let restored = raw.restore();
    match (outcome, restored) {
        (Err(outcome), _) => Err(outcome),
        (Ok(()), Err(restore)) => Err(restore),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Draw loop and key loop, driven by injected reader/writer seams. `draw`
/// carries the palette resolved once by [`run`] and the size source; the
/// size is re-read before every frame, so a resize takes effect from the
/// next redraw — [`run`] passes [`terminal_size`] over the real stty and
/// tests pass a fixed closure so no frame needs a tty.
///
/// The dialog is read-only: any key closes it (`Enter`, `q`, and `Esc` all
/// land here, as does any other keypress), the process returns `Ok(())`,
/// and the popup auto-closes — no call is made and nothing is written. A
/// timed-out read means no key arrived yet, so the loop keeps running; a
/// closed stdin ends the dialog the same clean way.
fn run_with<R: Read, W: Write, S: FnMut() -> (usize, usize)>(
    dialog: &Dialog,
    mut draw: Draw<S>,
    reader: R,
    mut writer: W,
    read_timeout: bool,
) -> Result<(), String> {
    let mut keys = Keys::new(reader, read_timeout);
    loop {
        let drawn = frame(dialog, &draw.palette, (draw.size)());
        writer
            .write_all(drawn.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|e| format!("cannot draw the setup dialog: {e}"))?;
        match read_key(&mut keys)? {
            // No key arrived within the read timeout; keep the dialog open.
            Key::Timeout => {}
            _ => return Ok(()),
        }
    }
}

/// The terminal's drawable area as `(rows, cols)`, read by running `stty
/// size` through the given stty seam — the same invocation pattern as the
/// raw-mode `stty` calls, bound to the controlling tty. Called before every
/// frame so a resize applies on the next redraw. Any failure (no `stty`, a
/// non-tty stdin, unparsable output) silently falls back to
/// [`DEFAULT_SIZE`]: an undersized dialog is recoverable, a refused redraw
/// is not.
fn terminal_size(stty: Stty) -> (usize, usize) {
    stty(&["size"])
        .ok()
        .and_then(|output| parse_stty_size(&output))
        .unwrap_or(DEFAULT_SIZE)
}

/// Parses the `"<rows> <cols>"` output of `stty size`: exactly two
/// whitespace-separated positive integers, or `None` so the caller falls
/// back to the default drawable area.
fn parse_stty_size(output: &str) -> Option<(usize, usize)> {
    let mut fields = output.split_whitespace();
    let rows: usize = fields.next()?.parse().ok()?;
    let cols: usize = fields.next()?.parse().ok()?;
    if rows == 0 || cols == 0 || fields.next().is_some() {
        return None;
    }
    Some((rows, cols))
}

/// One blank frame line (top, filler, or separator): the `panel_bg`
/// background painted across the full width with no foreground. Under the
/// ANSI-16 fallback `panel_bg` is `Default` and `bg_sgr()` is empty, so the
/// line emits no SGR at all — not even a reset — and plain spaces reach the
/// terminal.
fn blank_line(palette: &Palette, width: usize) -> String {
    let bg = palette.panel_bg.bg_sgr();
    let spaces = " ".repeat(width);
    if bg.is_empty() {
        format!("{spaces}\r\n")
    } else {
        format!("{bg}{spaces}{RESET}\r\n")
    }
}

/// One full-width content line: the two-column left padding, the base
/// text-on-panel colors, the text padded to the full width so the panel
/// background stays continuous (the right padding), the reset, and the line
/// break.
fn styled_line(palette: &Palette, text: &str, width: usize) -> String {
    let padded = format!("{LEFT_PAD}{text}");
    let padding = " ".repeat(width.saturating_sub(padded.chars().count()));
    format!(
        "{}{}{padded}{padding}{RESET}\r\n",
        palette.text.fg_sgr(),
        palette.panel_bg.bg_sgr()
    )
}

/// The catalog's name column width: the widest tool id, so every install
/// command starts two columns after the longest name.
fn name_column_width() -> usize {
    tool::CATALOG
        .iter()
        .map(|tool| tool.id.chars().count())
        .max()
        .unwrap_or(0)
}

/// One catalog row: the two-column left padding, the tool name padded to
/// the name column, a two-column gap, then the install command dim on the
/// same line — the whole line padded to the full width so the panel
/// background stays continuous.
fn row_line(palette: &Palette, tool: &Tool, width: usize) -> String {
    let head = format!(
        "{LEFT_PAD}{:<name_width$}  ",
        tool.id,
        name_width = name_column_width()
    );
    let text_len = head.chars().count() + tool.install.chars().count();
    let padding = " ".repeat(width.saturating_sub(text_len));
    format!(
        "{}{}{head}{DIM}{}{padding}{RESET}\r\n",
        palette.text.fg_sgr(),
        palette.panel_bg.bg_sgr(),
        tool.install
    )
}

/// Renders one complete frame as a single string: hide the cursor, home,
/// clear the tail, then lines that fill the whole drawable area — one blank
/// top line (no title: the popup window already shows the manifest pane
/// title), the universal content block, blank filler lines down to the
/// bottom, and the help footer as the frame's last line, pinned to the
/// drawable bottom. Every content line carries the two-column left padding
/// and is padded with spaces to the full width inside its SGR styling so
/// the background always spans the row; the inline install command is the
/// only dim text. No reverse video (SGR `7`) and no bold are ever emitted —
/// there is no selected row.
///
/// The content is eleven lines — the config-file label and value, a blank,
/// the tools label and three rows, a blank, the URL label and value, and
/// the blank above the footer — so with the top blank and the footer the
/// frame needs thirteen rows, and the manifest's `height = 15` popup fits
/// them with a two-row wrap margin: a machine-dependent config path longer
/// than the popup width wraps onto extra visual lines, and the margin
/// keeps up to two wrapped rows visible with the footer still pinned last.
/// `size` is the drawable `(rows, cols)`; the frame emits exactly `rows`
/// lines when the content fits, the filler count is `rows - 13` (clamped
/// at zero), and no negative padding is ever emitted.
///
/// The frame ends immediately after the footer line, without a trailing
/// line break: on a full-height frame the extra newline would scroll the
/// top line off the drawable area.
fn frame(dialog: &Dialog, palette: &Palette, size: (usize, usize)) -> String {
    let (height, width) = size;
    let blank = blank_line(palette, width);
    let mut out = String::new();
    out.push_str(HIDE_CURSOR);
    out.push_str(HOME);
    out.push_str(CLEAR_TAIL);
    // The first content line is blank: no internal title.
    out.push_str(&blank);
    let mut drawn = 0usize;
    // The config-file block: label, then the path (or the hint).
    out.push_str(&styled_line(palette, CONFIG_FILE_LINE, width));
    drawn += 1;
    out.push_str(&styled_line(palette, &dialog.config_line, width));
    drawn += 1;
    out.push_str(&blank);
    drawn += 1;
    out.push_str(&styled_line(palette, ADD_TOOLS_LINE, width));
    drawn += 1;
    for tool in tool::CATALOG.iter() {
        out.push_str(&row_line(palette, tool, width));
        drawn += 1;
    }
    out.push_str(&blank);
    drawn += 1;
    out.push_str(&styled_line(palette, MORE_INFO_LINE, width));
    drawn += 1;
    out.push_str(&styled_line(palette, REPO_URL, width));
    drawn += 1;
    // The blank between the URL block and the footer.
    out.push_str(&blank);
    drawn += 1;
    // Fill the rest of the drawable area with blank lines; the saturating
    // subtract drops the fill to zero on a height too small for the chrome,
    // so no line is ever emitted below the footer.
    let fill = height.saturating_sub(drawn + 2);
    for _ in 0..fill {
        out.push_str(&blank);
    }
    // The help footer is the frame's last line, pinned to the drawable
    // bottom: fg = overlay1 on bg = panel_bg, padded to the full width.
    let footer_padding = " ".repeat(width.saturating_sub(FOOTER.chars().count()));
    out.push_str(&palette.overlay1.fg_sgr());
    out.push_str(&palette.panel_bg.bg_sgr());
    out.push_str(FOOTER);
    out.push_str(&footer_padding);
    out.push_str(RESET);
    // The frame ends immediately after the footer's last byte: no trailing
    // line break. On a full-height frame the extra newline would scroll the
    // terminal one row up and push the blank top line off the drawable area.
    out
}

/// One result from [`Keys::next_byte`].
enum Byte {
    /// A byte arrived.
    Value(u8),
    /// The `min 0 time 1` read timeout elapsed with no key on a tty.
    TimedOut,
    /// End of input on a reader that is not a tty.
    Eof,
}

/// One decoded key press. The dialog has no selection, so the only
/// distinction that matters is "no key yet" (timeout) versus everything
/// else, which closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Enter,
    Cancel,
    Eof,
    /// No key arrived within the read timeout; keep the loop running.
    Timeout,
    Other,
}

/// Decodes the next key, translating the enter, cancel, and end-of-input
/// cases and ignoring everything else — every decoded key closes the
/// dialog, so `Other` is as good as a named key.
fn read_key<R: Read>(keys: &mut Keys<R>) -> Result<Key, String> {
    let byte = match keys.next_byte()? {
        Byte::Value(byte) => byte,
        // A timed-out read with no key pending is not a close; return to the
        // loop so the dialog stays open.
        Byte::TimedOut => return Ok(Key::Timeout),
        Byte::Eof => return Ok(Key::Eof),
    };
    match byte {
        b'\r' | b'\n' => Ok(Key::Enter),
        b'q' => Ok(Key::Cancel),
        0x1b => read_escape(keys),
        _ => Ok(Key::Other),
    }
}

/// Decodes an escape sequence. A lone `\x1b` (nothing follows it) is a
/// close, and so is a complete or partial CSI/SS3 sequence — the dialog
/// closes on any key. The timeout and end-of-input outcomes after a bare
/// Escape are the only ones distinguished, because that is how a lone
/// Escape is told apart from a closed terminal.
fn read_escape<R: Read>(keys: &mut Keys<R>) -> Result<Key, String> {
    let introducer = match keys.next_byte()? {
        Byte::Value(byte) => byte,
        Byte::TimedOut | Byte::Eof => return Ok(Key::Cancel),
    };
    match introducer {
        b'[' | b'O' => match keys.next_byte()? {
            // The sequence's final byte (arrow keys and friends) is consumed
            // as part of it; an incomplete sequence at a timeout or EOF is
            // still one keypress, which closes.
            Byte::Value(_) | Byte::TimedOut | Byte::Eof => Ok(Key::Other),
        },
        other => {
            // Not a CSI/SS3 introducer (for example Alt+<key>): push the byte
            // back; the Escape itself already closes the dialog.
            keys.unread(other);
            Ok(Key::Other)
        }
    }
}

/// A byte source with one byte of pushback.
struct Keys<R> {
    reader: R,
    pending: Option<u8>,
    /// Whether a 0-byte read means the `min 0 time 1` timeout rather than end
    /// of input.
    read_timeout: bool,
}

impl<R: Read> Keys<R> {
    fn new(reader: R, read_timeout: bool) -> Self {
        Self {
            reader,
            pending: None,
            read_timeout,
        }
    }

    /// Reads the next byte, retrying on `Interrupted`. A 0-byte read becomes
    /// [`Byte::TimedOut`] when `read_timeout` is set and [`Byte::Eof`]
    /// otherwise: on a tty in raw mode it means the 0.1 s read timeout
    /// elapsed with no key, while any other reader (a regular file, a closed
    /// stdin, or the in-memory slices the tests drive) is at end of input.
    fn next_byte(&mut self) -> Result<Byte, String> {
        if let Some(byte) = self.pending.take() {
            return Ok(Byte::Value(byte));
        }
        let mut buf = [0u8; 1];
        loop {
            match self.reader.read(&mut buf) {
                Ok(0) if self.read_timeout => return Ok(Byte::TimedOut),
                Ok(0) => return Ok(Byte::Eof),
                Ok(_) => return Ok(Byte::Value(buf[0])),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("cannot read from the terminal: {e}")),
            }
        }
    }

    /// Queues `byte` to be returned by the next [`Keys::next_byte`] call.
    fn unread(&mut self, byte: u8) {
        self.pending = Some(byte);
    }
}

/// Saved terminal settings plus a restore flag; restores raw mode and the
/// cursor on drop so no exit path can leave the terminal broken.
struct RawMode {
    stty: Stty,
    saved: String,
    restored: bool,
}

impl RawMode {
    /// Saves the current settings with `stty -g`, then switches the terminal
    /// to raw mode with echo off and a 0.1 s read timeout (`min 0 time 1`,
    /// that is VMIN=0 and VTIME=1) so a `read` never blocks waiting for
    /// another byte. Errors when `stty` is missing or fails.
    fn enter(stty: Stty) -> Result<Self, String> {
        let saved = stty(&["-g"])?.trim().to_string();
        if saved.is_empty() {
            return Err("stty -g did not report the terminal settings".to_string());
        }
        stty(&["raw", "-echo", "min", "0", "time", "1"])?;
        Ok(Self {
            stty,
            saved,
            restored: false,
        })
    }

    /// Restores the saved settings and shows the cursor. Idempotent.
    fn restore(&mut self) -> Result<(), String> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        let settings = (self.stty)(&[self.saved.as_str()]);
        let cursor = show_cursor();
        settings.and(cursor)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Best effort: the explicit restore in the lifecycle runner reports
        // the error.
        let _ = self.restore();
    }
}

/// Shows the hardware cursor on stdout.
fn show_cursor() -> Result<(), String> {
    let mut out = std::io::stdout();
    out.write_all(SHOW_CURSOR.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("cannot restore the terminal cursor: {e}"))
}

/// Runs `stty` with its stdin bound to the controlling terminal, returning
/// its stdout. An unset or unopenable `stty` is an error, never a silent
/// fallback into cooked mode.
fn real_stty(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("stty");
    cmd.args(args);
    match std::fs::File::open("/dev/tty") {
        Ok(tty) => cmd.stdin(tty),
        Err(_) => cmd.stdin(Stdio::inherit()),
    };
    let output = cmd.output().map_err(|e| {
        format!(
            "failed to run stty {}; is it installed? {e}",
            args.join(" ")
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "stty {} failed with {}",
            args.join(" "),
            output.status
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|e| format!("stty {} printed invalid UTF-8: {e}", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The drawable width the frame-renderer tests pin: the standard 80
    /// columns, wide enough for the longest content line.
    const TEST_WIDTH: usize = 80;

    /// The manifest's declared popup height: the universal content — eleven
    /// content lines, the blank top line, and the footer — fits it with a
    /// two-row wrap margin above the pinned footer.
    const DECLARED_HEIGHT: usize = 15;

    /// A drawable height taller than the content, so the bottom-pinned
    /// geometry (filler blanks above the footer) shows in every snapshot.
    const TALL_HEIGHT: usize = 16;

    /// The drawable size the loop tests pin.
    const TEST_SIZE: (usize, usize) = (24, TEST_WIDTH);

    /// A scratch directory removed when the test ends, for the config-path
    /// test.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "herdr-review-setup-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
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

    /// The one universal dialog with a resolved config directory.
    fn dialog() -> Dialog {
        Dialog::new(Some(Path::new("/cfg/review")))
    }

    /// A fixed truecolor palette (the built-in table's catppuccin tokens)
    /// injected into the renderer tests so SGR bytes are deterministic
    /// without reading a herdr config.
    fn test_palette() -> Palette {
        Palette {
            accent: crate::theme::TokenColor::Rgb(137, 180, 250),
            panel_bg: crate::theme::TokenColor::Rgb(24, 24, 37),
            text: crate::theme::TokenColor::Rgb(205, 214, 244),
            overlay1: crate::theme::TokenColor::Rgb(127, 132, 156),
        }
    }

    /// The drawing context the loop tests inject: the fixture palette and a
    /// fixed drawable size, so no frame needs a tty.
    fn test_draw() -> Draw<impl FnMut() -> (usize, usize)> {
        Draw {
            palette: test_palette(),
            size: || TEST_SIZE,
        }
    }

    /// Renders a frame at the manifest-declared popup height.
    fn render(dialog: &Dialog) -> String {
        frame(dialog, &test_palette(), (DECLARED_HEIGHT, TEST_WIDTH))
    }

    /// Renders a frame at a taller drawable area, leaving blank filler
    /// above the footer.
    fn render_tall(dialog: &Dialog) -> String {
        frame(dialog, &test_palette(), (TALL_HEIGHT, TEST_WIDTH))
    }

    /// Strips SGR escape sequences from one frame line; the content lines
    /// the tests inspect contain only terminated `\x1b[...m` sequences.
    fn plain(line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(start) = rest.find('\x1b') {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let end = rest
                .find('m')
                .expect("frame test lines carry only terminated SGR sequences");
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// A plain (SGR-stripped) full-width content line: the two-column left
    /// padding, the text, and the right padding to the full width.
    fn plain_line(text: &str) -> String {
        let padded = format!("{LEFT_PAD}{text}");
        format!(
            "{padded}{}",
            " ".repeat(TEST_WIDTH.saturating_sub(padded.chars().count()))
        )
    }

    /// The frame's lines with the clear-sequence prefix removed: one entry
    /// per drawn line, the footer last. The frame carries no trailing line
    /// break, so splitting `\r\n` yields exactly the drawn lines, and a frame
    /// that ends on a break surfaces here as a final empty piece.
    fn frame_lines(drawn: &str) -> Vec<&str> {
        let prefix = format!("{HIDE_CURSOR}{HOME}{CLEAR_TAIL}");
        assert!(
            drawn.starts_with(&prefix),
            "frame must start with the clear sequences: {drawn:?}"
        );
        let lines: Vec<&str> = drawn[prefix.len()..].split("\r\n").collect();
        assert!(
            lines.last().copied() != Some(""),
            "the frame must end on the footer's last byte, not a trailing \\r\\n: {drawn:?}"
        );
        lines
    }

    /// Drives the key loop over an in-memory byte slice with no terminal
    /// involved. A slice is not a tty, so a 0-byte read is end of input, not
    /// a timeout.
    fn run_bytes(input: &[u8]) -> Result<(), String> {
        run_with(&dialog(), test_draw(), input, Vec::new(), false)
    }

    // --- Byte-exact style snapshots ---------------------------------------
    //
    // The helpers below build the exact line bytes the frame must emit for
    // the fixture palette (`test_palette()`, the catppuccin tokens): content
    // lines are fg = text on bg = panel_bg with the two-column left padding
    // and the right padding to the full `TEST_WIDTH`, and blank lines carry
    // the panel background alone.

    /// One content line: the base colors, the left padding, `text`, spaces
    /// painting the rest of `TEST_WIDTH`, the reset, and the line break —
    /// exactly as `frame()` emits it.
    fn snap_line(text: &str) -> String {
        let padded = format!("{LEFT_PAD}{text}");
        format!(
            "{}{}{padded}{}{RESET}\r\n",
            test_palette().text.fg_sgr(),
            test_palette().panel_bg.bg_sgr(),
            " ".repeat(TEST_WIDTH.saturating_sub(padded.chars().count()))
        )
    }

    /// One catalog row: the base colors, the left padding, the name padded
    /// to the catalog's name column with the two-column gap, the dim
    /// install command, the right padding, the reset, and the line break.
    fn snap_row(name: &str, install: &str) -> String {
        let head = format!(
            "{LEFT_PAD}{name:<name_width$}  ",
            name_width = name_column_width()
        );
        let text_len = head.chars().count() + install.chars().count();
        format!(
            "{}{}{head}{DIM}{install}{}{RESET}\r\n",
            test_palette().text.fg_sgr(),
            test_palette().panel_bg.bg_sgr(),
            " ".repeat(TEST_WIDTH.saturating_sub(text_len))
        )
    }

    /// One blank line: the panel background alone across the full width.
    fn snap_blank() -> String {
        format!(
            "{}{}{RESET}\r\n",
            test_palette().panel_bg.bg_sgr(),
            " ".repeat(TEST_WIDTH)
        )
    }

    /// The footer line, without the line break: the frame's last line.
    fn snap_footer_tail() -> String {
        format!(
            "{}{}{FOOTER}{}{RESET}",
            test_palette().overlay1.fg_sgr(),
            test_palette().panel_bg.bg_sgr(),
            " ".repeat(TEST_WIDTH - FOOTER.chars().count())
        )
    }

    /// A whole expected frame of `height` lines with `body` content lines:
    /// the clear-sequence prefix, one blank top line, the body, blank filler
    /// lines carrying the panel down to the bottom, and the footer as the
    /// last line — which the frame ends on without a line break.
    fn snap_frame(height: usize, body: String) -> String {
        let drawn = body.matches("\r\n").count();
        let mut out = format!("{HIDE_CURSOR}{HOME}{CLEAR_TAIL}{}", snap_blank());
        out.push_str(&body);
        for _ in 0..height.saturating_sub(drawn + 2) {
            out.push_str(&snap_blank());
        }
        out.push_str(&snap_footer_tail());
        out
    }

    /// The full body of the universal frame: eleven content lines, the last
    /// one the blank above the footer.
    fn snap_body(config_line: &str) -> String {
        let mut body = snap_line(CONFIG_FILE_LINE);
        body.push_str(&snap_line(config_line));
        body.push_str(&snap_blank());
        body.push_str(&snap_line(ADD_TOOLS_LINE));
        for (name, install) in [
            ("lumen", "brew install jnsahaj/lumen/lumen"),
            ("revdiff", "brew install umputun/apps/revdiff"),
            ("tuicr", "brew install tuicr"),
        ] {
            body.push_str(&snap_row(name, install));
        }
        body.push_str(&snap_blank());
        body.push_str(&snap_line(MORE_INFO_LINE));
        body.push_str(&snap_line(REPO_URL));
        body.push_str(&snap_blank());
        body
    }

    /// A key source for the timeout tests. Each entry is either a byte to
    /// yield or `None` to model a 0-byte read that timed out (a tty under
    /// `min 0 time 1`). An exhausted script is an error so a broken loop
    /// fails the test instead of spinning.
    struct ScriptedKeys {
        script: Vec<Option<u8>>,
        position: usize,
    }

    impl Read for ScriptedKeys {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            match self.script.get(self.position) {
                Some(Some(byte)) => {
                    self.position += 1;
                    buf[0] = *byte;
                    Ok(1)
                }
                Some(None) => {
                    self.position += 1;
                    Ok(0)
                }
                None => Err(std::io::Error::other("scripted key source exhausted")),
            }
        }
    }

    /// Drives the loop over a scripted source with empty reads treated as
    /// timeouts, as a real tty under `min 0 time 1` behaves.
    fn run_scripted(script: &[Option<u8>]) -> Result<(), String> {
        let reader = ScriptedKeys {
            script: script.to_vec(),
            position: 0,
        };
        run_with(&dialog(), test_draw(), reader, Vec::new(), true)
    }

    /// A recording stty fake: every call is appended to a thread-local log,
    /// and `stty -g` answers a plausible payload.
    fn recording_stty(args: &[&str]) -> Result<String, String> {
        STTY_CALLS.with(|c| c.borrow_mut().push(args.join(" ")));
        if args == ["-g"] {
            Ok("saved:settings\n".to_string())
        } else {
            Ok(String::new())
        }
    }

    thread_local! {
        static STTY_CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// The stty protocol: `-g` first, the raw-mode switch, then the restore.
    fn assert_stty_restored() {
        STTY_CALLS.with(|c| {
            let calls = c.borrow();
            assert_eq!(
                calls.first().map(String::as_str),
                Some("-g"),
                "the settings must be saved first: {calls:?}"
            );
            assert!(
                calls.contains(&"raw -echo min 0 time 1".to_string()),
                "raw mode must be entered: {calls:?}"
            );
            assert_eq!(
                calls.last().map(String::as_str),
                Some("saved:settings"),
                "the saved settings must be restored last: {calls:?}"
            );
        });
    }

    /// A writer that fails on the first write, so the loop errors after the
    /// frame is attempted.
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("write failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("flush failed"))
        }
    }

    #[test]
    fn every_entry_case_renders_the_same_dialog() {
        // Scenario "Every entry case renders the same dialog": the dialog's
        // only input is the config directory, so every entry case — `tool`
        // unset, blank, invalid, configured-but-missing, or no catalog tool
        // installed — renders identical content apart from the config path
        // value, and no line states which case occurred (the dialog runs no
        // PATH detection and reads no config content).
        let drawn = render(&dialog());
        assert_eq!(
            drawn,
            snap_frame(DECLARED_HEIGHT, snap_body("/cfg/review/config.toml"))
        );
        for absent in [
            "No review tool is configured.",
            "not on PATH",
            "Configured tool",
            "Add tool = ",
        ] {
            assert!(
                !drawn.contains(absent),
                "{absent} must not appear: {drawn:?}"
            );
        }
        // Another config directory changes only the path value.
        let other = render(&Dialog::new(Some(Path::new("/cfg/elsewhere"))));
        assert_eq!(
            other,
            snap_frame(DECLARED_HEIGHT, snap_body("/cfg/elsewhere/config.toml"))
        );
    }

    #[test]
    fn tool_rows_are_alphabetical_with_dim_inline_install_commands() {
        // Scenario "Tool rows are alphabetical with dim inline install
        // commands": the rows appear in alphabetical order — lumen,
        // revdiff, tuicr — each with its install command rendered dim after
        // the tool name on the same line (two-column gap), never on an
        // indented line of its own.
        let drawn = render(&dialog());
        let mut last = 0usize;
        for (name, install) in [
            ("lumen", "brew install jnsahaj/lumen/lumen"),
            ("revdiff", "brew install umputun/apps/revdiff"),
            ("tuicr", "brew install tuicr"),
        ] {
            let row = snap_row(name, install);
            let index = drawn
                .find(&row)
                .unwrap_or_else(|| panic!("{name} row missing: {drawn:?}"));
            assert!(index > last, "alphabetical order: {drawn:?}");
            last = index;
        }
        // The install command is dim and sits after the padded name.
        assert!(
            drawn.contains(&format!("{DIM}brew install umputun/apps/revdiff")),
            "{drawn:?}"
        );
        // No install command renders as its own left-padded content line.
        for install in [
            "brew install jnsahaj/lumen/lumen",
            "brew install umputun/apps/revdiff",
            "brew install tuicr",
        ] {
            assert!(!drawn.contains(&snap_line(install)), "{drawn:?}");
        }
    }

    #[test]
    fn repo_url_line_closes_the_content() {
        // Scenario "Repo URL line closes the content": `For more
        // information:` followed by the repository URL is the last content
        // block — a blank line, then the wrap-margin filler blanks, then
        // the pinned footer as the frame's last line.
        let drawn = render(&dialog());
        let lines = frame_lines(&drawn);
        assert_eq!(lines.len(), DECLARED_HEIGHT, "{drawn:?}");
        assert_eq!(plain(lines[9]), plain_line(MORE_INFO_LINE), "{drawn:?}");
        assert_eq!(plain(lines[10]), plain_line(REPO_URL), "{drawn:?}");
        assert!(
            plain(lines[11]).chars().all(|c| c == ' '),
            "a blank line follows the URL: {drawn:?}"
        );
        assert_eq!(lines[DECLARED_HEIGHT - 1], snap_footer_tail(), "{drawn:?}");
    }

    #[test]
    fn config_path_points_at_the_resolved_file() {
        // Scenario "Config path points at the resolved file": the printed
        // path is `<config dir>/config.toml`, shown even when the file does
        // not exist yet. The path line renders left-padded like every
        // content line; a machine-dependent path longer than the test width
        // simply clips the right padding, so the line is checked by content
        // rather than through the byte-exact snapshot helpers.
        let dir = TempDir::new("config-path");
        assert!(!dir.path().join("config.toml").exists());
        let dialog = Dialog::new(Some(dir.path()));
        let expected = dir.path().join("config.toml").display().to_string();
        assert_eq!(dialog.config_line, expected);
        let drawn = render(&dialog);
        assert!(
            drawn.contains(&format!("{LEFT_PAD}{expected}")),
            "{drawn:?}"
        );
    }

    #[test]
    fn unresolvable_config_directory_still_renders() {
        // Scenario "Unresolvable config directory still renders": the dialog
        // prints the `herdr plugin config-dir alexeyco.review` command as the
        // way to find the path, instead of a file path, and otherwise renders
        // identically.
        let dialog = Dialog::new(None);
        assert_eq!(dialog.config_line, CONFIG_DIR_HINT);
        assert!(
            dialog
                .config_line
                .contains("herdr plugin config-dir alexeyco.review")
        );
        let drawn = render(&dialog);
        assert!(drawn.contains(&snap_line(CONFIG_DIR_HINT)), "{drawn:?}");
        assert_eq!(
            drawn,
            snap_frame(DECLARED_HEIGHT, snap_body(CONFIG_DIR_HINT))
        );
        assert!(drawn.contains(&snap_blank()), "{drawn:?}");
        assert!(drawn.ends_with(&snap_footer_tail()), "{drawn:?}");
    }

    #[test]
    fn content_carries_top_left_and_right_padding() {
        // Scenario "Content carries top, left, and right padding": the first
        // line is blank, every content line starts with the two-column left
        // padding, and the panel-background fill extends every line to the
        // full drawable width.
        let drawn = render(&dialog());
        let lines = frame_lines(&drawn);
        assert!(
            plain(lines[0]).chars().all(|c| c == ' '),
            "the first line must be blank: {drawn:?}"
        );
        let footer = lines.len() - 1;
        for (index, line) in lines.iter().enumerate() {
            if index == 0 || index == footer {
                // The blank top line and the footer are not content lines.
                continue;
            }
            let text = plain(line);
            assert!(
                text.starts_with(LEFT_PAD),
                "line {index} lacks the left padding: {drawn:?}"
            );
            assert_eq!(
                text.chars().count(),
                TEST_WIDTH,
                "line {index} is not padded to the full width: {drawn:?}"
            );
        }
    }

    #[test]
    fn content_fits_the_declared_popup_height() {
        // Scenario "Content fits the declared popup height": at the
        // manifest-declared popup height (15), every content line and the
        // pinned footer are visible without scrolling and the footer is the
        // last line — thirteen fixed rows (blank top, eleven content lines,
        // footer) plus the two-row wrap margin as blank filler above the
        // footer, no clipping.
        let drawn = render(&dialog());
        let lines = frame_lines(&drawn);
        assert_eq!(lines.len(), DECLARED_HEIGHT, "{drawn:?}");
        assert_eq!(
            drawn.matches("\r\n").count(),
            DECLARED_HEIGHT - 1,
            "a `height`-line frame carries `height - 1` line breaks: {drawn:?}"
        );
        assert_eq!(lines[DECLARED_HEIGHT - 1], snap_footer_tail(), "{drawn:?}");
        // The wrap margin is the filler above the pinned footer: two blank
        // panel rows at the declared height, so a wrapped config-path line
        // cannot push the footer out of the popup.
        let blank = snap_blank();
        let blank = blank.trim_end_matches("\r\n");
        for line in &lines[DECLARED_HEIGHT - 3..DECLARED_HEIGHT - 1] {
            assert_eq!(
                *line, blank,
                "the wrap-margin rows must be blank panel lines: {drawn:?}"
            );
        }
        // Every content line is present between the top blank and the
        // footer.
        assert!(drawn.contains(&snap_line(CONFIG_FILE_LINE)), "{drawn:?}");
        assert!(drawn.contains(&snap_line(ADD_TOOLS_LINE)), "{drawn:?}");
        assert!(drawn.contains(&snap_line(MORE_INFO_LINE)), "{drawn:?}");
        assert!(drawn.contains(&snap_line(REPO_URL)), "{drawn:?}");
        for (name, install) in [
            ("lumen", "brew install jnsahaj/lumen/lumen"),
            ("revdiff", "brew install umputun/apps/revdiff"),
            ("tuicr", "brew install tuicr"),
        ] {
            assert!(drawn.contains(&snap_row(name, install)), "{drawn:?}");
        }
    }

    #[test]
    fn cancel_exits_cleanly() {
        // Scenario "Cancel exits cleanly": `q`, Enter, and Esc each close the
        // dialog with no herdr call, no write, and a clean exit — the loop
        // returns `Ok(())` and the module performs no calls and no writes
        // by construction.
        for input in [b"q".as_ref(), b"\r".as_ref(), b"\x1b".as_ref()] {
            let mut out = Vec::new();
            let result = run_with(&dialog(), test_draw(), input, &mut out, false);
            assert!(result.is_ok(), "{result:?}");
            assert!(!out.is_empty(), "a frame must be drawn: {out:?}");
        }
    }

    #[test]
    fn escape_cancels() {
        // Scenario "Escape cancels": a lone Escape keypress closes the dialog
        // as if `q` was pressed.
        assert!(run_bytes(b"\x1b").is_ok());
        assert!(run_bytes(b"q").is_ok());
    }

    #[test]
    fn escape_then_timeout_closes_without_another_byte() {
        // Scenario "Escape cancels", timeout clause: on a real tty a lone
        // Escape is followed by a 0-byte read, not end of input, and still
        // closes — the key reader uses a ~0.1 s timeout, not a blocking
        // read.
        assert!(run_scripted(&[Some(0x1b), None]).is_ok());
    }

    #[test]
    fn timeout_with_no_key_keeps_the_dialog_open() {
        // A 0-byte read means "no key yet": the loop redraws (a second frame
        // appears) and the following key closes.
        let mut out = Vec::new();
        let reader = ScriptedKeys {
            script: vec![None, Some(b'q')],
            position: 0,
        };
        let result = run_with(&dialog(), test_draw(), reader, &mut out, true);
        assert!(result.is_ok(), "{result:?}");
        let frames = String::from_utf8(out).unwrap();
        assert_eq!(
            frames.matches(HIDE_CURSOR).count(),
            2,
            "two frames drawn: {frames:?}"
        );
    }

    #[test]
    fn eof_closes_the_dialog() {
        // A closed stdin ends the dialog the same clean way; the terminal
        // restore still runs on that path.
        assert!(run_bytes(b"").is_ok());
    }

    #[test]
    fn terminal_is_restored_on_every_exit_path() {
        // Scenario "Terminal is restored on every exit path": exit by key,
        // by end of input, and by error each restore the saved stty settings
        // (and the cursor, written on the same path).
        let dialog = dialog();
        for input in [b"\r".as_slice(), b"q".as_slice(), b"".as_slice()] {
            STTY_CALLS.with(|c| c.borrow_mut().clear());
            let result = run_lifecycle(
                &dialog,
                test_draw(),
                recording_stty,
                input,
                Vec::new(),
                false,
            );
            assert!(result.is_ok(), "{result:?}");
            assert_stty_restored();
        }
        // The drawing-error exit path restores too.
        STTY_CALLS.with(|c| c.borrow_mut().clear());
        let result = run_lifecycle(
            &dialog,
            test_draw(),
            recording_stty,
            &b"\r"[..],
            FailingWriter,
            false,
        );
        assert!(result.is_err(), "{result:?}");
        assert_stty_restored();
    }

    #[test]
    fn frame_chrome_carries_no_internal_title() {
        // Scenario "Frame chrome carries no internal title": the first
        // content line is a blank line, no title text is rendered (the popup
        // window chrome already shows the manifest pane title), and the
        // frame's last byte is the footer padding with no trailing newline —
        // the whole frame pinned byte for byte.
        let drawn = render(&dialog());
        assert_eq!(
            drawn,
            snap_frame(DECLARED_HEIGHT, snap_body("/cfg/review/config.toml"))
        );
        let lines = frame_lines(&drawn);
        assert_eq!(lines.len(), DECLARED_HEIGHT, "{drawn:?}");
        assert!(
            plain(lines[0]).chars().all(|c| c == ' '),
            "the first content line must be blank: {drawn:?}"
        );
        assert!(!drawn.contains("Review tool setup"), "{drawn:?}");
        assert!(
            !drawn.ends_with("\r\n"),
            "the frame must not end with a trailing line break: {drawn:?}"
        );
        assert!(
            drawn.ends_with(&snap_footer_tail()),
            "the frame's last bytes are the footer padding: {drawn:?}"
        );
    }

    #[test]
    fn footer_is_pinned_to_the_drawable_bottom() {
        // Scenario "Footer is pinned to the drawable bottom": the terminal
        // is taller than the content, so blank filler lines carry the panel
        // background down to the bottom-most line, the help footer is that
        // line, and the frame holds exactly `height - 1` line breaks.
        let drawn = render_tall(&dialog());
        let lines = frame_lines(&drawn);
        assert_eq!(lines.len(), TALL_HEIGHT, "{drawn:?}");
        assert_eq!(
            drawn.matches("\r\n").count(),
            TALL_HEIGHT - 1,
            "a `height`-line frame carries `height - 1` line breaks: {drawn:?}"
        );
        let blank = snap_blank();
        let blank = blank.trim_end_matches("\r\n");
        // The content ends after the blank above the footer (top blank + 11
        // content lines = 12 lines); every line between there and the footer
        // is a panel blank.
        for line in &lines[12..TALL_HEIGHT - 1] {
            assert_eq!(
                *line, blank,
                "every filler line is a panel blank: {drawn:?}"
            );
        }
        assert_eq!(lines[TALL_HEIGHT - 1], snap_footer_tail());
    }

    #[test]
    fn the_terminal_size_is_reread_on_every_redraw() {
        // Scenario "Footer is pinned to the drawable bottom", redraw clause:
        // the drawable size is re-read before every frame, so a resize takes
        // effect from the next redraw.
        let dialog = dialog();
        let reads = RefCell::new(0usize);
        run_with(
            &dialog,
            Draw {
                palette: test_palette(),
                size: || {
                    *reads.borrow_mut() += 1;
                    TEST_SIZE
                },
            },
            // A timeout (no key yet), then a key: two frames.
            ScriptedKeys {
                script: vec![None, Some(b'q')],
                position: 0,
            },
            Vec::new(),
            true,
        )
        .unwrap();
        // Two frames drawn — the initial one and the one after the timeout —
        // so the size seam answered once per frame.
        assert_eq!(*reads.borrow(), 2, "one size read per frame");
    }

    #[test]
    fn stty_size_output_parses_and_falls_back() {
        // The per-frame size read: `stty size` output parses as exactly two
        // positive integers, and any failure — a missing stty, a nonzero
        // exit, unparsable output — silently falls back to the 80×24
        // default.
        assert_eq!(parse_stty_size("24 80\n"), Some((24, 80)));
        assert_eq!(parse_stty_size("10 40"), Some((10, 40)));
        for bad in ["", "24", "24 80 extra", "0 80", "24 0", "a b", "-1 80"] {
            assert_eq!(parse_stty_size(bad), None, "{bad:?}");
        }
        let fail: Stty = |_| Err("no stty".to_string());
        assert_eq!(terminal_size(fail), DEFAULT_SIZE);
        let garbage: Stty = |_| Ok("garbage".to_string());
        assert_eq!(terminal_size(garbage), DEFAULT_SIZE);
    }

    #[test]
    fn frame_colors_follow_the_herdr_theme_palette() {
        // Scenario "Frame colors follow the herdr theme palette", frame side:
        // the painted tokens (panel background, text, overlay1) come from
        // the palette resolved once before the first frame — the resolution
        // itself (built-in names, $HERDR_CONFIG_PATH, and the
        // unset/unreadable/invalid/unknown fallbacks) is covered by
        // src/theme.rs. Under the ANSI-16 fallback the content lines emit no
        // fg/bg SGR at all, the blanks emit no SGR at all, and the footer
        // carries only the bright-black foreground.
        let dialog = dialog();
        for palette in [test_palette(), crate::theme::fallback_palette()] {
            let drawn = frame(&dialog, &palette, (DECLARED_HEIGHT, TEST_WIDTH));
            assert!(
                drawn.contains(&format!(
                    "{}{}{LEFT_PAD}revdiff",
                    palette.text.fg_sgr(),
                    palette.panel_bg.bg_sgr()
                )),
                "the rows must paint with the palette's text/panel_bg tokens: {drawn:?}"
            );
            assert!(
                drawn.contains(&format!(
                    "{}{}{FOOTER}",
                    palette.overlay1.fg_sgr(),
                    palette.panel_bg.bg_sgr()
                )),
                "the footer must paint with the palette's overlay1/panel_bg tokens: {drawn:?}"
            );
        }
        // The ANSI-16 fallback's exact line shapes.
        let drawn = frame(
            &dialog,
            &crate::theme::fallback_palette(),
            (DECLARED_HEIGHT, TEST_WIDTH),
        );
        assert!(
            drawn.contains(&format!(
                "{LEFT_PAD}revdiff  {DIM}brew install umputun/apps/revdiff"
            )),
            "the fallback row carries no fg/bg SGR: {drawn:?}"
        );
        assert!(drawn.contains(&format!("\x1b[90m{FOOTER}")), "{drawn:?}");
        let lines = frame_lines(&drawn);
        assert!(
            !lines[0].contains('\x1b'),
            "fallback blanks emit no SGR at all"
        );
        assert_eq!(plain(lines[0]), " ".repeat(TEST_WIDTH));
    }
}
