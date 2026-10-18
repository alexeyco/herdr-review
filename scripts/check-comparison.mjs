#!/usr/bin/env node
// Check the README `## Comparison` section for freshness against live
// sources.
//
// Usage:
//   node scripts/check-comparison.mjs [--readme <path>] [--report <path>]
//                                     [--summary <path>]
//   node scripts/check-comparison.mjs --fix [--readme <path>]
//                                     [--workflow <path>]
//   node scripts/check-comparison.mjs --report-issue <path>
//
// Three modes. Check mode (the default) takes every expected value from
// the env contract — EXPECT_REPOS — never from the README's text, and
// verifies three fact classes: release tags per env entry (one
// releases/latest GET each, repo existence as a byproduct — there is no
// separate existence GET), link liveness of every http:/https: URL in
// the README (one HEAD each, redirects followed, no GET fallback), and
// release-cell presence (pure case-sensitive substring search, never a
// structural parse). The README is read exactly once — feeding both the
// liveness URL set and the presence substrings. `--fix` dual-updates
// the README release cells and the workflow's EXPECT_REPOS line to the
// live tags; drift the fixer must not rewrite is routed to the report
// the issue mode consumes. Issue mode reads the drift report written by
// `--report` and creates or updates the single `comparison-drift` issue
// through the GitHub REST API (no `gh` CLI).
//
// `--summary <path>` (check mode only — fix and issue modes ignore it,
// as issue mode also ignores `--readme`/`--report`) appends a markdown
// audit trail to <path> on EVERY classification, clean runs included:
// per-item rows for all three fact classes plus a classification line.
// A summary write failure is an UNAVAILABLE finding (exit 2 through the
// dominance rule) — except in the unreadable-README branch, where the
// pending exit 2 already stands. The renderer receives the env values
// as a parameter built by reading EXPECT_REPOS by name, so no other
// environment value (token and repository included) can reach the file.
//
// Exit codes (check mode): 0 clean · 1 DRIFT only (machine-fixable and
// issue-only drift share exit 1 — the routing happens through the two
// output files) · 2 any UNAVAILABLE finding, whether or not drift also
// exists. UNAVAILABLE dominates because a run that could not reach its
// sources cannot vouch for the table. Fix mode exits 0 on success and
// 2 on any failure; issue mode exits 0 on success and 2 on any REST or
// environment failure. `--fix` together with `--report-issue` is a
// usage error (exit 2), as is any unknown argument.
//
// Environment seams (defaults and the env contract pinned in
// openspec/changes/add-positioning-comparison-freshness/design.md):
//   EXPECT_REPOS          comma-separated owner/repo@tag list
//                         (required in check and fix modes)
//   COMPARISON_GITHUB_API GitHub API base (default https://api.github.com)
//   GITHUB_TOKEN          Bearer auth for GitHub API calls
//                         (required in issue mode)
//   GITHUB_REPOSITORY     owner/repo (required in issue mode)

import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PREFIX = "comparison-check:";
const USAGE =
  "node scripts/check-comparison.mjs [--readme <path>] [--report <path>]" +
  " [--summary <path>] | --fix [--readme <path>] [--workflow <path>]" +
  " | --report-issue <path>";
const ATTEMPT_TIMEOUT_MS = 10_000;
const RETRY_DELAY_MS = 2_000;
const ISSUE_TITLE = "README comparison drift";
const ISSUE_LABEL = "comparison-drift";
// Drift the fixer must not rewrite: these classes (and only these) are
// written to the --report body the issue mode consumes. Machine-fixable
// drift (release-drift, stale-cell) writes no report — the fix PR is
// that alarm.
const ISSUE_ONLY_CLASSES = new Set([
  "repo-gone",
  "dead-link",
  "moved-url",
  "missing-cell",
]);

const GITHUB_API = (
  process.env.COMPARISON_GITHUB_API ?? "https://api.github.com"
).replace(/\/+$/, "");
const GITHUB_TOKEN = process.env.GITHUB_TOKEN;
const GITHUB_REPOSITORY = process.env.GITHUB_REPOSITORY;

function say(line) {
  console.log(`${PREFIX} ${line}`);
}

function finding(severity, cls, subject, expected, found, source) {
  return { severity, cls, subject, expected, found, source };
}

