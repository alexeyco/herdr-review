# Security Policy

## Reporting a vulnerability

Use GitHub private vulnerability reporting:
<https://github.com/alexeyco/herdr-review/security/advisories/new>.

Do not open a public issue for a security problem. Include a description, the
affected version, and a minimal reproduction if you have one. Expect an initial
response within a week.

## Supported versions

Only the latest tagged release is supported. Fixes land on `master` and ship in
the next release.

## Scope

This policy covers this plugin's own code. It executes external binaries:

- `HERDR_BIN_PATH`, set by herdr; every callback runs through it.
- The selected review tool (`revdiff`, `tuicr`, or `lumen`), resolved from
  `PATH` in the herdr daemon environment.
- `git`, for the read-only scope probes.

All of them run with your privileges, so treat them as trusted input. Report
bugs in herdr or the review tools to their upstream projects — this plugin is
not responsible for their behavior.
