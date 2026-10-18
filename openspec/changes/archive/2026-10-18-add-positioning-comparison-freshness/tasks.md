# Tasks

Cell contents, tags, and evidence links come from the "Recon data"
section of `design.md` — do not re-derive them. Groups 1–4 touch disjoint
files; group 4 spawns group 2's script, and group 5 verifies everything,
so the consumption order is 1/2/3 → 4 → 5 (group 4 needs
`scripts/check-comparison.mjs` to exist — no stub is written for it).

## 1. README comparison section

Files: `README.md`.

- [x] 1.1 Insert the `## Comparison` section between the end of the
      `## Quickstart` section (README.md:40) and `## The flow`
      (README.md:42), with the parts in the spec's fixed order: intro
      sentence + legend line; the 7-axis × 6-column matrix with every
      cell value and footnote evidence from the design.md recon data
      (axis 4 formulated "Delivers on close, no extra keypress");
      numbered footnotes with unique markers in reading order, each
      definition carrying its public source link (repo-root or
      tag-pinned blob URLs as listed in design.md); the dated line
      `All cells checked 2026-10-18 against the linked public sources.`;
      the `<details>` "Detailed comparison" table (Delivery form,
      Underlying tool, Latest release, License / language — license and
      language read from each repo's GitHub page, each cell linking the
      repo); and the combination-only positioning paragraph (delivery on
      close + user-selectable tool). Scenarios: "Heading order puts
      Comparison after Quickstart", "Section anatomy in fixed order",
      "Seven axes across six columns", "Legend defines the three states",
      "Questioned cell carries its source", "Unique-combination claim
      only", "Absence stated neutrally".
- [x] 1.2 Write the detailed table's Latest release row in the canonical
      shape: each of the five competitor cells exactly
      `[<tag>](https://github.com/<owner>/<repo>/releases/tag/<tag>)`
      with the design.md TASK C snapshot tags and no dates; the subject
      cell links `https://github.com/alexeyco/herdr-review/releases` with
      no pinned tag. Scenario: "Latest-release cells carry the
      machine-checkable shape".
- [x] 1.3 Run `npx --yes prettier@3.9.6 --write README.md`, then
      `npx --yes prettier@3.9.6 --check "**/*.md"` → green. Expected
      after group 1 alone: prettier check passes; `git diff --stat`
      shows only `README.md` changed.

## 2. Freshness script

Files: `scripts/check-comparison.mjs` (new; Node builtins only —
`node:fs`, `node:path`, `node:url`; no `package.json`).

- [x] 2.1 Check-mode skeleton: `comparison-check:` stdout format, arg
      parsing (`--readme`, `--report`, `--summary`; unknown argument and
      `--fix` + `--report-issue` together are usage errors, exit 2), the
      `EXPECT_REPOS` env contract (last-`@` split, two non-empty
      `owner/repo` segments, non-empty tag, trimmed segments, every
      malformed or unset state an UNAVAILABLE finding naming variable and
      entry, no silent skip), single README read feeding the deduplicated
      `https?:` URL set and the presence text (never structural), exit
      0/1/2 with UNAVAILABLE dominance, and the network contract:
      `COMPARISON_GITHUB_API` seam, Bearer header when `GITHUB_TOKEN` is
      set, 10 s per-attempt timeout, exactly one retry after 2 s on
      transport error/429/5xx. Scenarios: "Env entry yields a release
      expectation", "Malformed env entry is loud, never skipped", "Clean
      run exits zero", "Drift-only run exits one", "Unreachable source
      exits two", "Mixed findings exit two".
- [x] 2.2 The three fact classes with the design.md routing table: one
      `releases/latest` GET per entry (200 match clean; 200 mismatch
      `release-drift`; 404 `repo-gone`; other non-2xx including 403 and
      transport failure `unavailable`; no separate existence GET);
      liveness HEAD per URL, redirects followed, no GET fallback (final
      2xx alive; final 404 `dead-link`; final URL ≠ requested after
      stripping one trailing `/` on both sides `moved-url`; everything
      else `unavailable`); presence substring
      `github.com/<owner>/<repo>/releases/tag/<tag>` per entry with the
      tagless-prefix probe on a miss (prefix present `stale-cell`, absent
      `missing-cell`), case-sensitive and layout-insensitive; unreadable
      README exits 2 after still attempting the summary. Scenarios: "A tag
      mismatch is machine-fixable drift", "A matching tag verifies clean",
      "A 404 release endpoint is issue-only drift", "A rate-limit 403 is
      unavailable, not drift", "A dead link is issue-only drift", "A moved
      URL is issue-only drift", "A stale release cell is machine-fixable
      presence drift", "A missing release cell is issue-only presence
      drift", "Presence checks are substring, not structural".