function printFinding(f) {
  say(
    `  ✗ ${f.cls} ${f.subject}: expected ${f.expected}, found ${f.found} (${f.source})`,
  );
}

// Every executed check records a result row — clean ones included —
// feeding only the `--summary` audit trail; findings stay the stdout /
// exit-code / `--report` surface unchanged.
function recordRow(rows, table, item, expected, found, verdict) {
  rows.push({ table, item, expected, found, verdict });
}

// ── Network helpers ─────────────────────────────────────────────────────────

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

// One retry after RETRY_DELAY_MS on transport error, 429, or 5xx; every
// other status is final (a 403 rate limit or bot challenge is neither
// retried nor GET-fallbacked — it classifies UNAVAILABLE, pinned
// decision).
async function fetchWithRetry(url, init = {}) {
  for (let attempt = 1; ; attempt++) {
    let res;
    try {
      res = await fetch(url, {
        ...init,
        signal: AbortSignal.timeout(ATTEMPT_TIMEOUT_MS),
      });
    } catch (err) {
      if (attempt === 1) {
        await sleep(RETRY_DELAY_MS);
        continue;
      }
      throw err;
    }
    if (attempt === 1 && (res.status === 429 || res.status >= 500)) {
      await sleep(RETRY_DELAY_MS);
      continue;
    }
    return res;
  }
}

function errorMessage(err) {
  const name = err?.name ?? err?.cause?.name;
  if (name === "TimeoutError") return "timeout";
  const code = err?.cause?.code ?? err?.code;
  if (typeof code === "string") return code;
  if (typeof name === "string") return name;
  return err?.message ?? "unknown error";
}

function githubHeaders() {
  const headers = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (GITHUB_TOKEN) headers.Authorization = `Bearer ${GITHUB_TOKEN}`;
  return headers;
}

// One releases/latest GET per entry: the release tag, repo existence,
// and the fix mode's live tag all come from this single request — there
// is no separate existence GET. Check mode and fix mode each issue it
// once per entry per run.
async function releasesLatest(owner, repo) {
  const url = `${GITHUB_API}/repos/${owner}/${repo}/releases/latest`;
  try {
    const res = await fetchWithRetry(url, { headers: githubHeaders() });
    let tag;
    if (res.status === 200) {
      try {
        tag = (await res.json())?.tag_name;
      } catch {
        tag = undefined;
      }
    }
    return { url, status: res.status, tag, error: null };
  } catch (err) {
    return { url, status: null, tag: undefined, error: err };
  }
}

// ── Env contract ────────────────────────────────────────────────────────────

// EXPECT_REPOS has no script-level default: the workflow's job env
// block is its only production source, and an unset or empty variable
// is an UNAVAILABLE finding naming it — a run missing its expectations
// fails loudly instead of silently checking only links.
function envFinding(variable, entry, expected, found) {
  return finding(
    "unavailable",
    "unavailable",
    entry === null ? variable : `${variable} ${entry}`,
    expected,
    found,
    variable,
  );
}

// `<owner>/<repo>@<tag>` split on the LAST `@`; the tag side is
// otherwise unconstrained (tags are arbitrary strings). A malformed
// entry is dropped only after its UNAVAILABLE finding is recorded —
// never a silent skip. An empty segment (doubled or trailing comma)
// counts as malformed.
function parseExpectRepos(findings) {
  const value = process.env.EXPECT_REPOS;
  if (value === undefined || value.trim() === "") {
    findings.push(
      envFinding(
        "EXPECT_REPOS",
        null,
        "a comma-separated owner/repo@tag list",
        "unset or empty",
      ),
    );
    return [];
  }
  const entries = [];
  for (const raw of value.split(",").map((segment) => segment.trim())) {
    const label = raw === "" ? '""' : raw;
    const at = raw.lastIndexOf("@");
    if (at === -1) {
      findings.push(
        envFinding(
          "EXPECT_REPOS",
          label,
          "an owner/repo@tag entry",
          raw === "" ? "empty entry" : "no @ in the entry",
        ),
      );
      continue;
    }
    const repoSide = raw.slice(0, at);
    const tag = raw.slice(at + 1);
    const parts = repoSide.split("/");
    if (tag === "") {
      findings.push(
        envFinding(
          "EXPECT_REPOS",
          label,
          "an owner/repo@tag entry",
          "empty tag segment",
        ),
      );
      continue;
    }
    if (parts.length !== 2 || parts.some((part) => part === "")) {
      findings.push(
        envFinding(
          "EXPECT_REPOS",
          label,
          "an owner/repo@tag entry",
          `owner/repo side "${repoSide}" is not two non-empty segments`,
        ),
      );
      continue;
    }
    entries.push({ owner: parts[0], repo: parts[1], tag, raw });
  }
  return entries;
}

