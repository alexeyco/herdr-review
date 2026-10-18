# Spec Delta

## Purpose

The comparison-freshness capability is the monthly mechanism that keeps
the README `## Comparison` section's machine-checkable facts true: a
Node-builtins-only script that verifies competitor release tags, repo
existence, and link liveness against the GitHub API, and a workflow that
routes the outcome — a rolling dual-update fix PR for machine-fixable
drift, one deduped issue for drift a script must not rewrite, a red run
for unreachable sources, and a job-summary audit trail on every run.

## ADDED Requirements

### Requirement: Expectations live in the workflow environment

The expected values `scripts/check-comparison.mjs` verifies SHALL come
from one environment variable — pinned by the job-level `env` block of
`.github/workflows/comparison-freshness.yml` on a single line — and never
from the comparison table's text:

- `EXPECT_REPOS`: a comma-separated list of `<owner>/<repo>@<tag>`
  entries. Each entry SHALL be split on the LAST `@`: everything before
  it is `owner/repo` (exactly two non-empty segments), everything after
  is the expected release `tag_name` (non-empty, otherwise unconstrained
  — tags are arbitrary strings). Whitespace around segments SHALL be
  trimmed. The list carries no self entry for `alexeyco/herdr-review`:
  the subject's release cell is deliberately unpinned, so there is no
  machine-checkable self release fact (the ci.yml version-sync job checks
  `Cargo.toml`↔`herdr-plugin.toml` consistency only).

A malformed env string SHALL never be skipped silently: an entry without
`@`, an `owner/repo` side that is not two non-empty segments, an empty
tag, an empty segment (doubled or trailing comma), or the variable unset
or empty SHALL each yield an UNAVAILABLE finding (exit 2) naming the
variable and, where one exists, the offending entry.

The README (default: the working-tree file at the repo root, overridable
with `--readme <path>`) SHALL be read exactly once — for the liveness URL
set and the presence substrings — and never parsed for structure: every
`http:`/`https:` URL anywhere in the whole file joins the deduplicated
link-liveness set, and the same single read supplies the text the
presence checks search. The tables' layout, cell boundaries, and footnote
grammar SHALL NOT be consulted; every expected value comes from the env
contract above. The workflow's job-level `env` block SHALL set
`EXPECT_REPOS` to the pinned expectation values alongside the
`GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` mapping. The exact values are
drift-maintenance data — updated by the `--fix` dual-update, not by hand
— so the values themselves are not spec.

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

#### Scenario: The workflow env block carries EXPECT_REPOS and the token mapping

- **WHEN** the job-level `env` block of
  `.github/workflows/comparison-freshness.yml` is read
- **THEN** it sets `EXPECT_REPOS` on one line in the format above,
  carries a dual-update comment, and maps
  `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`

#### Scenario: Presence checks are substring, not structural

- **WHEN** a fixture README's table row is reformatted — spacing or cell
  boundaries changed — but the substring
  `github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0` still
  appears in the file
- **THEN** the presence check for that entry passes — layout-insensitive,
  no structure parsed; only a missing substring turns it into DRIFT

### Requirement: Three fact classes are verified against live sources

The script SHALL verify exactly three fact classes and nothing else. All
GitHub API calls go to the base `https://api.github.com` (overridable via
env `COMPARISON_GITHUB_API`, the test seam), with an
`Authorization: Bearer <token>` header whenever `GITHUB_TOKEN` is set.
Each fetch attempt SHALL time out after 10 s (`AbortSignal.timeout`), and
a transport error, 429, or 5xx SHALL be retried exactly once after 2 s
before classifying UNAVAILABLE.

1. **Release tags (and repo existence as a byproduct).** For each
   `EXPECT_REPOS` entry, one GET
   `<github-api-base>/repos/<owner>/<repo>/releases/latest`. A 200 whose
   `.tag_name` differs from the entry's tag is DRIFT, class
   `release-drift`, machine-fixable (the `--fix` dual-update heals it); a
   matching tag is clean. A 404 — the repo disappeared, went private, or
   dropped releases — is DRIFT, class `repo-gone`, issue-only: there is
   no live tag to write, so no separate existence GET exists. Any other
   non-2xx (403 rate limit included) and any transport failure is
   UNAVAILABLE.
