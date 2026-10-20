# Review Tool Picker

## Purpose

The review-tool-picker capability lets the review action choose among supported review tools already installed on `PATH`. It provides a compact, borderless popup only when multiple tools are available and remembers the last successful selection.

## Requirements

### Requirement: The action chooses only among installed catalog tools

The action SHALL inspect the supported catalog (`lumen`, `revdiff`, `tuicr`) on `PATH`. It SHALL launch the sole installed tool directly, open the picker when multiple tools are installed, and route to the existing setup popup when none are installed.

#### Scenario: No catalog tools are installed

- **WHEN** the action finds no supported tool executable on `PATH`
- **THEN** it opens the existing setup popup with its install guidance and does not open a review pane

#### Scenario: One catalog tool is installed

- **WHEN** exactly one supported tool executable is found on `PATH`
- **THEN** the action opens the review pane directly with that tool id and does not open a picker

#### Scenario: Multiple catalog tools are installed

- **WHEN** two or more supported tool executables are found on `PATH`
- **THEN** the action opens the tool picker popup and does not open a review pane before selection

### Requirement: The picker supports selection and cancellation

The picker SHALL move the highlighted row with Up and Down, launch the highlighted tool on Enter, and close without launching on Esc.

#### Scenario: Arrow keys move the highlight and Enter launches

- **WHEN** the user moves the highlight with Up or Down and presses Enter
- **THEN** the review pane opens with the highlighted installed tool id and its tab receives focus after the picker popup closes

#### Scenario: Escape closes without launching

- **WHEN** the user presses Esc in the picker
- **THEN** the picker exits cleanly without opening a review pane

#### Scenario: Multiple tools are installed without interactive input

- **WHEN** the action has multiple installed tools but cannot read interactive input
- **THEN** it reports that selection is unavailable and exits without launching a review tool

### Requirement: The picker restores the last installed choice

The picker SHALL persist the last selected tool id in the plugin state directory. When opened, it SHALL highlight the stored id if that id is currently installed; a missing, unreadable, or unknown stored value SHALL fall back to the first installed tool and SHALL NOT prevent launch.

#### Scenario: Last choice remains installed

- **WHEN** the stored last choice is among the currently installed tools
- **THEN** that tool is highlighted when the picker opens

#### Scenario: Last choice is unavailable or cannot be read

- **WHEN** the stored value is missing, unreadable, unknown, or names a tool no longer installed
- **THEN** the first installed tool is highlighted and selection remains available

#### Scenario: State write failure does not prevent launch

- **WHEN** the user selects an installed tool and persisting that choice fails
- **THEN** the review pane still opens with the selected tool

### Requirement: The picker is compact, borderless, and palette independent

The picker SHALL render a compact list without a frame or border. The popup width SHALL fit the complete footer line without extra horizontal space. The selected row SHALL be highlighted across the full popup width. The picker SHALL use terminal-default colors and basic text attributes without reading the herdr theme configuration.

#### Scenario: Picker renders under any terminal palette

- **WHEN** the picker opens in any terminal color palette or light/dark mode
- **THEN** it uses terminal-default colors and basic text attributes, draws no border, sizes the popup to the footer line, and highlights the selected row across the full width

#### Scenario: The selected row fills the popup width

- **WHEN** the picker renders the selected tool
- **THEN** the highlight extends across the full content row, including trailing blank cells
