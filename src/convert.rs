//! Converting tool-native annotation exports to the canonical block format.
//!
//! The canonical format is revdiff's native output grammar, which revdiff
//! passes through unconverted: each block is a header line
//! `## <path>:<line> (<side>)` — side `+` or `-` — followed by comment body
//! lines, with blocks separated by blank lines. tuicr JSON comments and lumen
//! exports are converted into it, lossily by design: one side marker and one
//! anchor line per block is what the delivered prompt contract expects.
//!
//! lumen captures begin with terminal-probe escape sequences (the live spike
//! observed `ESC]11;?BEL` + `ESC[5n` before the payload), so the lumen parser
//! strips CSI/OSC/ANSI escapes before block parsing.

use serde_json::Value;

/// One tuicr review comment, as emitted by `tuicr review comments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    /// File path; the canonical header's `<path>`.
    pub(crate) path: String,
    /// Anchor line; the canonical header's `<line>`. `end_line` is not
    /// rendered — the block anchors on the start line.
    pub(crate) start_line: u64,
    /// `side` field: RIGHT/new map to `+`, LEFT/old to `-`, anything else
    /// (including absent) to the `+` default.
    pub(crate) side: Option<String>,
    /// The comment text; the canonical body.
    pub(crate) content: String,
}

/// Parses the JSON array `tuicr review comments` prints. Comments without a
/// usable file path are skipped: the canonical grammar has no slot for a
/// review-level comment.
pub(crate) fn parse_comments(json: &str) -> Result<Vec<Comment>, String> {
    let v: Value =
        serde_json::from_str(json).map_err(|e| format!("invalid tuicr comments JSON: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "tuicr comments JSON is not an array".to_string())?;
    Ok(arr.iter().filter_map(comment_from_value).collect())
}

/// Extracts one comment; `None` skips an entry with no non-blank `path`.
fn comment_from_value(v: &Value) -> Option<Comment> {
    let path = v.get("path")?.as_str()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(Comment {
        path: path.to_string(),
        start_line: v.get("start_line").and_then(Value::as_u64).unwrap_or(0),
        side: v.get("side").and_then(Value::as_str).map(str::to_string),
        content: v
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Converts tuicr comments to the canonical text. `side` maps RIGHT→`+` and
/// LEFT→`-`; tuicr's own lowercase `new`/`old` map the same way, and a missing
/// or unrecognized side defaults to `+` — annotations overwhelmingly target
/// new code.
pub(crate) fn tuicr_to_canonical(comments: &[Comment]) -> String {
    let blocks: Vec<String> = comments
        .iter()
        .map(|c| {
            format!(
                "## {}:{} ({})\n{}",
                c.path,
                c.start_line,
                side_marker(c.side.as_deref()),
                c.content.trim()
            )
        })
        .collect();
    if blocks.is_empty() {
        return String::new();
    }
    format!("{}\n", blocks.join("\n\n"))
}

/// The canonical side marker for one tuicr side value.
fn side_marker(side: Option<&str>) -> &'static str {
    match side.map(str::trim) {
        Some("RIGHT") | Some("new") => "+",
        Some("LEFT") | Some("old") => "-",
        _ => "+",
    }
}

/// Converts a lumen stdout export to the canonical text. Returns `None` when
/// a non-blank export parses to zero blocks — the caller's hard "cannot parse"
/// error. A blank input yields empty output: the caller treats blank stdout as
/// the clean no-annotations case before parsing.
///
/// The export layout: an optional `# <ref>` header line, blocks separated by
/// `---` lines, each starting with `**path** line N (RIGHT)`,
/// `**path** lines A-B (LEFT)`, or a bare `**path**` (file-level). Remaining
/// lines of a block are the annotation text. `# <ref>` headers and other
/// unrecognized lines before the first block are dropped.
pub(crate) fn lumen_to_canonical(raw: &str) -> Option<String> {
    let stripped = strip_terminal_escapes(raw);
    let blocks = parse_lumen_blocks(&stripped);
    if blocks.is_empty() {
        return if stripped.trim().is_empty() {
            Some(String::new())
        } else {
            None
        };
    }
    Some(format!("{}\n", blocks.join("\n\n")))
}

/// Splits the export into rendered blocks: a `---` line closes the current
/// block, a header line opens one, and every other line is body text when a
/// block is open (or dropped before the first block).
fn parse_lumen_blocks(input: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in input.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line == "---" {
            if let Some(block) = current.take() {
                blocks.push(render_block(block));
            }
            continue;
        }
        if let Some(header) = parse_lumen_header(line) {
            if let Some(block) = current.take() {
                blocks.push(render_block(block));
            }
            current = Some((header, Vec::new()));
            continue;
        }
        // Ref headers and other unrecognized text before the first block are
        // dropped; inside a block the line is body text.
        if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    if let Some(block) = current.take() {
        blocks.push(render_block(block));
    }
    blocks
}

/// Renders one block: the canonical header, then the body with leading and
/// trailing blank lines trimmed (lumen puts a blank line between the header
/// and the text) and interior blank lines kept.
fn render_block((header, body): (String, Vec<&str>)) -> String {
    let trimmed: Vec<&str> = body
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .copied()
        .collect();
    let end = trimmed
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(0, |i| i + 1);
    let body_lines = &trimmed[..end];
    if body_lines.is_empty() {
        return header;
    }
    format!("{header}\n{}", body_lines.join("\n"))
}

/// Parses one lumen header line into its canonical header, or `None` when the
/// line is not a header. A line/range annotation without a `(RIGHT)`/`(LEFT)`
/// marker takes the `+` default; a range anchors on its first line; a
/// file-level `**path**` block anchors on line 0.
fn parse_lumen_header(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("**")?;
    let (path, rest) = rest.split_once("**")?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let rest = rest.trim();
    let (spec, side) = match rest.strip_suffix("(RIGHT)") {
        Some(spec) => (spec.trim(), "+"),
        None => match rest.strip_suffix("(LEFT)") {
            Some(spec) => (spec.trim(), "-"),
            None => (rest, "+"),
        },
    };
    let line_no = if let Some(n) = spec.strip_prefix("line ") {
        n.trim().parse().ok()?
    } else if let Some(range) = spec.strip_prefix("lines ") {
        let (first, _) = range.trim().split_once('-')?;
        first.trim().parse().ok()?
    } else if spec.is_empty() {
        0
    } else {
        return None;
    };
    Some(format!("## {path}:{line_no} ({side})"))
}

/// Strips terminal-probe escape sequences from a captured stdout: CSI
/// sequences (`ESC[` … final byte), OSC sequences (`ESC]` … `BEL` or `ESC\`),
/// and other two-byte ANSI ESC sequences. The live spike showed lumen's
/// capture begins with `ESC]11;?BEL` + `ESC[5n` before the annotation payload;
/// the payload itself never carries an `ESC`.
pub(crate) fn strip_terminal_escapes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        // An ESC byte: consume the whole escape sequence, emit nothing.
        i += 1;
        match bytes.get(i) {
            None => {}
            Some(b'[') => {
                // CSI: parameter and intermediate bytes, then the final byte
                // in 0x40..=0x7e.
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            Some(b']') => {
                // OSC: terminated by BEL or ST (ESC \).
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            Some(_) => {
                // Any other ESC-prefixed pair (ESC 7, ESC M, …): drop both.
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(path: &str, start_line: u64, side: Option<&str>, content: &str) -> Comment {
        Comment {
            path: path.to_string(),
            start_line,
            side: side.map(str::to_string),
            content: content.to_string(),
        }
    }

    #[test]
    fn tuicr_comment_converts_with_side_mapping() {
        // Scenario "tuicr comment converts with side mapping": the header
        // anchors on the start line; end_line is never rendered; LEFT maps to
        // `-` and RIGHT to `+`.
        let canonical = tuicr_to_canonical(&[
            comment("src/a.rs", 12, Some("LEFT"), "move this"),
            comment("src/b.rs", 3, Some("RIGHT"), "keep this"),
        ]);
        assert_eq!(
            canonical,
            "## src/a.rs:12 (-)\nmove this\n\n## src/b.rs:3 (+)\nkeep this\n"
        );
        assert!(!canonical.contains("15"), "{canonical}");
    }

    #[test]
    fn tuicr_comment_without_side_defaults_to_plus() {
        // Scenario "tuicr comment without side defaults to plus": a missing
        // or unrecognized side yields `(+)`.
        for side in [None, Some("sideways"), Some("")] {
            let canonical = tuicr_to_canonical(&[comment("a.rs", 1, side, "note")]);
            assert!(canonical.starts_with("## a.rs:1 (+)"), "{canonical:?}");
        }
    }

    #[test]
    fn tuicr_new_and_old_sides_map_like_right_and_left() {
        // tuicr 0.27 emits lowercase `new`/`old`; mapped to the same markers
        // as the specced RIGHT/LEFT.
        assert_eq!(side_marker(Some("new")), "+");
        assert_eq!(side_marker(Some("old")), "-");
    }

    #[test]
    fn tuicr_empty_comment_list_is_empty_canonical_text() {
        assert_eq!(tuicr_to_canonical(&[]), "");
    }

    #[test]
    fn tuicr_comments_parse_from_json() {
        let json = r#"[
            {"path":"src/a.rs","start_line":12,"end_line":15,"side":"LEFT","content":"move this"},
            {"path":"src/b.rs","line":3,"content":"keep this"}
        ]"#;
        let comments = parse_comments(json).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].side.as_deref(), Some("LEFT"));
        assert_eq!(comments[1].start_line, 0);
        assert_eq!(comments[1].side, None);
    }

    #[test]
    fn tuicr_invalid_comments_json_is_an_error() {
        for json in ["{not json", "{}", "42"] {
            assert!(parse_comments(json).is_err(), "{json}");
        }
    }

    #[test]
    fn tuicr_comments_without_a_path_are_skipped() {
        // A review-level comment has no file anchor; the canonical grammar has
        // no slot for it.
        let json = r#"[{"content":"general note"},{"path":"  ","content":"blank path"},{"path":"a.rs","start_line":1,"content":"kept"}]"#;
        let comments = parse_comments(json).unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].path, "a.rs");
    }

    #[test]
    fn lumen_right_line_converts() {
        // Scenario "lumen RIGHT line converts": `(LEFT)` maps to `-` the same
        // way. The blank line between header and body is trimmed.
        let raw = "**src/a.rs** line 7 (RIGHT)\n\nrename this\n---\n**src/b.rs** line 2 (LEFT)\n\ndrop this\n";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## src/a.rs:7 (+)\nrename this\n\n## src/b.rs:2 (-)\ndrop this\n"
        );
    }

    #[test]
    fn lumen_line_range_anchors_at_the_first_line() {
        // Scenario "lumen line range anchors at the first line".
        let raw = "**src/a.rs** lines 4-9 (LEFT)\n\nsimplify this range\n";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## src/a.rs:4 (-)\nsimplify this range\n"
        );
    }

    #[test]
    fn lumen_file_level_annotation_converts_to_line_0() {
        // Scenario "lumen file-level annotation converts to line 0".
        let raw = "**src/a.rs**\n\ngeneral file note\n";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## src/a.rs:0 (+)\ngeneral file note\n"
        );
    }

    #[test]
    fn lumen_ref_header_is_dropped() {
        // Scenario "lumen ref header is dropped": the `# <ref>` line
        // contributes no block and no body text; unrecognized lines before
        // the first block are dropped too.
        let raw =
            "# origin/main\nnoise before the first block\n**src/a.rs** line 1 (RIGHT)\n\nnote\n";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## src/a.rs:1 (+)\nnote\n"
        );
    }

    #[test]
    fn lumen_blank_input_is_blank_output() {
        assert_eq!(lumen_to_canonical("").unwrap(), "");
        assert_eq!(lumen_to_canonical("  \n\t").unwrap(), "");
    }

    #[test]
    fn lumen_unparseable_non_blank_input_is_none() {
        assert!(lumen_to_canonical("just some text\n").is_none());
        assert!(lumen_to_canonical("---\n---\n").is_none());
    }

    #[test]
    fn lumen_multibyte_body_survives() {
        let raw = "**src/ review.rs** line 1 (RIGHT)\n\nпримечание\n";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## src/ review.rs:1 (+)\nпримечание\n"
        );
    }

    #[test]
    fn terminal_probe_preamble_is_stripped_before_parsing() {
        // Scenario "Terminal-probe preamble is stripped before parsing": the
        // live-spike preamble (`ESC]11;?BEL` + `ESC[5n`) precedes the payload
        // and is discarded; the blocks parse normally.
        let raw = "\u{1b}]11;?\u{7}\u{1b}[5n**file.txt** line 2 (RIGHT)\n\nspike annotation 2";
        assert_eq!(
            lumen_to_canonical(raw).unwrap(),
            "## file.txt:2 (+)\nspike annotation 2\n"
        );
    }

    #[test]
    fn escape_stripper_removes_csi_osc_and_two_byte_escapes() {
        // The spike preamble, a color SGR burst, an OSC with ST terminator,
        // and a lone trailing ESC all vanish; the payload text stays.
        let raw = "\u{1b}]11;?\u{7}\u{1b}[5nplain\u{1b}[38;2;1;2;3mtext\u{1b}[0m\u{1b}]0;title\u{1b}\\more\u{1b}M";
        assert_eq!(strip_terminal_escapes(raw), "plaintextmore");
    }

    #[test]
    fn escape_stripper_keeps_text_without_escapes_untouched() {
        let raw = "## a.rs:1 (+)\nbody\n";
        assert_eq!(strip_terminal_escapes(raw), raw);
    }
}