2. **Link liveness.** Every URL of the deduplicated liveness set receives
   one HEAD with redirects followed and no GET fallback: a final 2xx is
   alive; a final 404 is DRIFT, class `dead-link`, issue-only; a final
   URL that differs from the requested URL — after stripping one trailing
   `/` from both sides — is DRIFT, class `moved-url`, issue-only (the
   script must not rewrite links it does not understand); every other
   final status — including a 403 bot challenge or rate limit — and every
   transport failure is UNAVAILABLE.
3. **Release-cell presence.** For each `EXPECT_REPOS` entry, the README
   SHALL contain the case-sensitive substring
   `github.com/<owner>/<repo>/releases/tag/<tag>` with the entry's exact
   tag. On a miss the script SHALL probe the tagless prefix
   `github.com/<owner>/<repo>/releases/tag/`: prefix present means a
   release cell exists with a stale tag — DRIFT, class `stale-cell`,
   machine-fixable; prefix absent means the cell or its row was removed —
   DRIFT, class `missing-cell`, issue-only, because the fixer rewrites
   existing cells and cannot recreate a deleted row. A presence miss is
   never UNAVAILABLE — the file was readable; UNAVAILABLE stays reserved
   for an unreadable README (exit 2, after still writing the summary),
   env-format problems, and unreachable sources.

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

The script SHALL print every line with the prefix `comparison-check:` —
one summary line, then
`  ✗ <class> <subject>: expected <e>, found <f> (<source>)` lines — and
SHALL exit:

- `0` when every check ran and no DRIFT finding exists; the final line is
  `comparison-check: ok`.
- `1` when at least one DRIFT finding and no UNAVAILABLE finding exists —
  machine-fixable and issue-only drift share exit 1; the routing happens
  through the two output files.
- `2` when at least one UNAVAILABLE finding exists, whether or not DRIFT
  findings also exist — UNAVAILABLE dominates because a run that could
  not reach its sources cannot vouch for the table; both classes still
  print. Malformed env entries, an unreadable README, and unknown
  command-line arguments count as exit-2 failures.

When `--report <path>` is passed, the script SHALL write the issue-body
markdown to `<path>` if and only if issue-only DRIFT findings exist
(`repo-gone`, `dead-link`, `moved-url`, `missing-cell`): a header line
with the run timestamp and GitHub run id when set, a
`Class | Expectation | Expected | Found | Source` table of exactly those
findings, and a fix guidance paragraph stating these findings are not
machine-fixable. Machine-fixable-only drift (`release-drift`,
`stale-cell`) SHALL write no report file — the fix PR is that alarm — so
the workflow's report-guarded issue step stays silent. Issue mode
(`--report-issue <path>`) SHALL read the report written by check mode,
require `GITHUB_TOKEN` and `GITHUB_REPOSITORY` (exit 2 when unset),
turn a plain `GitHub run <n>` into the run URL when `GITHUB_SERVER_URL`
and `GITHUB_RUN_ID` are set, and create or update the single issue
through the REST API: dedup via
`GET /repos/{owner}/{repo}/issues?labels=comparison-drift&state=open`;
if an open issue exists, `PATCH` its body to the fresh findings;
otherwise first ensure the label with an idempotent
`POST /repos/{owner}/{repo}/labels` sending the name `comparison-drift`
— a 422 already-exists response SHALL count as success, any other
non-2xx SHALL fail the mode — and then `POST /repos/{owner}/{repo}/issues`
with the fixed title `README comparison drift`, label
`comparison-drift`, and the report as body; issue creation never relies
on label auto-creation. Issue mode exits 0 on success and 2 on any REST
or environment failure, and ignores `--readme`, `--report`, and
`--summary`; `--fix` together with `--report-issue` is a usage error
(exit 2). The `gh` CLI SHALL NOT be used anywhere.

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

