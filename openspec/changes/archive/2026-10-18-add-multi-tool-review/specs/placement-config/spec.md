# Spec Delta

## Purpose

The placement-config capability reads the plugin's `config.toml` — the
flat-key file pattern of sibling plugin herdr-gitui — to decide which
review tool runs and where the review pane opens. The action reads it once
per launch, before deciding between the direct review-pane open and the
setup dialog, degrades every failure to documented defaults with warnings,
and refuses the `popup` placement for the review pane.

## ADDED Requirements

### Requirement: The config file is read once per action launch from the plugin config directory

The action SHALL read `<config dir>/config.toml` exactly once per launch,
after context parsing and before any pane open: a non-blank
`HERDR_PLUGIN_CONFIG_DIR` wins outright, otherwise `herdr plugin
config-dir alexeyco.review` via `HERDR_BIN_PATH` resolves the directory. Every failure degrades
to the defaults `placement = "tab"`, `direction = "right"`, `tool` unset,
with one warning each, never failing the launch; unknown keys are ignored.

#### Scenario: Environment directory wins

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is set to a non-blank path containing a
  valid `config.toml`
- **THEN** the file is read from that directory and no `config-dir` CLI
  call is made

#### Scenario: CLI fallback resolves the directory

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is unset and `herdr plugin config-dir
alexeyco.review` prints a directory
- **THEN** `config.toml` is read from the printed directory

#### Scenario: No config directory available warns and uses defaults

- **WHEN** `HERDR_PLUGIN_CONFIG_DIR` is unset or blank and the `config-dir`
  call fails
- **THEN** the action prints `herdr-review: warning: config directory
unavailable: <reason>; using default settings` and proceeds with
  defaults — `tool` unset, so the launch continues to the setup dialog

#### Scenario: Missing file uses defaults

- **WHEN** the resolved directory contains no `config.toml`
- **THEN** the launch proceeds with `tab` placement, `right` direction,
  `tool` unset, and one warning line

#### Scenario: Corrupt file warns and uses defaults

- **WHEN** `config.toml` content is not valid TOML
- **THEN** a warning naming the parse problem is printed, defaults are
  used, and the launch proceeds to the setup dialog instead of failing

### Requirement: The tool key names the review tool

The `tool` key SHALL accept exactly the catalog ids `revdiff`, `tuicr`,
and `lumen`; a valid value selects the review tool the action checks
against `PATH` for the direct open. An unset or blank value leaves the tool
unset, which routes the launch to the setup dialog. Any other value SHALL
print a `herdr-review: warning:` naming it and SHALL be treated as unset.

#### Scenario: Valid tool id is taken as configured

- **WHEN** `tool = "tuicr"` is set in `config.toml`
- **THEN** the action's PATH check runs for `tuicr` and, on a hit, the
  review pane opens with `--env HERDR_REVIEW_TOOL=tuicr`

#### Scenario: Invalid tool value warns and counts as unset

- **WHEN** `tool = "diffy"` is set
- **THEN** a warning naming `diffy` is printed and the tool counts as
  unset, routing the launch to the setup dialog

#### Scenario: Blank tool value counts as unset

- **WHEN** `tool` is set to an empty or whitespace-only string
- **THEN** the tool counts as unset with no warning

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

The `direction` key SHALL accept `right` and `down`, passed as
`--direction <value>` only when placement is `split` (the herdr CLI accepts
exactly these two values). The value `auto`, an unset key, and any
placement other than `split` SHALL result in no `--direction` flag. An
invalid value SHALL print a warning naming it and be treated as `auto` for
that launch.

#### Scenario: Split with direction

- **WHEN** `placement = "split"` and `direction = "down"`
- **THEN** the argv carries `--placement split --direction down`

#### Scenario: Auto omits the flag

- **WHEN** `placement = "split"` and `direction = "auto"`
- **THEN** the argv carries `--placement split` and no `--direction` flag

#### Scenario: Direction ignored without split

- **WHEN** `placement = "tab"` and `direction = "down"`
- **THEN** the argv carries `--placement tab`, no `--direction` flag, and
  no warning

#### Scenario: Invalid direction warns and omits the flag

- **WHEN** `placement = "split"` and `direction = "left"`
- **THEN** a warning naming `left` is printed and no `--direction` flag is
  passed
