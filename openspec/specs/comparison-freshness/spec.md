# Comparison Freshness

## Purpose

The comparison-freshness capability is the monthly mechanism that keeps
the README `## Comparison` section's machine-checkable facts true: a
Node-builtins-only script that verifies competitor release tags, repo
existence, and link liveness against the GitHub API, and a workflow that
routes the outcome — a rolling dual-update fix PR for machine-fixable
drift, one deduped issue for drift a script must not rewrite, a red run
for unreachable sources, and a job-summary audit trail on every run.

## Requirements

### Requirement: Expectations live in a tracked data file

The checker SHALL read expected releases from `EXPECT_REPOS`, a comma-separated list of `<owner>/<repo>@<tag>` entries. Reject malformed or missing values as UNAVAILABLE. Read the README once for URL liveness and release-cell presence; never derive expectations from its text. The workflow loads the tracked data file into the environment, and `--fix` updates it with the README.

#### Scenario: Env entry yields a release expectation

- **WHEN** `EXPECT_REPOS` contains `jhochenbaum/herdr-hunk-diff@v0.5.0`
- **THEN** the script GETs
  `<github-api-base>/repos/jhochenbaum/herdr-hunk-diff/releases/latest`
  and expects `.tag_name` = `v0.5.0`, both values taken from that entry —
  a fixture API answering `v0.5.0` makes the run clean, any other tag
  makes it DRIFT

#### Scenario: Malformed env entry is loud, never skipped

- **WHEN** an `EXPECT_REPOS` entry reads `jhochenbaum/herdr-hunk-diff`
  (no `@`), or `skellleks@v0.1.0` (owner/repo side not two segments), or
  `quantk/herdr-review@` (empty tag), or the variable is unset or empty
- **THEN** the script exits 2 and prints a `comparison-check:` `✗` line
  naming the variable and the offending entry; it never exits 0 or 1
  having skipped it

#### Scenario: The workflow loads expectations and maps the token

- **WHEN** the job-level `env` block of
  `.github/workflows/comparison-freshness.yml` is read
- **THEN** the workflow loads `EXPECT_REPOS` from
  `.github/comparison-expected-repos.txt` and maps
  `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`

#### Scenario: Presence checks are substring, not structural

- **WHEN** a fixture README's table row is reformatted — spacing or cell
  boundaries changed — but the substring
  `github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0` still
  appears in the file
- **THEN** the presence check for that entry passes — layout-insensitive,
  no structure parsed; only a missing substring turns it into DRIFT

### Requirement: Three fact classes are verified against live sources

The script SHALL verify release tags, README URL liveness, and release-cell presence against GitHub. Use the configurable API base, optional bearer token, 10-second timeouts, and one retry after 2 seconds for transport errors, 429, or 5xx. Classify drift as machine-fixable or issue-only and unreachable sources as UNAVAILABLE; scenarios define individual status handling.

#### Scenario: A tag mismatch is machine-fixable drift

- **WHEN** `EXPECT_REPOS` claims `jhochenbaum/herdr-hunk-diff@v0.5.0` and
  the fixture API answers `"tag_name": "v0.6.0"`
- **THEN** the run exits 1 and the DRIFT finding names the env entry,
  expected `v0.5.0`, found `v0.6.0`, class `release-drift`, and the
  source URL

#### Scenario: A matching tag verifies clean

- **WHEN** every entry's `releases/latest` answers exactly the expected
  `tag_name`, every README URL is alive, and every presence substring is
  found
- **THEN** the run produces no finding and exits 0

#### Scenario: A 404 release endpoint is issue-only drift

- **WHEN** an entry's `releases/latest` GET answers 404 — repo gone,
  private, or releases dropped
- **THEN** the finding is DRIFT class `repo-gone` (issue-only, written to
  the `--report` body), never machine-fixable, and no existence GET is
  issued

#### Scenario: A rate-limit 403 is unavailable, not drift

- **WHEN** a `releases/latest` GET or a liveness HEAD answers 403 (rate
  limit or bot challenge), after the single retry where applicable
- **THEN** the finding classifies UNAVAILABLE and the run exits 2 — never
  DRIFT

#### Scenario: A dead link is issue-only drift

- **WHEN** an `https:` URL in the README answers HEAD with a final 404
  after redirects
- **THEN** the run exits 1 with a `dead-link` DRIFT finding naming the
  URL, written to the `--report` body

#### Scenario: A moved URL is issue-only drift

- **WHEN** a liveness HEAD ends 2xx at a final URL that differs from the
  requested URL after stripping one trailing `/` from both sides — the
  comparison basis is the full URL, so a trailing-slash-only difference
  is tolerated and nothing else is
- **THEN** the run exits 1 with a `moved-url` DRIFT finding naming both
  URLs, written to the `--report` body — the script never rewrites the
  link itself

#### Scenario: A stale release cell is machine-fixable presence drift