### Requirement: Fix mode dual-updates the README and the workflow env

`--fix` SHALL be a third script mode (alongside check and
`--report-issue`), running the dual-update the fix PR commits: for each
`EXPECT_REPOS` entry it GETs `releases/latest` from the live API and, for
every entry answered 200, rewrites both surfaces to the live `tag_name`:

- in the README (`--readme <path>`, default the repo-root `README.md`),
  every occurrence of the anchored cell shape
  `[<any tag>](https://github.com/<owner>/<repo>/releases/tag/<any tag>)`
  becomes `[<live tag>](https://github.com/<owner>/<repo>/releases/tag/<live tag>)`
  — the match is anchored on the repo's `releases/tag/` URL, so two
  competitors sharing a tag string on one table line can never collide;
- in the workflow file (`--workflow <path>`, default
  `.github/workflows/comparison-freshness.yml`), the single
  `EXPECT_REPOS:` line's entry `<owner>/<repo>@<old tag>` becomes
  `<owner>/<repo>@<live tag>` — the line-oriented rewrite requires the
  value to stay on one line.

The rewrite SHALL be idempotent-convergent: every 200-answered entry is
rewritten to its live tag whether or not it mismatched, so a
hand-broken README cell (env and live agree, README wrong) heals in the
same run. Entries answered 404 SHALL be skipped untouched — the issue
path owns them — and so SHALL a 200-answered entry whose anchored cell
shape matches nowhere in the README: check mode routes that state to
`missing-cell` (issue-only) and the fixer cannot recreate a deleted row,
so it prints a `comparison-check:` skip line and leaves that entry's
README cell and env segment untouched while converging the remaining
entries — never aborting a run whose issue step must still alarm. All
replacements SHALL be computed in memory before any write; the mode SHALL
exit 2 writing nothing only when an entry's live tag cannot be determined
(transport failure or non-404 non-2xx) or when the workflow file or its
`EXPECT_REPOS:` line is missing. Otherwise it exits 0 after writing both
files, including when nothing changed. The fixer SHALL NOT modify
✅/⚠️/❌ matrix cells, footnotes, the checked-date line, or any file
other than the two named — its patterns match only `releases/tag/` links
and the env line. Fix mode ignores `--report` and `--summary`.

#### Scenario: Fix updates the README cell and the env line together

- **WHEN** the fixture API answers `v0.6.0` for an entry pinned `v0.5.0`
  in both the fixture README's release cell and the fixture workflow's
  `EXPECT_REPOS` line, and `--fix` runs
- **THEN** after the run both files carry `v0.6.0` — the README cell
  reads
  `[v0.6.0](https://github.com/<owner>/<repo>/releases/tag/v0.6.0)` and
  the env entry reads `<owner>/<repo>@v0.6.0` — and the exit code is 0

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
- **THEN** `--fix` exits 2 and both the README and the workflow file are
  byte-identical to their pre-run state — no partial dual-update

#### Scenario: A missing EXPECT_REPOS line is loud

- **WHEN** `--fix` runs against a workflow file that has no
  `EXPECT_REPOS:` line (or the file itself is missing)
- **THEN** it exits 2 with a `comparison-check:` `✗` line naming the
  file, and writes nothing

### Requirement: A monthly workflow routes drift to a rolling fix PR and one deduped issue

`.github/workflows/comparison-freshness.yml` SHALL run on `schedule`
with cron `"0 6 1 * *"` (day 1, 06:00 UTC — monthly, bounding the rot
window at 31 days) and on `workflow_dispatch`; declare top-level
`permissions: contents: write, pull-requests: write, issues: write`; and
declare a `concurrency` group `comparison-freshness` with
`cancel-in-progress: false` — serialized because two concurrent runs
could both see zero open issues in the dedup GET and both POST, or race
on the fixed fix branch. The single job SHALL run on `ubuntu-latest`,
set `GITHUB_TOKEN` and `EXPECT_REPOS` in the job-level `env` block, and
use `actions/checkout@v7` and `actions/setup-node@v7` with
`node-version: 22`, no cache key, and no `npm ci` — the script imports
Node builtins only.

