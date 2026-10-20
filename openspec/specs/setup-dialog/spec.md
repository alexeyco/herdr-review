# Setup Dialog

## Purpose

The setup-dialog capability is the read-only popup pane the review action
opens when no supported review tool is installed. It shows one universal
layout — the config file path, the static catalog with install commands,
and the repo URL — as guidance for installing a supported tool, and closes
on any key. It never branches on install or config state, never selects a
tool, never writes state, and never opens a review pane. The config path is
informational; users do not configure a tool there.

## Requirements

### Requirement: The dialog shows one universal layout for every entry case

The dialog SHALL render the same content whenever it is opened because no
catalog tool is installed: a `Config file:` line with the resolved config path,
an
`Install one of these review tools:` line with the three static catalog
rows, a `For more information:` line with the repository URL, and the
pinned footer. The guidance SHALL NOT ask users to configure a `tool` key.
No branch status line, no PATH state, and no greyed rows are rendered; the
dialog performs no PATH detection at all.

#### Scenario: Every entry case renders the same dialog

- **WHEN** the popup opens because no catalog tool is installed
- **THEN** the rendered content is identical in all of them apart from the
  config path value, no line states which case occurred, and no line asks
  the user to configure a `tool` key

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
with terminal-default background as the right padding. The manifest's `setup`
pane SHALL declare popup size keys fitted to the content without
scrolling: `width = "60%"` and `height = 15` (13 content lines plus a
two-line wrap margin for long config paths — live-probed clipping).

#### Scenario: Content carries top, left, and right padding

- **WHEN** any frame is drawn
- **THEN** the first line is blank, every content line starts with the
  two-column left padding, and terminal-default background fills the
  remaining drawable width

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

#### Scenario: Any key event closes the dialog

- **WHEN** a key event arrives while the terminal is open
- **THEN** the dialog closes without interpreting movement or selection keys

#### Scenario: Terminal is restored on every exit path

- **WHEN** the dialog exits by any closing key or by error
- **THEN** the terminal state and visible cursor are restored before the
  process returns

### Requirement: The dialog frame keeps the content compact and the footer visible

The dialog SHALL draw no internal title line, place one blank line above
the rows, and pin the help footer to the drawable bottom. It SHALL use the
terminal backend's current frame size for each redraw.

#### Scenario: Frame chrome carries no internal title

- **WHEN** any frame is drawn
- **THEN** the first content line is a blank line, no title text is
  rendered (the popup window chrome already shows the manifest pane
  title)

#### Scenario: Footer is pinned to the drawable bottom

- **WHEN** the terminal is taller than the chrome plus rows
- **THEN** filler blank lines push the footer to the bottom-most line and
  the current frame size is used on every redraw

### Requirement: The dialog runs as a raw-mode popup TUI with terminal-default colors

The dialog SHALL use a terminal TUI with terminal-default foreground and
background colors and basic text attributes. It SHALL NOT read or parse
the herdr theme configuration. When stdin is non-interactive, it SHALL
print the same guidance as plain text and read one byte before exiting.

#### Scenario: Dialog uses terminal-default colors without reading the herdr theme

- **WHEN** the dialog starts under any terminal palette or light/dark mode
- **THEN** it uses terminal-default colors and basic text attributes without
  reading or parsing the herdr theme configuration