- **WHEN** the README lacks
  `github.com/quantk/herdr-review/releases/tag/v0.2.7` but still contains
  the prefix `github.com/quantk/herdr-review/releases/tag/`
- **THEN** the finding is DRIFT class `stale-cell` (machine-fixable), not
  written to the `--report` body

#### Scenario: A missing release cell is issue-only presence drift

- **WHEN** the README lacks even the tagless prefix
  `github.com/quantk/herdr-review/releases/tag/`
- **THEN** the finding is DRIFT class `missing-cell` (issue-only),
  written to the `--report` body naming the entry

### Requirement: Exit codes separate clean, drift, and unavailable

Prefix output with `comparison-check:`. Exit 0 for clean results, 1 for drift, and 2 when any source is unavailable (which takes precedence). Reports contain issue-only drift; issue mode creates or updates the deduplicated issue through the REST API. Check-only flags are ignored by fix and issue modes; the script SHALL NOT use `gh`.

#### Scenario: Clean run exits zero

- **WHEN** every fixture endpoint answers the env expectations and every
  README URL is alive
- **THEN** the script exits 0 and stdout ends with `comparison-check: ok`

#### Scenario: Drift-only run exits one

- **WHEN** the run produces only DRIFT findings, of either routing class
- **THEN** the script exits 1 and each printed finding carries class,
  subject, expected, found, and source

#### Scenario: Unreachable source exits two

- **WHEN** `COMPARISON_GITHUB_API` points at a closed TCP port
  (connection refused, still refused after the single retry)
- **THEN** the script exits 2 with a `✗` line naming the unreachable
  source URL — never exit 0 or 1

#### Scenario: Mixed findings exit two

- **WHEN** one source yields DRIFT and another yields UNAVAILABLE in the
  same run
- **THEN** the script exits 2, the output prints both classes, and the
  workflow opens no PR and files no issue

#### Scenario: Machine-fixable-only drift writes no report

- **WHEN** the only findings are `release-drift` or `stale-cell` and
  `--report <path>` was passed
- **THEN** the script exits 1 and `<path>` does not exist — the workflow's
  issue step, guarded on the report file, stays silent and the fix PR is
  the only alarm

### Requirement: Fix mode dual-updates the README and expectations file

Fix mode SHALL fetch each expected repository's latest release and update its README cell and expectations entry together. Compute changes before writing. Skip 404 responses and missing cells; abort without writing when a source is unavailable or the expectations file is missing. Do not change matrix cells, footnotes, the audit date, or other files.

#### Scenario: Fix updates the README cell and expectations together

- **WHEN** the fixture API answers `v0.6.0` for an entry pinned `v0.5.0`
  in both the fixture README's release cell and the fixture expectations
  file, and `--fix` runs
- **THEN** after the run both files carry `v0.6.0` — the README cell
  reads
  `[v0.6.0](https://github.com/<owner>/<repo>/releases/tag/v0.6.0)` and
  the expectations entry reads `<owner>/<repo>@v0.6.0` — and the exit code is 0

#### Scenario: Fix never touches matrix cells or the checked-date line

- **WHEN** `--fix` rewrites a drifted release tag
- **THEN** every other byte of the README is unchanged: the ✅/⚠️/❌
  matrix cells, the footnotes, and the line
  `All cells checked 2026-10-18 against the linked public sources.`
  survive verbatim — the bot never bumps the human-audit date

#### Scenario: Fix skips a 404 entry and converges the others

- **WHEN** one entry's `releases/latest` answers 404 and another's
  answers a tag newer than pinned
- **THEN** the fixer leaves the 404 entry's README cell and env segment
  untouched, dual-updates the other entry, and exits 0

#### Scenario: Fix skips an unmatchable cell and the run stays green

- **WHEN** an entry's `releases/latest` answers 200 but its anchored cell
  shape matches nowhere in the README — the state check mode classified
  `missing-cell` and wrote to the report
- **THEN** the fixer prints a `comparison-check:` skip line, leaves that
  entry's README and env segment untouched, converges the remaining
  entries, and exits 0 — the workflow run stays green and the
  report-guarded issue step files the alarm

#### Scenario: Fix is atomic when a live source is unreachable

- **WHEN** any entry's `releases/latest` GET fails on transport or a
  non-404 non-2xx status
- **THEN** `--fix` exits 2 and both the README and expectations file are
  byte-identical to their pre-run state — no partial dual-update

#### Scenario: A missing expectations file is loud

- **WHEN** `--fix` runs when the expectations file is missing
- **THEN** it exits 2 with a `comparison-check:` `✗` line naming the
  file, and writes nothing

### Requirement: A monthly workflow routes drift to a rolling fix PR and one deduped issue

The workflow SHALL run monthly and on manual dispatch, with concurrent runs serialized. Check captures the script's exit code: 0 finishes green, 1 runs the fixer, formats the README, and opens or updates a rolling PR; it files an issue only for issue-only findings. Code 2 fails without a PR or issue. The bot does not update master directly.

