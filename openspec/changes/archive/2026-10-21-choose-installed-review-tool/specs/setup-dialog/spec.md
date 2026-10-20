## MODIFIED Requirements

### Requirement: The dialog shows one universal layout for every entry case

The dialog SHALL show the existing no-tool setup guidance when the action opens it because no supported review tool is installed. It SHALL retain the catalog and install commands while removing any instruction to configure a `tool` key. The dialog does not perform its own PATH detection.

#### Scenario: Every entry case renders the same dialog

- **WHEN** the setup popup opens because no supported catalog tool is installed
- **THEN** it renders the existing no-tool guidance, including the supported tool install information, without asking the user to set a `tool` configuration key

#### Scenario: Tool rows are alphabetical with dim inline install commands

- **WHEN** the setup dialog renders the install guidance
- **THEN** the rows appear in alphabetical order — `lumen`, `revdiff`, `tuicr` — each with its existing dim inline install command

#### Scenario: Repo URL line closes the content

- **WHEN** the setup dialog renders
- **THEN** `For more information:` followed by `https://github.com/alexeyco/herdr-review` is the last content block before the footer

#### Scenario: Config path points at the resolved file

- **WHEN** the config directory resolves through `HERDR_PLUGIN_CONFIG_DIR` or the `herdr plugin config-dir alexeyco.review` fallback
- **THEN** the printed path is `<config dir>/config.toml`, shown even when the file does not exist yet

#### Scenario: Unresolvable config directory still renders

- **WHEN** no config directory can be resolved
- **THEN** the dialog prints the `herdr plugin config-dir alexeyco.review` command as the way to find the path, and otherwise renders the same no-tool guidance