// ── README reading ──────────────────────────────────────────────────────────

// One read feeding two consumers: the liveness URL set and the presence
// search text. The `https?:` superset exists so offline fixtures can
// point liveness URLs at a local http://127.0.0.1 server. The
// comparison table's structure — rows, columns, cell shapes, footnote
// grammar — is never parsed; every expected value comes from the env
// contract.
function readReadme(readmePath, summaryPath) {
  let text;
  try {
    text = readFileSync(readmePath, "utf-8");
  } catch (err) {
    // The audit trail covers every classification, this branch's
    // included: the run is UNAVAILABLE with zero checks ran, so the
    // summary is still written — every class heading carries its
    // no-checks-ran line, and this read failure is the one unavailable
    // in the classification count. A failed write here changes
    // nothing: exit 2 already stands.
    if (summaryPath) {
      appendSummary(summaryPath, [], summaryEnvValues(), [
        finding(
          "unavailable",
          "unavailable",
          readmePath,
          "a readable README file",
          err.message,
          readmePath,
        ),
      ]);
    }
    say(`cannot read ${readmePath}: ${err.message}`);
    process.exit(2);
  }
  const urls = [
    ...new Set([...text.matchAll(/https?:\/\/[^\s)"'`<>]+/g)].map((m) => m[0])),
  ];
  return { text, urls };
}

// ── Fact class 1: release tags (repo existence as a byproduct) ──────────────

async function checkRelease(entry, rows) {
  const r = await releasesLatest(entry.owner, entry.repo);
  const subject = `EXPECT_REPOS ${entry.raw}`;
  const item = `${entry.owner}/${entry.repo}`;
  if (r.error) {
    const found = `fetch failed (${errorMessage(r.error)})`;
    recordRow(rows, "release", item, entry.tag, found, "unavailable");
    return [finding("unavailable", "unavailable", subject, entry.tag, found, r.url)];
  }
  if (r.status === 200) {
    if (typeof r.tag !== "string" || r.tag === "") {
      recordRow(
        rows,
        "release",
        item,
        entry.tag,
        "no tag_name in the response",
        "unavailable",
      );
      return [
        finding(
          "unavailable",
          "unavailable",
          subject,
          entry.tag,
          "no tag_name in the response",
          r.url,
        ),
      ];
    }
    if (r.tag !== entry.tag) {
      recordRow(rows, "release", item, entry.tag, r.tag, "drift");
      return [finding("drift", "release-drift", subject, entry.tag, r.tag, r.url)];
    }
    recordRow(rows, "release", item, entry.tag, r.tag, "ok");
    return [];
  }
  if (r.status === 404) {
    // The repo disappeared, went private, or dropped releases — no live
    // tag to write, so no separate existence GET exists and the issue
    // path owns it.
    recordRow(rows, "release", item, entry.tag, "404", "drift");
    return [
      finding(
        "drift",
        "repo-gone",
        subject,
        entry.tag,
        "404 (repo gone, private, or releases dropped)",
        r.url,
      ),
    ];
  }
  const found = `HTTP ${r.status}`;
  recordRow(rows, "release", item, entry.tag, found, "unavailable");
  return [finding("unavailable", "unavailable", subject, entry.tag, found, r.url)];
}

// ── Fact class 2: link liveness ─────────────────────────────────────────────

// One HEAD per deduplicated URL, redirects followed, no GET fallback —
// a HEAD-hostile host classifies UNAVAILABLE and reddens the run (a
// persistent false red is maintainer-visible and fixable by editing the
// README link). Final 2xx: alive, unless the final URL differs from the
// requested one after stripping one trailing `/` from both sides —
// moved-url drift the script must not rewrite itself. Final 404:
// dead-link drift. Everything else — 403 bot challenge included — is
// UNAVAILABLE.
function stripOneTrailingSlash(url) {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

async function checkLiveness(url, rows) {
  const expected = "alive (2xx)";
  try {
    const res = await fetchWithRetry(url, { method: "HEAD" });
    if (res.ok) {
      const finalUrl = res.url ?? url;
      if (stripOneTrailingSlash(finalUrl) !== stripOneTrailingSlash(url)) {
        recordRow(
          rows,
          "liveness",
          url,
          expected,
          `${res.status} → ${finalUrl}`,
          "drift",
        );
        return [finding("drift", "moved-url", url, url, finalUrl, url)];
      }
      recordRow(rows, "liveness", url, expected, String(res.status), "alive");
      return [];
    }
    if (res.status === 404) {
      recordRow(rows, "liveness", url, expected, "404", "drift");
      return [finding("drift", "dead-link", url, expected, "404", url)];
    }
    recordRow(
      rows,
      "liveness",
      url,
      expected,
      String(res.status),
      "unavailable",
    );
    return [
      finding("unavailable", "unavailable", url, expected, String(res.status), url),
    ];
  } catch (err) {
    const found = `fetch failed (${errorMessage(err)})`;
    recordRow(rows, "liveness", url, expected, found, "unavailable");
    return [finding("unavailable", "unavailable", url, expected, found, url)];
  }
}

// ── Fact class 3: release-cell presence ─────────────────────────────────────

// Pure case-sensitive substring search over the text already read for
// the URL set — no network, no parser, layout-insensitive by design. On
// a miss the tagless `releases/tag/` prefix probe routes the drift: the
// prefix present means a stale cell (machine-fixable — the fixer
// rewrites tags), the prefix absent means the cell or row was removed
// (issue-only — the fixer cannot recreate a deleted row). A miss is
// never UNAVAILABLE: the file was readable.
function checkPresence(entries, readmeText, readmePath, rows) {
  const findings = [];
  for (const entry of entries) {
    const substring = `github.com/${entry.owner}/${entry.repo}/releases/tag/${entry.tag}`;
    const prefix = `github.com/${entry.owner}/${entry.repo}/releases/tag/`;
    const present = readmeText.includes(substring);
    recordRow(
      rows,
      "presence",
      `EXPECT_REPOS ${entry.raw}`,
      substring,
      present ? "yes" : "no",
      present ? "yes" : "no",
    );
    if (present) continue;
    if (readmeText.includes(prefix)) {
      findings.push(
        finding(
          "drift",
          "stale-cell",
          `EXPECT_REPOS ${entry.raw}`,
          substring,
          "stale tag (tagless release-cell prefix present)",
          readmePath,
        ),
      );
    } else {
      findings.push(
        finding(
          "drift",
          "missing-cell",
          `EXPECT_REPOS ${entry.raw}`,
          substring,
          "no release cell (tagless prefix absent)",
          readmePath,
        ),
      );
    }
  }
  return findings;
}

// ── Drift report and issue mode ─────────────────────────────────────────────

function markdownTable(rows) {
  const widths = rows[0].map((_, i) =>
    Math.max(...rows.map((row) => String(row[i]).length)),
  );
  const render = (cells) =>
    "| " +
    cells.map((cell, i) => String(cell).padEnd(widths[i])).join(" | ") +
    " |";
  const lines = [render(rows[0])];
  lines.push(
    "| " + widths.map((width) => "-".repeat(width)).join(" | ") + " |",
  );
  for (const row of rows.slice(1)) lines.push(render(row));
  return lines.join("\n");
}

function renderReport(drift) {
  const timestamp = new Date().toISOString();
  const runId = process.env.GITHUB_RUN_ID;
  const runClause = runId ? `, GitHub run ${runId}` : "";
  const table = markdownTable([
    ["Class", "Expectation", "Expected", "Found", "Source"],
    ...drift.map((f) => [f.cls, f.subject, f.expected, f.found, f.source]),
  ]);
  return [
    "The monthly comparison-freshness check found drift in the README",
    `\`## Comparison\` section (run ${timestamp}${runClause}) that the`,
    "fixer must not rewrite itself.",
    "",
    table,
    "",
    "Fix: these findings are not machine-fixable — update the README by",
    "hand (and the workflow env line when a release expectation moved),",
    "then close this issue; the next monthly run re-files it if drift",
    "persists.",
    "",
  ].join("\n");
}

async function restCall(url, init) {
  try {
    return { res: await fetchWithRetry(url, init), error: null };
  } catch (err) {
    return { res: null, error: err };
  }
}

// Issue mode: read the report check mode wrote, dedup by the open
// comparison-drift label, PATCH the existing issue's body or ensure the
// label and POST a new one. Creation never relies on issue-creation
// label auto-creation: the label POST happens first and a 422
// already-exists response counts as success.
async function runIssueMode(reportPath) {
  const missing = [];
  if (!GITHUB_TOKEN) missing.push("GITHUB_TOKEN");
  if (!GITHUB_REPOSITORY) missing.push("GITHUB_REPOSITORY");
  if (missing.length > 0) {
    say(`${missing.join(" and ")} not set — required for --report-issue`);
    process.exit(2);
  }

  const [owner, repo] = GITHUB_REPOSITORY.split("/");
  if (!owner || !repo || repo.includes("/")) {
    say(`GITHUB_REPOSITORY must be owner/repo, got "${GITHUB_REPOSITORY}"`);
    process.exit(2);
  }

  let body;
  try {
    body = readFileSync(reportPath, "utf-8");
  } catch (err) {
    say(`cannot read report ${reportPath}: ${err.message}`);
    process.exit(2);
  }

  // In the workflow, turn the plain "GitHub run <n>" into the run URL.
  const serverUrl = process.env.GITHUB_SERVER_URL;
  const runId = process.env.GITHUB_RUN_ID;
  if (serverUrl && runId) {
    const runUrl = `${serverUrl}/${GITHUB_REPOSITORY}/actions/runs/${runId}`;
    body = body.replace(/GitHub run (\d+)/, `GitHub run [$1](${runUrl})`);
  }

  const headers = {
    Authorization: `Bearer ${GITHUB_TOKEN}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const api = `${GITHUB_API}/repos/${owner}/${repo}`;

  const search = await restCall(
    `${api}/issues?labels=${ISSUE_LABEL}&state=open`,
    { headers },
  );
  if (search.error) {
    say(`issue search failed: fetch failed (${errorMessage(search.error)})`);
    process.exit(2);
  }
  if (!search.res.ok) {
    say(`issue search answered HTTP ${search.res.status}`);
    process.exit(2);
  }
  let issues;
  try {
    issues = await search.res.json();
  } catch {
    issues = null;
  }
  if (!Array.isArray(issues)) {
    say("issue search returned no issue array");
    process.exit(2);
  }

  if (issues.length > 0) {
    const number = issues[0]?.number;
    if (typeof number !== "number") {
      say("issue search returned an issue without a number");
      process.exit(2);
    }
    const update = await restCall(`${api}/issues/${number}`, {
      method: "PATCH",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify({ body }),
    });
    if (update.error) {
      say(`issue update failed: fetch failed (${errorMessage(update.error)})`);
      process.exit(2);
    }
    if (!update.res.ok) {
      say(`issue update answered HTTP ${update.res.status}`);
      process.exit(2);
    }
    say(`updated issue #${number}`);
    process.exit(0);
  }

  // No open issue: ensure the label before creating one. A 422
  // already-exists counts as success; any other non-2xx fails the mode
  // before any issue is created.
  const label = await restCall(`${api}/labels`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify({ name: ISSUE_LABEL }),
  });
  if (label.error) {
    say(`label creation failed: fetch failed (${errorMessage(label.error)})`);
    process.exit(2);
  }
  if (!label.res.ok && label.res.status !== 422) {
    say(`label creation answered HTTP ${label.res.status}`);
    process.exit(2);
  }

  const create = await restCall(`${api}/issues`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify({ title: ISSUE_TITLE, labels: [ISSUE_LABEL], body }),
  });
  if (create.error) {
    say(`issue creation failed: fetch failed (${errorMessage(create.error)})`);
    process.exit(2);
  }
  if (!create.res.ok) {
    say(`issue creation answered HTTP ${create.res.status}`);
    process.exit(2);
  }
  let created;
  try {
    created = await create.res.json();
  } catch {
    created = null;
  }
  say(`created issue #${created?.number ?? "?"}`);
  process.exit(0);
}

// ── Audit summary ────────────────────────────────────────────────────────────

// Column layout per fact class; the headings are pinned. Liveness rows
// show the effective final status in `found` (the redirect target on a
// moved URL); presence rows carry the yes/no verdict in both `found`
// and `verdict`.
const SUMMARY_TABLES = [
  {
    key: "release",
    heading: "## Release checks",
    header: ["Repo", "Expected", "Found", "Verdict"],
    cells: (row) => [row.item, row.expected, row.found, row.verdict],
  },
  {
    key: "presence",
    heading: "## README presence",
    header: ["Entry", "Substring searched", "Found"],
    cells: (row) => [row.item, row.expected, row.found],
  },
  {
    key: "liveness",
    heading: "## Link liveness",
    header: ["URL", "Final status", "Verdict"],
    cells: (row) => [row.item, row.found, row.verdict],
  },
];

const SUMMARY_ENV_VARIABLE = "EXPECT_REPOS";

function formatEnvValue(value) {
  return value === undefined || value === "" ? "(unset)" : value;
}

// The env value every summary renders, built in one place: both write
// sites — the post-check one and the unreadable-README one — read
// EXPECT_REPOS by name, and process.env is never iterated, so no other
// environment value (token and repository included) can reach the file.
function summaryEnvValues() {
  return { EXPECT_REPOS: process.env.EXPECT_REPOS };
}

// The renderer receives envValues as a parameter and never touches
// process.env itself.
function renderSummary(rows, envValues, findings) {
  const drift = findings.filter((f) => f.severity === "drift").length;
  const unavailable = findings.filter(
    (f) => f.severity === "unavailable",
  ).length;
  const word = unavailable > 0 ? "unavailable" : drift > 0 ? "drift" : "clean";
  const lines = [
    "# comparison-check — run summary",
    "",
    `Checked at: ${new Date().toISOString()}`,
    "",
    "Expectations as given:",
    "",
    `- \`${SUMMARY_ENV_VARIABLE}\`: ${formatEnvValue(envValues.EXPECT_REPOS)}`,
    "",
  ];
  for (const table of SUMMARY_TABLES) {
    const tableRows = rows.filter((row) => row.table === table.key);
    lines.push(table.heading, "");
    if (tableRows.length === 0) {
      // A class left without rows — its entries failed env validation
      // or none were configured — keeps its heading and says so.
      lines.push("_No checks ran._", "");
    } else {
      lines.push(
        markdownTable([table.header, ...tableRows.map(table.cells)]),
        "",
      );
    }
  }
  lines.push(
    `Classification: ${word} — ${rows.length} checks, ${drift} drift, ${unavailable} unavailable`,
    "",
  );
  return lines.join("\n");
}

// Appends the audit trail to <path> after the checks ran, before the
// exit-code decision. The classification line is rendered from the
// findings as they stand at this point; a failed write then pushes an
// UNAVAILABLE finding — which forces exit 2 through the dominance rule.
function appendSummary(summaryPath, rows, envValues, findings) {
  try {
    appendFileSync(summaryPath, renderSummary(rows, envValues, findings));
  } catch (err) {
    findings.push(
      finding(
        "unavailable",
        "unavailable",
        summaryPath,
        "a writable summary file",
        err?.message ?? String(err),
        summaryPath,
      ),
    );
  }
}

// ── Fix mode ────────────────────────────────────────────────────────────────

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// The anchored release-cell shape, anchored on the repo's releases/tag/
// URL so two competitors sharing a tag string on one table line can
// never collide, and so matrix cells, footnotes, and the checked-date
// line are unreachable by construction.
function cellPattern(entry) {
  return (
    "\\[[^\\]]*\\]\\(https://github\\.com/" +
    `${escapeRegExp(entry.owner)}/${escapeRegExp(entry.repo)}` +
    "/releases/tag/[^)]*\\)"
  );
}

function cellReplacement(entry, liveTag) {
  return (
    `[${liveTag}](https://github.com/${entry.owner}/${entry.repo}` +
    `/releases/tag/${liveTag})`
  );
}

// Locate the single `EXPECT_REPOS:` line and return its pieces: the
// line array, the index, the leading part (indentation + key + colon +
// spacing), the split segments, and whether the line ended with a CR.
// The value stays on one line — the line-oriented rewrite depends on it.
function readEnvLine(workflowText, workflowPath) {
  const lines = workflowText.split("\n");
  const idx = lines.findIndex((line) => line.trim().startsWith("EXPECT_REPOS:"));
  if (idx === -1) {
    say(`✗ no EXPECT_REPOS: line in ${workflowPath}`);
    process.exit(2);
  }
  const rawLine = lines[idx];
  const hadCr = rawLine.endsWith("\r");
  const lineBody = hadCr ? rawLine.slice(0, -1) : rawLine;
  const match = lineBody.match(/^(\s*EXPECT_REPOS:\s*)(.*)$/);
  if (!match) {
    say(`✗ malformed EXPECT_REPOS: line in ${workflowPath}`);
    process.exit(2);
  }
  return { lines, idx, lead: match[1], segments: match[2].split(","), hadCr };
}

// Dual-update: for each entry, GET releases/latest from the live API
// (fix mode re-GETs instead of consuming check-mode artifacts — the
// modes stay independently runnable and the fix never trusts a stale
// handoff file). Per-entry outcomes: 200 → rewrite both surfaces to the
// live tag (idempotent-convergent: rewritten whether or not it
// mismatched, so a hand-broken README cell heals in the same run); 404 →
// skip untouched (the issue path owns it); a 200 whose anchored cell
// shape matches nowhere in the README, or whose env segment is missing
// from the workflow line → skip untouched with a loud skip line (check
// mode routes that state to missing-cell, issue-only, and the fixer
// cannot recreate a deleted row — skipping keeps the workflow green so
// the issue step still alarms); anything else → abort. All replacements
// are computed in memory before any write, so an abort leaves both
// files byte-identical to their pre-run state.
async function runFixMode(readmePath, workflowPath) {
  const envFindings = [];
  const entries = parseExpectRepos(envFindings);
  if (envFindings.length > 0) {
    say(`${envFindings.length} problem(s):`);
    for (const f of envFindings) printFinding(f);
    process.exit(2);
  }

  let readmeText;
  try {
    readmeText = readFileSync(readmePath, "utf-8");
  } catch (err) {
    say(`✗ cannot read README ${readmePath}: ${err.message}`);
    process.exit(2);
  }
  let workflowText;
  try {
    workflowText = readFileSync(workflowPath, "utf-8");
  } catch (err) {
    say(`✗ cannot read workflow ${workflowPath}: ${err.message}`);
    process.exit(2);
  }
  const envLine = readEnvLine(workflowText, workflowPath);

  let readmeResult = readmeText;
  for (const entry of entries) {
    const r = await releasesLatest(entry.owner, entry.repo);
    if (r.error) {
      say(
        `✗ ${entry.owner}/${entry.repo} releases/latest: fetch failed (${errorMessage(r.error)}) (${r.url})`,
      );
      process.exit(2);
    }
    if (r.status === 404) {
      say(
        `skip ${entry.owner}/${entry.repo}: releases/latest answered 404 — the issue path owns it`,
      );
      continue;
    }
    if (r.status !== 200) {
      say(
        `✗ ${entry.owner}/${entry.repo} releases/latest: HTTP ${r.status} (${r.url})`,
      );
      process.exit(2);
    }
    if (typeof r.tag !== "string" || r.tag === "") {
      say(
        `✗ ${entry.owner}/${entry.repo} releases/latest: no tag_name in the response (${r.url})`,
      );
      process.exit(2);
    }
    const live = r.tag;

    const segIdx = envLine.segments.findIndex((segment) =>
      segment.trim().startsWith(`${entry.owner}/${entry.repo}@`),
    );
    if (segIdx === -1) {
      say(
        `skip ${entry.owner}/${entry.repo}: no ${entry.owner}/${entry.repo}@ segment in the EXPECT_REPOS: line of ${workflowPath}`,
      );
      continue;
    }
    const pattern = cellPattern(entry);
    if (!new RegExp(pattern).test(readmeResult)) {
      // Check mode routes this state to missing-cell (issue-only): the
      // fixer rewrites existing cells and cannot recreate a deleted
      // row, so it leaves both of this entry's surfaces untouched.
      say(
        `skip ${entry.owner}/${entry.repo}: no anchored release cell in ${readmePath} — the issue path owns it`,
      );
      continue;
    }
    // A replacer function, not a replacement string: cellPattern has a
    // capture group, and a string replacement would expand $&/$1/$'/$`
    // sequences in the live tag — tags are arbitrary strings.
    readmeResult = readmeResult.replace(
      new RegExp(pattern, "g"),
      () => cellReplacement(entry, live),
    );
    envLine.segments[segIdx] = `${entry.owner}/${entry.repo}@${live}`;
    say(`converged ${entry.owner}/${entry.repo} to ${live}`);
  }

  envLine.lines[envLine.idx] =
    envLine.lead + envLine.segments.join(",") + (envLine.hadCr ? "\r" : "");
  const workflowResult = envLine.lines.join("\n");

  try {
    writeFileSync(readmePath, readmeResult);
    writeFileSync(workflowPath, workflowResult);
  } catch (err) {
    say(`✗ write failed: ${err?.message ?? err}`);
    process.exit(2);
  }
  say("ok");
  process.exit(0);
}

// ── Check mode ──────────────────────────────────────────────────────────────

async function runCheckMode(readmePath, reportPath, summaryPath) {
  const readme = readReadme(readmePath, summaryPath);

  const envFindings = [];
  const entries = parseExpectRepos(envFindings);

  const rows = [];
  const tasks = [];
  for (const entry of entries) {
    tasks.push(async () => checkRelease(entry, rows));
  }
  for (const url of readme.urls) {
    tasks.push(async () => checkLiveness(url, rows));
  }

  const results = await Promise.all(tasks.map((task) => task()));
  const findings = [
    ...envFindings,
    ...checkPresence(entries, readme.text, readmePath, rows),
    ...results.flat(),
  ];

  if (summaryPath) {
    appendSummary(summaryPath, rows, summaryEnvValues(), findings);
  }

  if (findings.length === 0) {
    say("ok");
    process.exit(0);
  }
  say(`${findings.length} problem(s):`);
  for (const f of findings) {
    printFinding(f);
  }

  // The report exists if and only if issue-only drift exists: the fix PR
  // is the machine-fixable alarm, and the workflow's report guard keeps
  // the issue step silent without it.
  const issueOnly = findings.filter(
    (f) => f.severity === "drift" && ISSUE_ONLY_CLASSES.has(f.cls),
  );
  if (reportPath && issueOnly.length > 0) {
    try {
      writeFileSync(reportPath, renderReport(issueOnly));
    } catch (err) {
      say(`cannot write report ${reportPath}: ${err.message}`);
      process.exit(2);
    }
  }
  const hasUnavailable = findings.some((f) => f.severity === "unavailable");
  process.exit(hasUnavailable ? 2 : 1);
}

// ── Entry point ─────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const opts = {
    readme: join(ROOT, "README.md"),
    workflow: join(ROOT, ".github/workflows/comparison-freshness.yml"),
    report: null,
    summary: null,
    fix: false,
    reportIssue: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--fix") {
      opts.fix = true;
      continue;
    }
    if (
      arg !== "--readme" &&
      arg !== "--report" &&
      arg !== "--summary" &&
      arg !== "--workflow" &&
      arg !== "--report-issue"
    ) {
      say(`usage: unknown argument ${arg}`);
      say(`usage: ${USAGE}`);
      process.exit(2);
    }
    const value = argv[i + 1];
    if (!value || value.startsWith("--")) {
      say(`usage: ${arg} requires a path`);
      say(`usage: ${USAGE}`);
      process.exit(2);
    }
    if (arg === "--readme") opts.readme = value;
    else if (arg === "--workflow") opts.workflow = value;
    else if (arg === "--report") opts.report = value;
    else if (arg === "--summary") opts.summary = value;
    else opts.reportIssue = value;
    i++;
  }
  if (opts.fix && opts.reportIssue) {
    say("usage: --fix and --report-issue cannot be combined");
    say(`usage: ${USAGE}`);
    process.exit(2);
  }
  return opts;
}

try {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.reportIssue) {
    await runIssueMode(opts.reportIssue);
  } else if (opts.fix) {
    await runFixMode(opts.readme, opts.workflow);
  } else {
    await runCheckMode(opts.readme, opts.report, opts.summary);
  }
} catch (err) {
  say(`unexpected failure: ${err?.message ?? err}`);
  process.exit(2);
}
