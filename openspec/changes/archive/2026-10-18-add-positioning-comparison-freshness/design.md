# Design

## Approach

Port the mechanics of the reference implementation
(`/Users/alexeyco/ts/github.com/alexeyco/opencode-openspec`:
`scripts/check-comparison.mjs`, `.github/workflows/comparison-freshness.yml`,
`.github/workflows/vendor-sync.yml`) to this repo's fact surface. The
reference checks npm registry facts; here the machine facts are GitHub
release tags, repo existence (a byproduct of the same GET), and link
liveness — all against the GitHub API. The reference files an issue on
any drift; here drift splits into two routes — a NEW `--fix` mode opens a
rolling dual-update PR for machine-fixable drift, and only drift the
script must not rewrite files the deduped issue. Content of the README
section comes from the recon data below; nothing else in the repo
changes.

## Files

| Path                                         | Action                                                                                        |
| -------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `README.md`                                  | edit — insert `## Comparison` between line 40 (end of Quickstart) and line 42 (`## The flow`) |
| `scripts/check-comparison.mjs`               | new — check / `--fix` / `--report-issue` modes, `--summary` flag                              |
| `.github/workflows/comparison-freshness.yml` | new — monthly cron + dispatch, env contract, PR and issue routing                             |
| `tests/comparison.test.mjs`                  | new — `node --test`, offline `node:http` fixture server                                       |

Byte-identical, verified by task 5.4: `Makefile`,
`.github/workflows/ci.yml`, `.github/workflows/release.yml`,
`CHANGELOG.md`, `Cargo.toml`, `herdr-plugin.toml`, `src/**`, `docs/**`.
No `package.json` is added; the script and the tests import Node builtins
only. `cargo test --locked` ignores the `.mjs` file in `tests/` (cargo
compiles only `.rs` crate roots there).

## Recon data (apply-phase input, captured 2026-10-18)

Columns: subject `herdr-review` (alexeyco/herdr-review) plus five
competitors — display name → repo:

- `reviewr` → `persiyanov/herdr-reviewr`
- `hunk-diff` → `jhochenbaum/herdr-hunk-diff`
- `differ` → `JanSmrcka/differ`
- `quantk` → `quantk/herdr-review`
- `lasso` → `skellleks/lasso`

Matrix, 7 axes × 6 columns (✅/⚠️/❌; evidence for footnotes in
parentheses; negative searches performed 2026-10-18):

1. **Ships a herdr-plugin manifest** — subject ✅; reviewr ✅
   (herdr-plugin.toml); hunk-diff ✅; differ ❌ (no manifest in the tree;
   brew/go install; "works with herdr" badge only); quantk ✅; lasso ✅.
2. **Human-authored line annotations** — all six ✅ (each README
   documents line comments).
3. **Dedicated git-diff review surface** — all six ✅.
4. **Delivers on close, no extra keypress** — subject ✅ (delivery on
   review close); reviewr ⚠️ (`s` → pane.send_text, unsubmitted);
   hunk-diff ⚠️ (explicit send-review action → `herdr agent prompt`);
   differ ⚠️ (`S` submits via `herdr agent prompt`, manual key); quantk
   ⚠️ (F7 drafts, never submits); lasso ⚠️ (Shift+S keypress). KEY
   FINDING: none of the five delivers on close — all are key- or
   action-triggered, so this axis is one of the two where the subject is
   the only ✅; the axis is formulated "Delivers on close, no extra
   keypress" accordingly.
