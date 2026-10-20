# Tasks

## 1. Discover installed tools and route action launches

- [x] 1.1 Expose the installed catalog ids from `src/adapters/tool.rs` to the action, preserving PATH-only executable resolution and the existing canonical order.
- [x] 1.2 Update `src/settings.rs` and `src/features/review/mod.rs` so the action reads only placement and direction, launches one installed tool directly (`Exactly one catalog tool is installed`, `One catalog tool is installed`), and opens setup when none are installed (`No catalog tools are installed`, `No supported tool is installed`).
- [x] 1.3 Add a dedicated picker popup entrypoint in `src/main.rs` and `herdr-plugin.toml`; route multiple installed tools to it (`Several catalog tools are installed`, `Multiple catalog tools are installed`) while preserving placement/direction for review-pane opens.

## 2. Implement compact tool picker and remembered choice

- [x] 2.1 Add `src/features/picker.rs` with a compact borderless list, Up/Down highlight movement, Enter selection, Esc cancellation, and terminal-default styling (`Arrow keys move the highlight and Enter launches`, `Escape closes without launching`, `Picker renders under any terminal palette`).
- [x] 2.4 Size the picker popup to the footer line and extend the selected-row highlight across the full popup width (`The selected row fills the popup width`, `Picker renders under any terminal palette`).
- [x] 2.5 Re-focus the newly opened review tab after closing the picker popup (`Arrow keys move the highlight and Enter launches`).
- [x] 2.2 Persist the last selected tool id under plugin state and restore it as the highlight only when it remains installed; otherwise highlight the first installed tool. State failures must not prevent launch (`Last choice remains installed`, `Last choice is unavailable or cannot be read`).
- [x] 2.3 Ensure a non-interactive picker invocation reports selection unavailable and exits without launching a review tool (`Multiple tools are installed without interactive input`).

## 3. Remove theme parsing from dialogs

- [x] 3.1 Move the setup dialog to `src/features/setup.rs` and remove its theme module; use terminal-default colors and basic text attributes (`Dialog uses terminal-default colors without reading the herdr theme`).
- [x] 3.2 Keep the existing zero-tool setup guidance and close behavior while ensuring it no longer asks users to configure a `tool` key (`Every entry case renders the same dialog`, `Tool rows are alphabetical with dim inline install commands`, `Repo URL line closes the content`, `Config path points at the resolved file`, `Unresolvable config directory still renders`).

## 4. Update launch and configuration specifications

- [x] 4.1 Remove config-selected launch behavior from `openspec/specs/review-launch/spec.md` and add requirements `The action chooses a review tool from the installed catalog` and `The action opens the setup popup when no supported tool is installed` (`No catalog tools are installed`, `Exactly one catalog tool is installed`, `Multiple catalog tools are installed`, `No supported tool is installed`, `Setup-open argv carries no forwarding`, `Popup open succeeds without a pane id in the reply`).
- [x] 4.2 Modify `openspec/specs/placement-config/spec.md` requirement `The config file is read once per action launch from the plugin config directory` to cover placement and direction only, ignore the obsolete tool key, and remove the old requirement `The tool key names the review tool` (`Environment directory wins`, `CLI fallback resolves the directory`, `No config directory available warns and uses defaults`, `Missing file uses defaults`, `Corrupt file warns and uses defaults`, `Missing or invalid configuration preserves launch defaults`, `Obsolete tool key is ignored`, `Each accepted placement maps to argv`, `Unknown placement value falls back`, `Popup falls back to tab with a warning`, `Split with direction`, `Auto omits the flag`, `Direction ignored without split`, `Invalid direction warns and omits the flag`).
- [x] 4.3 Modify `openspec/specs/setup-dialog/spec.md` requirements `The dialog shows one universal layout for every entry case` and `The dialog runs as a raw-mode popup TUI with theme colors` to describe retained no-tool guidance without tool configuration and theme-independent terminal-default styling.
- [x] 4.4 Add `openspec/changes/choose-installed-review-tool/specs/review-tool-picker/spec.md` requirements for count-based selection, picker keyboard behavior, remembered choice, and borderless safe rendering.

## 5. Cover acceptance scenarios

- [x] 5.1 Update `tests/e2e.rs` for zero, one, and multiple installed tools, chosen-tool forwarding, and selection cancellation.
- [x] 5.2 Cover last-choice restoration and fallback (`Last choice remains installed`, `Last choice is unavailable or cannot be read`), verify state write failures still launch the selected tool (`State write failure does not prevent launch`), verify non-interactive invocation exits without launching (`Multiple tools are installed without interactive input`), preserve placement/direction, and confirm dialogs do not read theme configuration.