#### Scenario: Machine drift opens the rolling fix PR and the run stays green

- **WHEN** a run's check exits 1 with machine-fixable findings and the
  fixer dual-updates README and the expectations file
- **THEN** the workflow formats the README with prettier@3.9.6, commits
  to branch `chore/comparison-freshness`, opens (or updates, if one is
  already open) a PR against `master` with `delete-branch: true`, files
  no issue, and the run concludes green

#### Scenario: Semantic drift files one issue and the run stays green

- **WHEN** a run's check exits 1 having written `drift-report.md` with
  issue-only findings (`repo-gone`, `dead-link`, `moved-url`,
  `missing-cell`) and no open `comparison-drift` issue exists
- **THEN** the issue step POSTs an issue titled `README comparison drift`
  with label `comparison-drift` carrying the findings table as body, and
  the run concludes green

#### Scenario: Repeat drift updates the same issue

- **WHEN** a later run exits 1 with issue-only findings and the dedup GET
  `issues?labels=comparison-drift&state=open` returns an open issue
- **THEN** the script PATCHes that issue's body with the fresh findings
  and creates no second issue

#### Scenario: Issue creation ensures the label first

- **WHEN** issue mode runs with no open `comparison-drift` issue and the
  label POST answers 422 already-exists (or 201 created)
- **THEN** the script proceeds to `POST /issues` with
  `labels: ["comparison-drift"]` and exits 0, the request log showing the
  label POST before the issue POST; a label POST failing with any other
  non-2xx exits 2 and creates no issue

#### Scenario: Unavailable run goes red without PR or issue

- **WHEN** a run's check exits 2 — an unreachable source, a 403 rate
  limit, or a malformed env contract
- **THEN** the job concludes failed (red), and neither the fix/PR steps
  nor the issue step runs, regardless of partial findings

#### Scenario: Unknown exit code fails the run

- **WHEN** the check step captures an exit code outside {0, 1, 2} — e.g.
  127 (`node` missing) or 137 (process killed)
- **THEN** the step prints a `comparison-check:`-prefixed line and exits
  with that code, reddening the run, and no conditional step's
  `code == '1'`/`code == '2'` guard matches — no PR, no issue

#### Scenario: The fix PR passes the prettier gate

- **WHEN** the fix PR's commit is created
- **THEN** the committed `README.md` satisfies
  `npx --yes prettier@3.9.6 --check` — the workflow ran `--write` between
  the fixer and the commit — verified locally in the fix flow and
  enforced by the ci.yml:55 md gate on the master push after merge
  (`pull_request` workflows do not fire for PRs opened with the default
  `GITHUB_TOKEN`)

#### Scenario: Push and PR gates stay byte-identical

- **WHEN** a contributor runs `make check` or pushes a PR after this
  change merges
- **THEN** `Makefile` and `.github/workflows/ci.yml` are byte-identical
  to their pre-change state (`git diff` on both is empty), `make check`
  still runs exactly lint, fmt-check, test, build, and
  `scripts/check-comparison.mjs` appears in no gate

### Requirement: Every check run leaves a job-summary audit trail

When given `--summary`, check mode SHALL append a Markdown audit for clean, drift, and unavailable results. Include UTC time, expectations, per-class rows, and the final classification. Do not include credentials or environment values other than `EXPECT_REPOS`. A write failure is UNAVAILABLE; fix and issue modes ignore this flag.

#### Scenario: A clean run's summary lists every check with verdicts

- **WHEN** every fixture endpoint answers the env expectations and check
  mode runs with `--summary <path>`
- **THEN** the script exits 0 with stdout unchanged
  (`comparison-check: ok`), and `<path>` holds the checked-at UTC
  timestamp, the `EXPECT_REPOS` value as given, all three class tables
  with a row per item, and the classification line reading `clean` with
  zero drift and zero unavailable counts

#### Scenario: An unavailable run still writes its audit summary

- **WHEN** a source is unreachable — the API seam points at a closed
  port, a domain answers 403, or `EXPECT_REPOS` is malformed — and
  `--summary <path>` was passed
- **THEN** the run exits 2 (red in the workflow) and `<path>` still
  exists: affected rows carry the fetch error, status, or malformed value
  with verdict `unavailable`, a class whose entries all failed validation
  shows its no-checks-ran line, and the classification line reads
  `unavailable` — the audit survives the red run

#### Scenario: The summary never contains credentials

- **WHEN** check mode runs with `GITHUB_TOKEN` set to a fake value —
  exercised against the fixture server so the token-bearing GET really
  happens — and `--summary <path>`
- **THEN** the written file contains neither the token value nor any
  environment variable beyond `EXPECT_REPOS` as given

#### Scenario: Without --summary nothing changes

- **WHEN** check mode runs without the flag, on any classification
- **THEN** no summary file is created and stdout and the exit code are
  byte-identical to the flagless contract — `comparison-check: ok`, the
  `✗` finding shapes, and the 0/1/2 exits hold verbatim