- [x] 2.3 `--report <path>` writer: the issue-body markdown (header with
      UTC timestamp and `GitHub run <n>` when `GITHUB_RUN_ID` is set,
      `Class | Expectation | Expected | Found | Source` table,
      not-machine-fixable guidance paragraph) written if and only if
      issue-only drift findings exist; `release-drift`/`stale-cell`-only
      runs write no file. Scenario: "Machine-fixable-only drift writes no
      report".
- [x] 2.4 `--fix` mode (`--workflow <path>`, default
      `.github/workflows/comparison-freshness.yml`): per-entry
      `releases/latest` GET; 200 → rewrite every anchored cell match
      `\[[^\]]*\]\(https://github\.com/<owner>/<repo>/releases/tag/[^)]*\)`
      in the README and the matching segment of the workflow's single
      `EXPECT_REPOS:` line to the live tag (idempotent-convergent); 404 →
      skip untouched; 200 but the anchored cell shape matches nowhere in
      the README → skip untouched with a loud `comparison-check:` skip
      line (the issue path owns that `missing-cell` state); all
      replacements computed in memory before any write; exit 2 with zero
      writes only on transport failure, non-404 non-2xx, or a missing
      workflow file or `EXPECT_REPOS:` line; exit 0 otherwise including
      no-change; ignores `--report`/`--summary` (and `--fix` together
      with `--report-issue` is a usage error); never touches matrix
      cells, footnotes, or the checked-date line. Scenarios: "Fix
      updates the README cell and the env line together", "Fix never
      touches matrix cells or the checked-date line", "Fix skips a 404
      entry and converges the others", "Fix skips an unmatchable cell
      and the run stays green", "Fix is atomic when a live source is
      unreachable", "A missing EXPECT_REPOS line is loud".
- [x] 2.5 `--report-issue <path>` mode: requires `GITHUB_TOKEN` +
      `GITHUB_REPOSITORY` (exit 2 when unset or not `owner/repo`); run-URL
      enrichment from `GITHUB_SERVER_URL`/`GITHUB_RUN_ID`; dedup GET
      `/repos/{owner}/{repo}/issues?labels=comparison-drift&state=open` →
      PATCH the first open issue's body, else ensure the label with an
      idempotent POST `/repos/{owner}/{repo}/labels` (422 already-exists
      counts as success; any other non-2xx exits 2 creating no issue),
      then POST with title `README comparison drift` and
      `labels: ["comparison-drift"]` — never relying on issue-creation
      label auto-creation; exit 0 on success, 2 on any REST failure;
      ignores `--readme`/`--report`/`--summary` (`--fix` together with
      `--report-issue` is a usage error, task 2.1); no `gh` CLI.
      Scenarios: "Repeat drift updates the same issue" (script side; the
      workflow guard is task 3.1), "Issue creation ensures the label
      first".
- [x] 2.6 `--summary <path>` audit trail: appended on every
      classification in check mode — checked-at UTC, `EXPECT_REPOS` as
      given (`(unset)` fallback), the three class tables (`## Release
checks`, `## README presence`, `## Link liveness`) with
      `_No checks ran._` under empty class headings, and the
      `Classification: …` line with counts under exit-code dominance; env
      values passed as a parameter read by name, `process.env` never
      iterated; failed write = UNAVAILABLE finding; unreadable-README
      branch still writes the zero-row summary; fix and issue modes
      ignore the flag. Scenarios: "A clean run's summary lists every
      check with verdicts", "An unavailable run still writes its audit
      summary", "The summary never contains credentials", "Without
      --summary nothing changes".
- [x] 2.7 Offline sanity: `env -u EXPECT_REPOS node
scripts/check-comparison.mjs` → exit 2 with a `✗` line naming
      `EXPECT_REPOS`, no network needed. Expected after group 2 alone:
      that command exits 2 loudly; no file outside `scripts/` changed.

## 3. Monthly workflow