5. **Agent-agnostic delivery** — subject ✅; reviewr ✅ (Claude
   Code/Codex/OpenCode/Pi); hunk-diff ✅ ("agent kinds are not
   addressable", `src/herdr.ts:73`); quantk ✅ (opaque agentKind);
   differ ✅ (claude, codex, gemini, copilot, opencode, aider); lasso ⚠️
   (core generic, cross-repo discovery Claude-only, `src/runtime.rs:258`).
6. **No LLM keys in the core flow** — all six ✅.
7. **User-selectable underlying review tool** — subject ✅
   (revdiff/tuicr/lumen catalog); all five ❌ (single built-in viewer
   each). The subject's second sole-✅ axis.

Detailed table rows:

- **Delivery form** — subject, reviewr, hunk-diff, quantk, lasso: herdr
  plugin (manifest install); differ: standalone binary (brew/go install)
  integrating with herdr.
- **Underlying tool** — subject: user-selected revdiff | tuicr | lumen;
  each competitor: its single built-in viewer.
- **Latest release** — the TASK C snapshot below, cell shape
  `[<tag>](https://github.com/<owner>/<repo>/releases/tag/<tag>)`;
  subject cell: `[releases](https://github.com/alexeyco/herdr-review/releases)`,
  no pinned tag. No dates in cells — a date is not machine-checked and
  would rot silently between human audits.
- **License / language** — read from each repo's GitHub page at apply
  time (language from the repo's language bar, license from its LICENSE
  file); each cell links the repo. Human-audited under the dated line,
  like every non-release cell.

TASK C snapshot (2026-10-18, GitHub API `releases/latest`; dates are
recon metadata, not README content):

| Repo                          | tag_name  | published  |
| ----------------------------- | --------- | ---------- |
| `persiyanov/herdr-reviewr`    | `v0.46.0` | 2026-10-06 |
| `jhochenbaum/herdr-hunk-diff` | `v0.5.0`  | 2026-10-12 |
| `JanSmrcka/differ`            | `v1.8.1`  | 2026-10-05 |
| `quantk/herdr-review`         | `v0.2.7`  | 2026-07-28 |
| `skellleks/lasso`             | `v0.1.0`  | 2026-08-26 |

The workflow env line at authoring time (drift-maintenance data, moved
only by `--fix` afterwards):

```yaml
EXPECT_REPOS: persiyanov/herdr-reviewr@v0.46.0,jhochenbaum/herdr-hunk-diff@v0.5.0,JanSmrcka/differ@v1.8.1,quantk/herdr-review@v0.2.7,skellleks/lasso@v0.1.0
```

Footnote evidence links: repo-root link where the evidence is the
competitor's README; tag-pinned blob URL where it is a file and line —
`https://github.com/jhochenbaum/herdr-hunk-diff/blob/v0.5.0/src/herdr.ts`
(axis 5, hunk-diff) and
`https://github.com/skellleks/lasso/blob/v0.1.0/src/runtime.rs` (axis 5,
lasso). Tag-pinned blobs stay alive after later releases; refreshing them
is human-audit work under the dated line, never bot work.

## Script contract

`scripts/check-comparison.mjs`, three modes plus one flag:

| Surface                 | Behavior                                                                                                           |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------ |
| check mode (default)    | `--readme <path>` (default repo-root `README.md`), `--report <path>`, `--summary <path>`; exits 0/1/2              |
| `--fix`                 | dual-update mode; extra flag `--workflow <path>` (default `.github/workflows/comparison-freshness.yml`); exits 0/2 |
| `--report-issue <path>` | issue mode; needs `GITHUB_TOKEN`, `GITHUB_REPOSITORY`; exits 0/2                                                   |
| unknown arguments       | usage line, exit 2; `--fix` together with `--report-issue` is a usage error                                        |

Finding taxonomy — severity × routing:

| Class           | Trigger                                                                                                                      | Severity    | Routing                      |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------- | ---------------------------- |
| `release-drift` | `releases/latest` 200, `tag_name` ≠ env tag                                                                                  | drift       | machine-fixable → fix PR     |
| `stale-cell`    | env-tag substring absent, tagless `releases/tag/` prefix present                                                             | drift       | machine-fixable → fix PR     |
| `repo-gone`     | `releases/latest` 404 (gone, private, releases dropped)                                                                      | drift       | issue-only                   |
| `dead-link`     | liveness HEAD final 404                                                                                                      | drift       | issue-only                   |
| `moved-url`     | liveness HEAD final URL ≠ requested (trailing `/` stripped both sides)                                                       | drift       | issue-only                   |
| `missing-cell`  | tagless `releases/tag/` prefix absent from README                                                                            | drift       | issue-only                   |
| `unavailable`   | any other non-2xx (403 rate limit included), transport failure, malformed/unset env, unreadable README, failed summary write | unavailable | red run, dominates exit code |

Network contract (ported): every attempt times out after 10 s
(`AbortSignal.timeout`); transport error, 429, or 5xx retries exactly
once after 2 s, then classifies `unavailable`; liveness is one HEAD per
deduplicated URL, redirects followed, no GET fallback (a HEAD-hostile
host classifies `unavailable` and reddens the run — the ported pinned
decision; a persistent false red is maintainer-visible and fixable by
editing the README link). Seams: `COMPARISON_GITHUB_API` (default
`https://api.github.com`) is the only base-URL seam — there is no npm
surface here. `GITHUB_TOKEN` adds the Bearer header when set
(authenticated 5000/h vs anonymous 60/h on shared runner IPs). One
`releases/latest` GET per entry per run; one README read feeding both the
URL set and the presence substrings; presence matching is pure
case-sensitive substring search, layout-insensitive by design.

Stdout: every line prefixed `comparison-check:`; clean runs end
`comparison-check: ok`; findings print as
`  ✗ <class> <subject>: expected <e>, found <f> (<source>)`.

`--report <path>`: written if and only if issue-only drift exists —
header with UTC timestamp and `GitHub run <n>` (when `GITHUB_RUN_ID` is
set), a `Class | Expectation | Expected | Found | Source` markdown table
of the issue-only findings only, and a guidance paragraph (these are not
machine-fixable: update the README by hand — and the workflow env line
when a release expectation moved — then close the issue; the next monthly
run re-files if drift persists). Machine-fixable-only drift writes no
report: the PR is that alarm, and the workflow's `[ -f drift-report.md ]`
guard keeps the issue step silent.

`--report-issue <path>`: requires `GITHUB_TOKEN` + `GITHUB_REPOSITORY`
(exit 2 when unset or when `GITHUB_REPOSITORY` is not `owner/repo`);
turns plain `GitHub run <n>` into the run URL from
`GITHUB_SERVER_URL`/`GITHUB_RUN_ID`; dedup GET
`/repos/{owner}/{repo}/issues?labels=comparison-drift&state=open` →
PATCH the first open issue's body, else ensure the label with an
idempotent POST `/repos/{owner}/{repo}/labels` (422 already-exists counts
as success, any other non-2xx fails the mode) and then POST
`/repos/{owner}/{repo}/issues` with title `README comparison drift`,
`labels: ["comparison-drift"]`, and the report as body; exit 0 on
success, 2 on any REST failure. No `gh`
CLI anywhere.

`--summary <path>`: appended on every classification in check mode —
checked-at UTC, `EXPECT_REPOS` as given (`(unset)` when unset/empty),
three class tables (`## Release checks`: Repo | Expected | Found |
Verdict; `## README presence`: Entry | Substring searched | Found;
`## Link liveness`: URL | Final status | Verdict), `_No checks ran._`
under any empty class heading, and the classification line
`Classification: <clean|drift|unavailable> — <n> checks, <d> drift, <u> unavailable`.
The renderer receives the env values as a parameter built by reading
`EXPECT_REPOS` by name — `process.env` is never iterated, so no token or
other variable can leak into the file. A failed summary write is an
`unavailable` finding (exit 2 via dominance), except in the
unreadable-README branch where exit 2 already stands and the summary is
still attempted with zero check rows.

## --fix dual-update mechanics

For each `EXPECT_REPOS` entry: GET `releases/latest` (same seams,
headers, retry rules as check mode). Per-entry outcomes: 200 → rewrite
both surfaces to the live `tag_name` (idempotent-convergent: rewritten
whether or not it mismatched, so a hand-broken README cell heals); 404 →
skip untouched (the issue path owns it); 200 whose anchored cell shape
matches nowhere in the README → skip untouched with a loud
`comparison-check:` skip line (check mode routes that state to
`missing-cell`, issue-only, and the fixer cannot recreate a deleted row —
skipping instead of aborting keeps the workflow green so the issue step
still alarms); anything else → abort.

README rewrite: a per-entry regex anchored on the repo's releases URL —
`\[[^\]]*\]\(https://github\.com/<owner>/<repo>/releases/tag/[^)]*\)`
with `owner`/`repo` regex-escaped — replaced wholesale with
`[<live tag>](https://github.com/<owner>/<repo>/releases/tag/<live tag>)`.
Anchoring on the repo URL makes the rewrite collision-proof when two
competitors share a tag string on one table line, and confines the fixer
to release cells: matrix cells, footnotes, and the
`All cells checked 2026-10-18 …` line are unreachable by construction,
which is how "the PR never touches cells or the date line" holds
mechanically, not by care.

Workflow rewrite: find the single line whose trimmed form starts with
`EXPECT_REPOS:`, split its value on `,`, replace the segment whose
`owner/repo` side matches, rejoin. The value stays on one line — the
line-oriented rewrite depends on it, and the spec pins it.

Atomicity: all replacements for both files are computed in memory first;
a transport failure, non-404 non-2xx, missing workflow file, or missing
`EXPECT_REPOS:` line — and only those — aborts with exit 2 and zero
writes. A 200-answered entry whose anchored regex matches nowhere is
skipped, not aborted, so a run carrying `missing-cell` states alongside
machine-fixable drift still converges, stays green, and files its issue.
Exit 0 otherwise, including the no-change case. Fix mode ignores
`--report` and `--summary` (`--readme`/`--workflow` select the files to
fix); `--fix` together with `--report-issue` is a usage error (exit 2).
Fix mode re-GETs live sources instead of consuming check-mode artifacts
— the modes stay independently runnable, at the cost of five extra
authenticated GETs per fix run.

## Workflow and PR flow

`.github/workflows/comparison-freshness.yml`:

- `on`: `schedule` cron `"0 6 1 * *"` + `workflow_dispatch`.
- `concurrency`: group `comparison-freshness`,
  `cancel-in-progress: false` — pending runs wait: two concurrent runs
  could both see zero open issues in the dedup GET and both POST, or race
  on the fix branch.
- `permissions`: `contents: write`, `pull-requests: write`,
  `issues: write`.
- One job `check` on `ubuntu-latest`; job-level `env`:
  `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` (Actions does not export it
  automatically; the explicit mapping follows the release.yml
  secrets-usage precedent at :39,:68 — job-level because the check, fix,
  and issue steps all need it) and the single-line `EXPECT_REPOS` with a
  DUAL-UPDATE comment (on drift, the bot updates the README release cells
  and this line together; no self entry — the subject cell is
  deliberately unpinned, so there is no machine-checkable self release
  fact).
- Steps: `actions/checkout@v7`; `actions/setup-node@v7` with
  `node-version: 22` and no cache key (builtins only, nothing to cache,
  no `npm ci`); check step `id: check` — `set +e`, run
  `node scripts/check-comparison.mjs --report drift-report.md --summary "$GITHUB_STEP_SUMMARY"`,
  capture `code=$?`, `case` guard: a code outside {0,1,2} prints
  `comparison-check: unknown exit code <n>` and exits with it (127/137
  environment failures redden the run and match no conditional step),
  else `echo "code=$code" >> "$GITHUB_OUTPUT"`, exit 0.
- On `code == '1'`, in order: `--fix` step; prettier step
  `npx --yes prettier@3.9.6 --write README.md` (a no-op when the fixer
  changed nothing — the committed README is already prettier-clean);
  `peter-evans/create-pull-request@v8` with
  `branch: chore/comparison-freshness`, `base: master`,
  `delete-branch: true`, a title and body naming the monthly run, the
  dual-update, and the untouched cells/date line. Rolling semantics: an
  open PR on the fixed branch is updated by the next drifting run, never
  duplicated; `delete-branch: true` cleans up after merge. The action
  creates no PR when the workspace is clean (issue-only-drift runs). The
  bot never pushes master — PR only, the maintainer merges (AGENTS.md).
  A failing `--fix` (exit 2) reddens the run and skips the remaining
  steps — the same alarm philosophy as unreachable sources.
- On `code == '1'`, after the PR steps: issue step guarded by
  `if [ -f drift-report.md ]; then … fi` running
  `--report-issue drift-report.md`.
- On `code == '2'`: red step — print the `comparison-check:` message to
  stderr, `exit 1`.

## Issue flow

Title `README comparison drift`, label `comparison-drift`. The label does
not exist in this repo today (verified 404 read-only), so issue mode
ensures it before the creation POST with an idempotent
`POST /repos/{owner}/{repo}/labels` — a 422 already-exists response
counts as success, any other non-2xx fails the mode — and creation never
relies on `POST /issues` label auto-creation. Exactly one open issue by
dedup GET + PATCH/POST under the serialized concurrency group.

## Test strategy and trade-off

`tests/comparison.test.mjs`, run with `node --test tests/comparison.test.mjs`
(ported and adapted from the reference `tests/comparison.test.ts`, minus
the npm-registry surface):

- One `node:http` server on 127.0.0.1 plays the GitHub API seam
  (`COMPARISON_GITHUB_API`) and every fixture-README liveness URL.
  Per-test route tables; repeated entries for one method+url answer
  successive requests in order (the 429-then-200 and 5xx retry shapes);
  the request log records bodies and the `Authorization` header (proving
  the token-bearing GET and the issue POST/PATCH payloads); the
  unreachable case points the seam at a bound-then-closed port.
- Fixture READMEs are tiny documents with no table markup — layout is
  never parsed; one line per entry carries the
  `github.com/<owner>/<repo>/releases/tag/<tag>` substring. Fixture
  workflow files carry a single `EXPECT_REPOS:` line.
- Each case spawns the script as a child process and asserts exit status,
  stdout lines, report/summary file contents, and post-`--fix` file bytes
  (including the unchanged date line and ⚠️ cells).

Trade-off, decided: the tests join no CI gate. The push/PR gates must
stay byte-identical (confirmed constraint), this repo has no Node
toolchain gate, and adding one for a monthly maintenance script is not
proportionate. Consequence: a script regression surfaces at the next
monthly run (loudly — exit codes and the audit summary) or in local
`node --test` runs, not at push time. Mitigation: the authoring PR runs
the suite locally and reports it as evidence (tasks 4.x, 5.3).

## Decisions

1. Heading order: Quickstart → Comparison → The flow → Why (Comparison
   directly after Quickstart, mirroring the reference's sales ordering;
   `## The flow` stays after it, per the confirmed playback's latitude).
2. CHANGELOG untouched: per-release maintained, no Unreleased convention.
3. Release cells carry the tag link only, no dates — every machine fact
   in the README is one the bot dual-updates; dates would rot between
   human audits.
4. Subject column unpinned: no self entry in `EXPECT_REPOS` — the
   subject's release cell carries no tag, so there is no
   machine-checkable self release fact (the ci.yml version-sync job
   checks `Cargo.toml`↔`herdr-plugin.toml` consistency only, not
   releases).
5. Moved-URL detection compares the full URLs after stripping one
   trailing `/` from both sides, so the common benign `https://host` →
   `https://host/` redirect is not a permanent false issue.
6. Presence misses route by prefix probe: a stale cell is
   machine-fixable (the fixer rewrites tags), a missing cell is
   issue-only (the fixer cannot recreate a deleted row, and `--fix`
   skips such entries loudly instead of aborting) — keeps the fix PR
   convergent, the run green, and the issue list meaningful.
7. `--fix` re-GETs live sources rather than reading a check-mode
   artifact: modes stay independently runnable and the fix never trusts a
   stale handoff file; the cost is five extra authenticated GETs.
8. `--report` is written only for issue-only drift, and the workflow
   guards the issue step on the file's existence — one mechanism, two
   routes, no exit-code split needed between drift classes.
9. HEAD-only liveness with no GET fallback and 403 → `unavailable`
   (red), ported verbatim: a bot challenge is never mistaken for a dead
   link.
10. Tests are Node-side (`node --test`) and gate-free; `cargo test` and
    the Rust suite are untouched.
