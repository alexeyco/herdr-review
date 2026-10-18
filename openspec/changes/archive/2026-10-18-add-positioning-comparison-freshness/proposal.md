# Proposal

## Why

The niche filled: 10+ herdr review plugins now exist, and this README
claims nothing about the alternatives — a user comparing them leaves to
search GitHub by hand. Hand-written competitor claims rot the day they
land, so the comparison ships together with a freshness mechanism: a
monthly GitHub Actions check that verifies the machine-checkable facts
(competitor release tags, repo existence, link liveness) against the
GitHub API and alarms drift either as a rolling fix PR (machine-fixable
facts) or as a single deduped issue (facts a script must not rewrite).
Every other cell is a sourced, human-audited claim under one dated line.

## What Changes

- **README** gains a `## Comparison` section — chosen heading order:
  `## Quickstart` → `## Comparison` → `## The flow` → `## Why`, i.e. the
  section is inserted directly after the Quickstart section (README.md:40)
  and before `## The flow` (README.md:42); this satisfies the confirmed
  "after `## Quickstart`, before `## Why`" placement and keeps
  `## The flow` after the comparison. Contents: intro + legend line, an
  at-a-glance matrix of 7 axes × 5 competitors (reviewr, hunk-diff,
  differ, quantk, lasso) with source links and numbered footnotes, the
  dated line `All cells checked 2026-10-18 against the linked public
sources.`, a `<details>` "Detailed comparison" table (delivery form,
  underlying tool, latest release, license/language), and a
  combination-only positioning paragraph.
- **New script** `scripts/check-comparison.mjs` (Node builtins only, no
  npm dependencies): check mode (default), a NEW `--fix` mode that
  dual-updates the README release cells and the workflow env expectations
  from live sources, `--report-issue` for the deduped issue REST flow, and
  `--summary` for the job-summary audit trail. Exit codes 0/1/2 with
  UNAVAILABLE dominance.
- **New workflow** `.github/workflows/comparison-freshness.yml`: monthly
  cron day-1 06:00 UTC + `workflow_dispatch`, serialized by a concurrency
  group. Machine drift → green run + rolling fix PR on the fixed branch
  `chore/comparison-freshness` (`npx --yes prettier@3.9.6 --write
README.md` before the commit; the PR never touches ✅/⚠️/❌ cells or the
  checked-date line). Issue-only drift → one deduped issue titled
  `README comparison drift` with label `comparison-drift`. Unreachable
  source (including a 403 rate limit) → exit 2 red run, no issue, no PR.
  Every run appends a job-summary audit trail via
  `--summary "$GITHUB_STEP_SUMMARY"`.
- **New tests** `tests/comparison.test.mjs` (`node --test`, offline
  `node:http` fixture server). They run locally and are reported as
  evidence in the authoring PR — no CI gate is added or changed, and no
  PR gate is claimed.
- **New capabilities**: `public-positioning` (README comparison content
  rules) and `comparison-freshness` (script, workflow, and alarm
  contract). Both are pure `## ADDED Requirements` deltas — no existing
  spec covers README content or repo automation.

ASSUMPTION: the explicit label-ensure `POST /repos/{owner}/{repo}/labels` (run before issue creation, so the label never depends on `POST /issues` auto-creation) is idempotent — a 422 already-exists counts as success — untested — a first-run label POST that neither 201-creates nor 422s disproves it; label absence verified read-only: `curl -s -o /dev/null -w '%{http_code}' https://api.github.com/repos/alexeyco/herdr-review/labels/comparison-drift` → `404`.
ASSUMPTION: GitHub-hosted `ubuntu-latest` runners provide Node 22 via `actions/setup-node@v7` and `npx` for prettier with no package.json in the repo — untested — a setup-node or npx failure in the first workflow run disproves it; consistent with the existing `npx --yes prettier@3.9.6` usage on the same runner (ci.yml:55).
ASSUMPTION: prettier@3.9.6 markdown-table formatting preserves the presence substrings the fixer writes — tested: scratch table cell `[v0.5.0](https://github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0)` run through `npx --yes prettier@3.9.6 --write README.md` → `grep -c 'github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0'` → `1`.
ASSUMPTION: `GET /repos/{owner}/{repo}/releases/latest` answers the release tag under `.tag_name` — tested: `curl -s https://api.github.com/repos/jhochenbaum/herdr-hunk-diff/releases/latest | grep -m1 '"tag_name"'` → `"tag_name": "v0.5.0",`.

REJECTED-RISK: a fix PR left unmerged for months keeps a stale release line in the README — accepted, the rolling PR itself is the alarm.
REJECTED-RISK: the human-audit date line "All cells checked <date>" lags release lines refreshed by the bot — accepted deliberately: one date, manual cell audits only; the bot never bumps it.

## Impact

- **Capabilities**: two new — `public-positioning`,
  `comparison-freshness`. The seven existing baseline specs
  (`annotation-delivery`, `mode-dispatch`, `placement-config`,
  `review-launch`, `review-scope-selection`, `reviewer-execution`,
  `setup-dialog`) are untouched.
- **Files**: `README.md` (section insertion); new
  `scripts/check-comparison.mjs`,
  `.github/workflows/comparison-freshness.yml`,
  `tests/comparison.test.mjs`. Unchanged, byte-identical: `Makefile`,
  `.github/workflows/ci.yml`, `.github/workflows/release.yml`,
  `CHANGELOG.md`, `src/**`, `docs/**`, `Cargo.toml`,
  `herdr-plugin.toml`. The script is NOT wired into `make check` or the
  ci.yml push/PR gates; `cargo test` ignores the `.mjs` file in `tests/`.
- **GitHub side effects**: the label `comparison-drift` comes into
  existence with the first issue; the bot branch
  `chore/comparison-freshness` and its PRs appear on drift; the bot never
  pushes master — every fix lands through a PR the maintainer merges
  (AGENTS.md rule).
- **Link checking**: README external links (the tool-catalog, herdr.dev,
  and comparison source links alike) gain their first liveness check
  through the monthly run. Nothing checks README links today — there is
  no `scripts/` directory in the tree and `make check` is
  lint/fmt-check/test/build plus the prettier gate (Makefile:28-29) — and
  README-relative links (`docs/how-it-works.md`, `CONTRIBUTING.md`) stay
  unchecked after this change; adding a relative-link checker is out of
  scope.
- **CHANGELOG**: no entry is added. The file is per-release maintained
  with no Unreleased convention; the maintainer records the addition at
  the next release.

## Out of Scope

- The existing tool-catalog table (revdiff/tuicr/lumen axes) stays as is.
- No stars/activity/popularity columns in either table.
- easy-review, modem-dev/hunk, plannotator browser, the pi-side tools,
  and Tier-3 TUIs stay out of the five competitor columns.
- Plugin code, the setup dialog, and `docs/**` are untouched.
- No `npm ci`, no `package.json`, no dependency install anywhere in the
  new workflow — the script imports Node builtins only.