Files: `.github/workflows/comparison-freshness.yml` (new).

- [x] 3.1 Write the workflow per design.md "Workflow and PR flow":
      `schedule` cron `"0 6 1 * *"` + `workflow_dispatch`; `concurrency`
      group `comparison-freshness`, `cancel-in-progress: false`;
      top-level `permissions` `contents: write` + `pull-requests: write` +
      `issues: write`; one `ubuntu-latest` job with job-level `env`
      `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` and the single-line
      `EXPECT_REPOS` (design.md snapshot values) plus the DUAL-UPDATE
      comment and the no-self-entry note; steps `actions/checkout@v7`,
      `actions/setup-node@v7` (`node-version: 22`, no cache, no
      `npm ci`), the `id: check` step (`set +e`, `--report
drift-report.md --summary "$GITHUB_STEP_SUMMARY"`, code capture,
      `case` guard reddening the run on codes outside {0,1,2}, code to
      `$GITHUB_OUTPUT`), then on `code == '1'` in order the `--fix` step,
      `npx --yes prettier@3.9.6 --write README.md`,
      `peter-evans/create-pull-request@v8` (`branch:
chore/comparison-freshness`, `base: master`, `delete-branch: true`,
      body naming the dual-update and the untouched cells/date line), and
      the issue step guarded by `[ -f drift-report.md ]`; on
      `code == '2'` the red step. Verify each conditional against the
      spec by reading the YAML — there is no local Actions runner, so the
      workflow scenarios are verified statically here and by the change
      verifier. Scenarios: "The workflow env block carries EXPECT_REPOS
      and the token mapping", "Machine drift opens the rolling fix PR and
      the run stays green", "Semantic drift files one issue and the run
      stays green", "Repeat drift updates the same issue" (workflow guard
      side), "Unavailable run goes red without PR or issue", "Unknown exit
      code fails the run", "The fix PR passes the prettier gate".
- [x] 3.2 Gate invariance: `git diff --stat Makefile
.github/workflows/ci.yml` → empty, and `grep -rn check-comparison
Makefile .github/workflows/ci.yml` → no hits. Scenario: "Push and
      PR gates stay byte-identical". Expected after group 3 alone: one
      new workflow file, gates untouched.

## 4. Node tests

Files: `tests/comparison.test.mjs` (new; runs under `node --test`; cargo
ignores it). Consumes group 2's script.

- [x] 4.1 Fixture harness per design.md "Test strategy": `node:http`
      server on 127.0.0.1 with per-test route tables (repeated entries
      answer successive requests in order for the retry shapes), request
      log recording bodies and the `Authorization` header, bound-then-
      closed port for the unreachable case, table-markup-free fixture
      READMEs, fixture workflow files with one `EXPECT_REPOS:` line,
      child-process spawning of `scripts/check-comparison.mjs`.
      Check-mode cases: clean exit 0 (`comparison-check: ok`); tag
      mismatch exit 1 + `release-drift` finding + no report file;
      matching tag clean; 404 → exit 1 + `repo-gone` + report written;
      403 → exit 2 + no report; dead link → exit 1 + `dead-link` +
      report; 301 to a different path → exit 1 + `moved-url`; redirect
      differing only by a trailing `/` → alive; stale-cell vs
      missing-cell prefix-probe routing; reformatted fixture table row
      still passes presence; malformed and unset `EXPECT_REPOS` → exit 2
      naming variable and entry; closed-port seam → exit 2; drift +
      unavailable mixed → exit 2; 429-then-200 → exactly two requests,
      clean; token present → `Authorization: Bearer` recorded on the
      release GET. Scenarios: "Env entry yields a release expectation",
      "Malformed env entry is loud, never skipped", "Presence checks are
      substring, not structural", "A tag mismatch is machine-fixable
      drift", "A matching tag verifies clean", "A 404 release endpoint is
      issue-only drift", "A rate-limit 403 is unavailable, not drift", "A
      dead link is issue-only drift", "A moved URL is issue-only drift",
      "A stale release cell is machine-fixable presence drift", "A
      missing release cell is issue-only presence drift", "Clean run exits
      zero", "Drift-only run exits one", "Unreachable source exits two",
      "Mixed findings exit two", "Machine-fixable-only drift writes no
      report".
