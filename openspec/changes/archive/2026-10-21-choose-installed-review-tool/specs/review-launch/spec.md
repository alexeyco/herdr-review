## ADDED Requirements

### Requirement: The action chooses a review tool from the installed catalog

The action SHALL inspect supported tool executables on `PATH`. It SHALL open the review pane directly with the sole installed tool, open the picker popup when multiple tools are installed, and open no review pane before the picker selection.

#### Scenario: No catalog tools are installed

- **WHEN** no supported tool executable is found on `PATH`
- **THEN** the action routes to the setup popup and does not open a review pane

#### Scenario: Exactly one catalog tool is installed

- **WHEN** exactly one supported tool executable is found on `PATH`
- **THEN** the action opens the review pane directly with that tool id and opens no picker

#### Scenario: Multiple catalog tools are installed

- **WHEN** multiple supported tool executables are found on `PATH`
- **THEN** the action opens the picker popup and waits for selection before opening a review pane

### Requirement: The action opens the setup popup when no supported tool is installed

When no supported review tool is installed, the action SHALL open the existing setup popup using `HERDR_BIN_PATH` with `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`, without `--workspace`, `--cwd`, or `--env` arguments. The existing no-tool guidance is retained, and success is determined by the herdr exit status.

#### Scenario: No supported tool is installed

- **WHEN** the PATH check finds none of the supported catalog tools
- **THEN** the setup popup opens with the existing no-tool guidance and the action does not open a review pane

#### Scenario: Setup-open argv carries no forwarding

- **WHEN** the action opens the setup popup
- **THEN** the argv is exactly `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`

#### Scenario: Popup open succeeds without a pane id in the reply

- **WHEN** the herdr CLI exits 0 for the setup popup open and the reply JSON carries no pane id
- **THEN** the action succeeds and does not treat the missing pane id as an error

## REMOVED Requirements

### Requirement: The action decides between the direct open and the setup dialog

The action SHALL read the plugin's `config.toml` once (placement-config capability) and check the configured `tool` id against `PATH` with reviewer-execution's executable-file rules. When the id is valid and its binary is detected, the action SHALL open the review pane directly and no dialog is shown — this is the main path. Otherwise (unset, blank, invalid and degraded to unset, or missing from `PATH`) the action SHALL open the setup popup instead and SHALL NOT open a review pane.

#### Scenario: Configured installed tool opens the review pane directly

- **WHEN** `tool = "revdiff"` is configured and an executable `revdiff` is on `PATH`
- **THEN** the action opens the review pane with `--env HERDR_REVIEW_TOOL=revdiff` and opens no popup

#### Scenario: Unset tool opens the setup dialog

- **WHEN** the `tool` key is unset or blank
- **THEN** the action opens the setup popup and no review pane

#### Scenario: Configured missing tool opens the setup dialog

- **WHEN** `tool = "lumen"` is configured and no executable `lumen` is on `PATH`
- **THEN** the action opens the setup popup and no review pane

#### Scenario: Invalid tool value opens the setup dialog after a warning

- **WHEN** `tool = "diffy"` is configured
- **THEN** the config layer warns and treats the key as unset, and the action opens the setup popup

### Requirement: The action opens the setup popup when no usable tool is configured

The setup popup SHALL be opened by invoking the herdr binary at `HERDR_BIN_PATH` with `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus` — no `--workspace` flag, no `--cwd`, and no `--env` pairs: the dialog needs no forwarding and resolves the config path itself. Success is the herdr exit status alone.

#### Scenario: Setup-open argv carries no forwarding

- **WHEN** the action opens the setup popup
- **THEN** the argv is exactly `plugin pane open --plugin alexeyco.review --entrypoint setup --placement popup --focus`

#### Scenario: Popup open succeeds without a pane id in the reply

- **WHEN** the herdr CLI exits 0 for the setup popup open and the reply JSON carries no pane id
- **THEN** the action succeeds and does not treat the missing pane id as an error
