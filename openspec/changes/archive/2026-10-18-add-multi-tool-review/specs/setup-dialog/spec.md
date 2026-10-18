# Spec Delta

## Purpose

The setup-dialog capability is the read-only popup pane the review action
opens when no usable review tool is configured. It shows one universal
layout — the config file path, the static catalog with install commands,
and the repo URL — telling the user how to configure a tool, and closes on
any key. It never branches on install or config state, never selects a
tool, never writes state, and never opens a review pane: the config file
is the only chooser.

## ADDED Requirements

### Requirement: The dialog shows one universal layout for every entry case

The dialog SHALL render the same content for every reason it was opened
(`tool` unset, blank, invalid, configured-but-missing, or nothing
installed): a `Config file:` line with the resolved config path, an
`Add one of these tools to config:` line with the three static catalog
rows, a `For more information:` line with the repository URL, and the
pinned footer. No branch status line, no PATH state, and no greyed rows
are rendered; the dialog performs no PATH detection at all.

#### Scenario: Every entry case renders the same dialog

- **WHEN** the popup opens for any of the entry cases — `tool` unset,
  blank, invalid, configured-but-missing, or no catalog tool installed
- **THEN** the rendered content is identical in all of them apart from the
  config path value, and no line states which case occurred

#### Scenario: Tool rows are alphabetical with dim inline install commands

- **WHEN** the dialog renders the catalog rows
- **THEN** the rows appear in alphabetical order — `lumen`, `revdiff`,
  `tuicr` — each with its install command rendered dim after the tool name
  on the same line: `brew install jnsahaj/lumen/lumen`, `brew install
umputun/apps/revdiff`, `brew install tuicr`

#### Scenario: Repo URL line closes the content

- **WHEN** the dialog renders
- **THEN** `For more information:` followed by
  `https://github.com/alexeyco/herdr-review` is the last content block
  before the footer

#### Scenario: Config path points at the resolved file

- **WHEN** the config directory resolves through `HERDR_PLUGIN_CONFIG_DIR`
  or the `herdr plugin config-dir alexeyco.review` fallback
- **THEN** the printed path is `<config dir>/config.toml`, shown even when
  the file does not exist yet

#### Scenario: Unresolvable config directory still renders

- **WHEN** no config directory can be resolved
- **THEN** the dialog prints the `herdr plugin config-dir alexeyco.review` command
  as the way to find the path, instead of a file path, and otherwise
  renders identically

### Requirement: The dialog content is padded and fits the declared popup size

The dialog SHALL keep the blank top line, indent every content line with a
visible left padding of two columns, and fill each line to the full width
with the panel background as the right padding. The manifest's `setup`
pane SHALL declare popup size keys fitted to the content without
scrolling: `width = "60%"` and `height = 15` (13 content lines plus a
two-line wrap margin for long config paths — live-probed clipping).

#### Scenario: Content carries top, left, and right padding

- **WHEN** any frame is drawn
- **THEN** the first line is blank, every content line starts with the
  two-column left padding, and the panel-background fill extends every
  line to the full drawable width

#### Scenario: Content fits the declared popup height

- **WHEN** the dialog renders at the manifest-declared popup height
- **THEN** every content line and the pinned footer are visible without
  scrolling and the footer is the last line

### Requirement: The setup dialog is read-only and every closing key exits cleanly

The dialog SHALL offer no selection: `Enter`, `q`, and `Esc` SHALL each
close it — the pane process exits 0, the popup auto-closes, no
`plugin pane open` call is made, and no file is written anywhere. There are
no movement keys and no highlighted row.

#### Scenario: Cancel exits cleanly

- **WHEN** the user presses `q`, `Enter`, or `Esc`
- **THEN** no herdr call is made, no file is written, and the process
  exits 0

#### Scenario: Escape cancels

- **WHEN** a lone Escape keypress arrives (terminal still open)
- **THEN** the dialog closes as if `q` was pressed; the key reader uses a
  short (~0.1 s) timeout, not a blocking read, so a lone Escape is
  distinguishable from a closed terminal

#### Scenario: Terminal is restored on every exit path

- **WHEN** the dialog exits by any closing key or by error
- **THEN** the saved stty settings and the visible cursor are restored
  before the process returns

### Requirement: The dialog frame layout matches herdr-open-in

The dialog SHALL draw the herdr-open-in dialog shape: no internal title
line, one blank line above the rows, the help footer pinned to the drawable
bottom, no trailing line break, and a per-frame terminal size re-read.

#### Scenario: Frame chrome carries no internal title

- **WHEN** any frame is drawn
- **THEN** the first content line is a blank line, no title text is
  rendered (the popup window chrome already shows the manifest pane
  title), and the frame's last byte is the footer padding with no
  trailing newline

#### Scenario: Footer is pinned to the drawable bottom

- **WHEN** the terminal is taller than the chrome plus rows
- **THEN** filler blank lines push the footer to the bottom-most line and
  the size is re-read via `stty size` on every redraw, with an 80×24
  fallback when it cannot be parsed

### Requirement: The dialog runs as a raw-mode popup TUI with theme colors

The dialog SHALL run a raw-mode TUI entered through an `stty` child
process, following the herdr-open-in pattern, with colors from a palette
resolved once from the herdr config theme (`$HERDR_CONFIG_PATH` else
`~/.config/herdr/config.toml`, built-ins), ANSI-16 fallback on any
resolution failure.

#### Scenario: Frame colors follow the herdr theme palette

- **WHEN** the dialog starts with a herdr config naming a built-in theme
- **THEN** the painted tokens (panel background, text, overlay1) come from
  that theme, and an unset, unreadable, invalid, or unknown theme name
  selects the ANSI-16 fallback
