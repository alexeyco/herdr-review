# Proposal: Choose an Installed Review Tool

## Why

Users currently select a review tool by editing `tool` in `config.toml`. This makes a routine choice a configuration task and prevents the action from adapting to which supported tools are installed.

## What Changes

- The action discovers installed catalog tools on `PATH`: it launches the sole match directly, opens a picker for multiple matches, and retains the current no-tool setup message when there are no matches.
- The picker is a compact popup without a border. Up and Down move the highlight, Enter launches the highlighted tool, and Esc closes without launching. The most recent choice is highlighted when it remains installed; otherwise the first installed tool is highlighted.
- If multiple tools are installed but no interactive terminal is available, the action reports that selection is unavailable and exits without launching a tool.
- Tool selection configuration and herdr theme parsing are removed. Placement and direction remain configurable. Dialogs use terminal-default colors and basic text attributes without reading herdr theme configuration, so they work across palettes and light or dark modes.

## Capabilities

### Modified Capabilities

- `review-launch`: select a tool from the installed catalog and open the picker only when more than one is installed.
- `placement-config`: keep placement and direction settings while removing the tool setting and its configuration requirement.
- `setup-dialog`: keep the zero-tool setup popup and remove theme parsing and bordered/palette-specific drawing.

### New Capabilities

- `review-tool-picker`: render the compact borderless picker, handle its keys, and remember the last choice.

## Impact

The action, tool discovery, configuration, and popup rendering/state code need changes. The selected tool continues to reach the review pane through `HERDR_REVIEW_TOOL`. Review execution, annotation delivery, and other review scenarios remain out of scope.

ASSUMPTION: persist the last tool id in the plugin state directory.

ASSUMPTION: if the remembered tool id is missing, unknown, unreadable, or no longer installed, highlight the first installed tool.

ASSUMPTION: storage read or write failures do not prevent launch.

REJECTED-RISK: a borderless popup may have less visual separation; the user explicitly requires no frame.
