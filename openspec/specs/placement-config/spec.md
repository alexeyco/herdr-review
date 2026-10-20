# Placement Configuration

## Purpose

The placement-config capability reads the plugin's `config.toml` — the
flat-key file pattern of sibling plugin herdr-gitui — to decide where the
review pane opens. The action reads it once per launch, before opening a
pane, degrades every failure to documented defaults with warnings, and
refuses the `popup` placement for the review pane. Tool selection uses the
installed catalog and does not read the obsolete `tool` key.

## Requirements

### Requirement: The config file is read once per action launch from the plugin config directory

The action SHALL read `<config dir>/config.toml` exactly once per launch, after context parsing and before any pane open, to resolve placement and direction only. A non-blank `HERDR_PLUGIN_CONFIG_DIR` wins outright; otherwise `herdr plugin config-dir alexeyco.review` via `HERDR_BIN_PATH` resolves the directory. Every failure degrades to `placement = "tab"` and `direction = "right"` with a warning, never failing the launch; unknown keys, including the obsolete `tool` key, are ignored. Tool selection is based only on installed tools detected on `PATH`.

#### Scenario: Environment directory wins

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is set to a non-blank path containing a valid `config.toml`
- **THEN** the file is read from that directory and no `config-dir` CLI call is made

#### Scenario: CLI fallback resolves the directory

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is unset and `herdr plugin config-dir alexeyco.review` prints a directory
- **THEN** `config.toml` is read from the printed directory for placement and direction

#### Scenario: No config directory available warns and uses defaults

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is unset or blank and the `config-dir` call fails
- **THEN** the action warns and proceeds with default placement and direction while still selecting from installed tools

#### Scenario: Missing file uses defaults

- **WHEN** the resolved directory contains no `config.toml`
- **THEN** the launch proceeds with `tab` placement and `right` direction, warns once, and selects from installed tools

#### Scenario: Corrupt file warns and uses defaults

- **WHEN** `config.toml` content is not valid TOML
- **THEN** a warning names the parse problem, defaults are used, and launch proceeds to installed-tool selection

#### Scenario: Missing or invalid configuration preserves launch defaults

- **WHEN** the config directory is unavailable, the file is missing, or its content is invalid
- **THEN** the action warns, uses default placement and direction, and still chooses among installed tools

#### Scenario: Obsolete tool key is ignored

- **WHEN** `config.toml` has no `tool` key or has an arbitrary value for that key
- **THEN** the action selects based on the tools detected on `PATH` without a tool-configuration warning

### Requirement: The placement key selects the review pane placement and popup is refused

The `placement` key SHALL accept `tab`, `split`, `overlay`, and `zoomed`,
each passed verbatim as the `--placement` value of the review-pane open.
The value `popup` SHALL be refused with a warning and `tab` used instead —
the review pane is a workspace pane, and `popup` is not among the
placements `herdr plugin pane open --help` documents; any other invalid
value SHALL warn and fall back to `tab`.

#### Scenario: Each accepted placement maps to argv

- **WHEN** `placement` is `overlay`
- **THEN** the review-pane open argv carries `--placement overlay`

#### Scenario: Popup falls back to tab with a warning

- **WHEN** `placement = "popup"` is configured
- **THEN** the action prints `herdr-review: warning: placement "popup" is
not supported for the review pane; using "tab"` and the argv carries
  `--placement tab`

#### Scenario: Unknown placement value falls back

- **WHEN** `placement` is `floating`
- **THEN** a warning naming `floating` is printed and `tab` is used

### Requirement: The direction key applies only to split placement

The `direction` key SHALL accept `right`, `down`, and `auto`; `right` and
`down` are passed as `--direction <value>` only when placement is `split`
(the herdr CLI accepts exactly these two values). The value `auto`, an unset key, and any
placement other than `split` SHALL result in no `--direction` flag. An
invalid value SHALL print a warning naming it and be treated as `auto` for
that launch.

#### Scenario: Split with direction

- **WHEN** `placement = "split"` and `direction = "down"`
- **THEN** the argv carries `--placement split --direction down`

#### Scenario: Auto omits the flag

- **WHEN** direction is `auto` or placement is not `split`
- **THEN** the argv preserves the selected placement and carries no `--direction` flag

#### Scenario: Direction ignored without split

- **WHEN** `placement = "tab"` and `direction = "down"`
- **THEN** the argv carries `--placement tab`, no `--direction` flag, and
  no warning

#### Scenario: Invalid direction warns and omits the flag

- **WHEN** `placement = "split"` and `direction = "left"`
- **THEN** a warning naming `left` is printed and no `--direction` flag is
  passed