The check step SHALL capture the script's exit code without failing the
step on it (`set +e`;
`node scripts/check-comparison.mjs --report drift-report.md --summary "$GITHUB_STEP_SUMMARY"`),
treat a captured code outside {0, 1, 2} — 127 when `node` is missing, 137
when killed — as an environment failure (print a `comparison-check:`
line, exit with that code; the conditional steps key on exact codes so
neither PR nor issue actions run), and export the code to
`$GITHUB_OUTPUT`. Then, all guarded on the captured code:

- On `1`, in order: the `--fix` step; `npx --yes prettier@3.9.6 --write
README.md` so the committed README satisfies the ci.yml:55 prettier
  check — enforced on the master push after merge, since `pull_request`
  workflows do not fire for PRs opened with the default `GITHUB_TOKEN`;
  `peter-evans/create-pull-request@v8` with branch
  `chore/comparison-freshness`, `base: master`, `delete-branch: true`,
  and a body naming the dual-update and the untouched cells — the rolling
  PR: while one is open, the next drifting run updates the same branch
  instead of opening a second PR, and when the fixer changed nothing the
  action creates no PR. The bot never pushes master; the maintainer
  merges. A failing `--fix` step (exit 2) reddens the run and skips the
  PR and issue steps — consistent with the unreachable-source rule.
- On `1`, after the PR steps: the issue step, guarded additionally by
  `[ -f drift-report.md ]`, running
  `node scripts/check-comparison.mjs --report-issue drift-report.md` —
  machine-fixable-only drift writes no report, so no issue is filed and
  the PR is the only alarm. The job finishes green on code 1: drift is
  alarmed, not a build failure.
- On `2`: fail the job (red run) with a `comparison-check:` message; no
  issue, no PR.
- On `0`: finish green with no action.

The script SHALL NOT be added to `make check` or to the ci.yml push/PR
gates — `Makefile` and `.github/workflows/ci.yml` stay byte-identical,
and the node tests run locally only.

#### Scenario: Machine drift opens the rolling fix PR and the run stays green

- **WHEN** a run's check exits 1 with machine-fixable findings and the
  fixer dual-updates README and env
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

Check mode SHALL accept a `--summary <path>` flag. When passed, the
script SHALL append a markdown audit report to `<path>` after the checks
run, on every classification — clean, drift, and unavailable runs alike,
because the trail exists exactly for the clean runs that file nothing and
whose logs expire with retention. A successful write SHALL print no
stdout line and change no exit code; a failed write SHALL be recorded as
an UNAVAILABLE finding naming the path, forcing exit 2 through the
dominance rule. Fix mode and issue mode SHALL ignore the flag. Without
the flag, no file is written and stdout plus the exit code stay the whole
contract.

The summary SHALL contain the checked-at timestamp in UTC, the
`EXPECT_REPOS` value as given (`(unset)` when unset or empty), one table
per fact class with a row per item — `## Release checks` (repo, expected
tag, found tag or status or fetch error, verdict), `## README presence`
(entry, substring searched, found yes/no), `## Link liveness` (URL, final
status, verdict) — and the classification line
`Classification: <clean|drift|unavailable> — <n> checks, <d> drift, <u> unavailable`
under the same dominance rule as the exit code. A class with no rows
keeps its heading and shows a `_No checks ran._` line. The summary SHALL
carry no credentials and no environment variable beyond `EXPECT_REPOS`,
which the renderer reads by name — `GITHUB_TOKEN` and `GITHUB_REPOSITORY`
are excluded explicitly and `process.env` is never iterated. The
workflow's check step SHALL pass `--summary "$GITHUB_STEP_SUMMARY"`; the
script itself never reads `GITHUB_STEP_SUMMARY` from the environment.

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