- [x] 4.2 Fix-mode cases: single-entry drift → README cell and env line
      both carry the fixture live tag, exit 0; byte-identity assertions
      that the fixture date line `All cells checked 2026-10-18 against
the linked public sources.` and a ⚠️ matrix cell survive `--fix`
      verbatim; 404 entry skipped while another converges; 200-answered
      entry whose anchored cell shape matches nowhere in the fixture
      README → loud `comparison-check:` skip line, that entry's README
      and env segment untouched, remaining entries converged, exit 0
      (the run stays green and the issue path owns it); one entry's
      endpoint on the closed port → exit 2 and both files byte-identical;
      fixture workflow without an `EXPECT_REPOS:` line → exit 2, nothing
      written; converged input → exit 0, files unchanged. Scenarios: "Fix
      updates the README cell and the env line together", "Fix never
      touches matrix cells or the checked-date line", "Fix skips a 404
      entry and converges the others", "Fix skips an unmatchable cell
      and the run stays green", "Fix is atomic when a live source is
      unreachable", "A missing EXPECT_REPOS line is loud".
- [x] 4.3 Issue-mode cases: dedup GET returning an open issue → PATCH to
      `/repos/owner/repo/issues/<n>` with the fresh body and no POST;
      dedup GET returning `[]` → label-ensure POST
      `/repos/owner/repo/labels` first (fixture answers 422
      already-exists in one case, 201 created in another → issue POST
      proceeds, exit 0, request log ordered label-then-issue; fixture
      answers 500 → exit 2 and no issue POST), then POST with title
      `README comparison drift` and `labels: ["comparison-drift"]`;
      missing `GITHUB_TOKEN` or `GITHUB_REPOSITORY` → exit 2; REST 500 on
      the issue write → exit 2 after the retry. Scenarios: "Repeat drift
      updates the same issue", "Issue creation ensures the label first",
      "Semantic drift files one issue and the run stays green" (script
      side of the POST path).
- [x] 4.4 Summary cases: clean run with `--summary` → file holds
      checked-at UTC, the `EXPECT_REPOS` value as given, all three class
      tables with per-item rows, classification `clean` with zero counts;
      closed-port seam with `--summary` → exit 2 and the file still
      written with `unavailable` verdicts and the no-checks-ran line
      where applicable; fake `GITHUB_TOKEN` value absent from the file
      while the token-bearing GET really ran; no flag → no file created
      and stdout/exit byte-identical to the flagless contract; unreadable
      README path → exit 2 with the zero-row summary still written.
      Scenarios: "A clean run's summary lists every check with verdicts",
      "An unavailable run still writes its audit summary", "The summary
      never contains credentials", "Without --summary nothing changes".
- [x] 4.5 Run `node --test tests/comparison.test.mjs` → all tests pass;
      `cargo test --locked` → unchanged and green (cargo ignores the
      `.mjs` file). Expected after group 4 alone: the node suite exits 0;
      no file outside `tests/` changed.

## 5. Verification and docs gates

Files: none new — commands only; runs after groups 1–4.

- [x] 5.1 `npx --yes prettier@3.9.6 --check "**/*.md"` → green (README
      with the new section, the openspec artifacts of this change, and
      every other md file untouched by it).
- [x] 5.2 `openspec validate add-positioning-comparison-freshness` →
      VALID.
- [x] 5.3 `node --test tests/comparison.test.mjs` → green, and
      `env -u EXPECT_REPOS node scripts/check-comparison.mjs` → exit 2
      with the loud `✗` line.
- [x] 5.4 Gate and surface invariance: `git diff --stat Makefile
.github/workflows/ci.yml .github/workflows/release.yml CHANGELOG.md
Cargo.toml herdr-plugin.toml src docs` → empty; `git status
--porcelain` shows exactly one modified file (`README.md`) plus the
      new `scripts/check-comparison.mjs`,
      `.github/workflows/comparison-freshness.yml`,
      `tests/comparison.test.mjs`, and this change's `openspec/` artifacts.
      Scenario: "Push and PR gates stay byte-identical".
- [x] 5.5 Coverage re-check: every `#### Scenario:` heading in
      `specs/public-positioning/spec.md` and
      `specs/comparison-freshness/spec.md` is cited verbatim by at least
      one task above (`grep -h '#### Scenario:' specs/*/spec.md` against
      this file). Expected final state: all commands in 5.1–5.4 green.
