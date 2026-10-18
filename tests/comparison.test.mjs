// Tests for scripts/check-comparison.mjs — check, --fix,
// --report-issue, and --summary modes against an offline fixture
// harness (design.md "Test strategy").
//
// One node:http server on 127.0.0.1 plays the GitHub API seam
// (COMPARISON_GITHUB_API) and every fixture-README liveness URL. Route
// tables are per-test; a route registered with several handlers answers
// successive requests in order (the 429-then-200 and 5xx retry shapes),
// a single handler answers every request. The request log records
// bodies and the Authorization header. The unreachable case points the
// seam at a bound-then-closed port.
//
// Fixture READMEs are tiny table-markup-free documents: presence checks
// are substring searches, so the release-cell text appears WITHOUT an
// http scheme and therefore never joins the liveness URL set — check
// mode stays fully offline. Fixture workflow files carry a single
// EXPECT_REPOS: line. Every case spawns the script as a child process
// with --readme/--workflow/--report/--summary/--report-issue always
// pointing into a temp dir; the repo-root files are never touched.
//
// Run: node --test tests/comparison.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import net from "node:net";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "scripts",
  "check-comparison.mjs",
);
const PREFIX = "comparison-check:";

// Scratch space for fixture files: the session's pre-created
// <tmpdir>/opencode directory when present (the approved external
// location in sandboxed sessions), else the system tmpdir itself.
const opencodeDir = join(tmpdir(), "opencode");
const TMP_BASE = existsSync(opencodeDir) ? opencodeDir : tmpdir();

// The drift report issue mode consumes — shaped like check mode's
// --report output, kept small on purpose: issue mode only forwards it.
const REPORT_BODY = [
  "The monthly comparison-freshness check found drift in the README",
  "`## Comparison` section (run 2026-10-18T00:00:00.000Z, GitHub run 424242)",
  "that the fixer must not rewrite itself.",
  "",
  "| Class | Expectation | Expected | Found | Source |",
  "| ----- | ----------- | -------- | ----- | ------ |",
  "| missing-cell | EXPECT_REPOS beta/two@v1.0.0 | github.com/beta/two/releases/tag/v1.0.0 | no release cell | missing.md |",
  "",
  "Fix: these findings are not machine-fixable — update the README by",
  "hand, then close this issue; the next monthly run re-files it if",
  "drift persists.",
  "",
].join("\n");

// ── Fixture harness ─────────────────────────────────────────────────────────

function respond(req, res, spec) {
  const { status, json, headers = {} } = spec;
  if (json === undefined) {
    res.writeHead(status, headers);
    res.end();
    return;
  }
  const payload = JSON.stringify(json);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
    ...headers,
  });
  res.end(req.method === "HEAD" ? undefined : payload);
}

async function withFixture(t, run) {
  const ctx = { routes: new Map(), log: [] };
  const server = http.createServer((req, res) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      ctx.log.push({
        method: req.method,
        url: req.url,
        body,
        authorization: req.headers.authorization ?? null,
      });
      const queue = ctx.routes.get(`${req.method} ${req.url}`);
      const handler =
        queue === undefined
          ? null
          : queue.length > 1
            ? queue.shift()
            : queue[0];
      if (handler == null) {
        respond(req, res, {
          status: 404,
          json: { message: `no fixture route for ${req.method} ${req.url}` },
        });
        return;
      }
      if (typeof handler === "function") {
        handler(req, res);
        return;
      }
      respond(req, res, handler);
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  ctx.port = server.address().port;
  ctx.api = `http://127.0.0.1:${ctx.port}`;
  ctx.tmp = mkdtempSync(join(TMP_BASE, "comparison-test-"));
  t.after(() => {
    server.close();
    server.closeAllConnections?.();
    rmSync(ctx.tmp, { recursive: true, force: true });
  });
  await run(ctx);
}

function route(ctx, method, url, ...handlers) {
  ctx.routes.set(`${method} ${url}`, handlers);
}

function fixture(ctx, name, content) {
  const path = join(ctx.tmp, name);
  writeFileSync(path, content);
  return path;
}

// The anchored release-cell markdown link, parameterized by repo and
// tag — the exact shape --fix rewrites.
function cell(repo, tag) {
  return `[${tag}](https://github.com/${repo}/releases/tag/${tag})`;
}

function requests(ctx, method, url) {
  return ctx.log.filter(
    (entry) => entry.method === method && entry.url === url,
  );
}

// A port that was bound and then closed: connects are refused, the
// transport-failure shape for the unreachable-seam cases.
async function closedPortApi() {
  const server = net.createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return `http://127.0.0.1:${port}`;
}

// Async spawn on purpose: the child's fetches must be answered by this
// process's fixture server while the child runs — a synchronous spawn
// would block the parent's event loop and deadlock against the server
// until every child request timed out.
function runScript(ctx, args, env = {}) {
  const childEnv = { ...process.env };
  for (const name of [
    "EXPECT_REPOS",
    "COMPARISON_GITHUB_API",
    "GITHUB_TOKEN",
    "GITHUB_REPOSITORY",
    "GITHUB_RUN_ID",
    "GITHUB_SERVER_URL",
  ]) {
    delete childEnv[name];
  }
  childEnv.COMPARISON_GITHUB_API = ctx.api;
  Object.assign(childEnv, env);
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [SCRIPT, ...args], {
      env: childEnv,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", (status) => resolve({ status, stdout, stderr }));
  });
}

// The summary text between one class heading and the next heading.
function summarySection(content, heading) {
  const start = content.indexOf(heading);
  assert.notEqual(start, -1, `summary is missing ${heading}`);
  const after = content.slice(start + heading.length);
  const next = after.indexOf("\n## ");
  return next === -1 ? after : after.slice(0, next);
}

// ── Check mode ───────────────────────────────────────────────────────────────

test("clean run exits zero (matching tag, alive link, present cell)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.5.0" },
    });
    route(ctx, "HEAD", "/docs", { status: 200 });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0",
        "",
        `docs: ${ctx.api}/docs`,
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0",
    });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trimEnd(), `${PREFIX} ok`);
  });
});

test("a tag mismatch is machine-fixable drift (exit 1, no report)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.6.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0",
    });
    assert.equal(out.status, 1);
    assert.match(
      out.stdout,
      new RegExp(
        "✗ release-drift EXPECT_REPOS jhochenbaum/herdr-hunk-diff@v0\\.5\\.0: " +
          "expected v0\\.5\\.0, found v0\\.6\\.0 " +
          "\\(http://127\\.0\\.0\\.1:\\d+/repos/jhochenbaum/herdr-hunk-diff/releases/latest\\)",
      ),
    );
    assert.ok(
      !existsSync(report),
      "machine-fixable-only drift must write no report",
    );
  });
});

test("a 404 release endpoint is issue-only drift (repo-gone, report written)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 404,
      json: { message: "Not Found" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 1);
    assert.ok(out.stdout.includes("repo-gone"));
    assert.ok(existsSync(report), "issue-only drift writes the report");
    const body = readFileSync(report, "utf8");
    assert.ok(body.includes("repo-gone"));
    assert.ok(body.includes("alpha/one@v1.0.0"));
    // One GET per entry and no separate existence GET.
    assert.equal(
      requests(ctx, "GET", "/repos/alpha/one/releases/latest").length,
      1,
    );
  });
});

test("a rate-limit 403 is unavailable, not drift (exit 2, no report)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 403,
      json: { message: "API rate limit exceeded" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 2);
    assert.ok(out.stdout.includes("HTTP 403"));
    assert.ok(out.stdout.includes("unavailable"));
    assert.ok(!out.stdout.includes("release-drift"));
    assert.ok(!existsSync(report));
  });
});

test("a dead link is issue-only drift (dead-link, report written)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    route(ctx, "HEAD", "/dead", { status: 404 });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
        `docs: ${ctx.api}/dead`,
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 1);
    assert.match(
      out.stdout,
      new RegExp(
        `✗ dead-link ${ctx.api}/dead: expected alive \\(2xx\\), found 404 \\(${ctx.api}/dead\\)`,
      ),
    );
    assert.ok(existsSync(report));
    assert.ok(readFileSync(report, "utf8").includes("dead-link"));
  });
});

test("a moved URL is issue-only drift (moved-url, report written)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    route(ctx, "HEAD", "/moved", {
      status: 301,
      headers: { Location: `${ctx.api}/moved-elsewhere` },
    });
    route(ctx, "HEAD", "/moved-elsewhere", { status: 200 });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
        `guide: ${ctx.api}/moved`,
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 1);
    assert.match(
      out.stdout,
      new RegExp(
        `✗ moved-url ${ctx.api}/moved: expected ${ctx.api}/moved, found ${ctx.api}/moved-elsewhere \\(${ctx.api}/moved\\)`,
      ),
    );
    assert.ok(existsSync(report));
    assert.ok(readFileSync(report, "utf8").includes("moved-url"));
  });
});

test("a redirect differing only by a trailing slash is alive", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    route(ctx, "HEAD", "/slash", {
      status: 301,
      headers: { Location: `${ctx.api}/slash/` },
    });
    route(ctx, "HEAD", "/slash/", { status: 200 });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
        `docs: ${ctx.api}/slash`,
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trimEnd(), `${PREFIX} ok`);
  });
});

test("a stale release cell is machine-fixable presence drift (no report)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/quantk/herdr-review/releases/latest", {
      status: 200,
      json: { tag_name: "v0.2.7" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/quantk/herdr-review/releases/tag/v0.9.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "quantk/herdr-review@v0.2.7",
    });
    assert.equal(out.status, 1);
    assert.ok(out.stdout.includes("stale-cell"));
    assert.ok(
      !existsSync(report),
      "machine-fixable-only drift must write no report",
    );
  });
});

test("a missing release cell is issue-only presence drift (report written)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/quantk/herdr-review/releases/latest", {
      status: 200,
      json: { tag_name: "v0.2.7" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/someone/else/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS: "quantk/herdr-review@v0.2.7",
    });
    assert.equal(out.status, 1);
    assert.ok(out.stdout.includes("missing-cell"));
    assert.ok(out.stdout.includes("quantk/herdr-review@v0.2.7"));
    assert.ok(existsSync(report));
    const body = readFileSync(report, "utf8");
    assert.ok(body.includes("missing-cell"));
    assert.ok(body.includes("quantk/herdr-review@v0.2.7"));
  });
});

test("presence checks are substring, not structural (reformatted row)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.5.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "|      hunk-diff      |        v0.5.0    →    github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0        |      ✅      |",
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0",
    });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trimEnd(), `${PREFIX} ok`);
  });
});

test("malformed and unset EXPECT_REPOS entries are loud, never skipped", async (t) => {
  await withFixture(t, async (ctx) => {
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const cases = [
      { value: "jhochenbaum/herdr-hunk-diff", entry: "jhochenbaum/herdr-hunk-diff" },
      { value: "skellleks@v0.1.0", entry: "skellleks@v0.1.0" },
      { value: "quantk/herdr-review@", entry: "quantk/herdr-review@" },
      { value: "", entry: null },
      { value: undefined, entry: null },
    ];
    for (const { value, entry } of cases) {
      const env = value === undefined ? {} : { EXPECT_REPOS: value };
      const out = await runScript(ctx, ["--readme", readme], env);
      assert.equal(out.status, 2, `EXPECT_REPOS=${JSON.stringify(value)}`);
      assert.ok(out.stdout.includes("✗"));
      assert.ok(out.stdout.includes("EXPECT_REPOS"));
      if (entry !== null) {
        assert.ok(
          out.stdout.includes(entry),
          `finding must name the offending entry ${entry}`,
        );
      }
    }
    assert.equal(
      ctx.log.length,
      0,
      "a malformed env string issues no request — it fails loudly",
    );

    // An empty segment (trailing comma) is malformed next to a valid
    // entry: the run still exits 2, never skipping the bad segment.
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "alpha/one@v1.0.0,",
    });
    assert.equal(out.status, 2);
    assert.ok(out.stdout.includes('EXPECT_REPOS ""'));
  });
});

test("an unreachable source exits two (closed-port seam)", async (t) => {
  await withFixture(t, async (ctx) => {
    const deadApi = await closedPortApi();
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
      COMPARISON_GITHUB_API: deadApi,
    });
    assert.equal(out.status, 2);
    assert.ok(out.stdout.includes("✗"));
    assert.ok(
      out.stdout.includes(
        `${deadApi}/repos/alpha/one/releases/latest`,
      ),
      "the ✗ line must name the unreachable source URL",
    );
  });
});

test("mixed drift and unavailable findings exit two with both printed", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.6.0" },
    });
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 403,
      json: { message: "API rate limit exceeded" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "hunk-diff: github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0",
        "alpha: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const report = join(ctx.tmp, "report.md");
    const out = await runScript(ctx, ["--readme", readme, "--report", report], {
      EXPECT_REPOS:
        "jhochenbaum/herdr-hunk-diff@v0.5.0,alpha/one@v1.0.0",
    });
    assert.equal(out.status, 2);
    assert.ok(out.stdout.includes("release-drift"));
    assert.ok(out.stdout.includes("HTTP 403"));
    assert.ok(
      !existsSync(report),
      "no issue-only drift in this mix, so no report",
    );
  });
});

test("a 429-then-200 release GET makes exactly two requests and runs clean", async (t) => {
  await withFixture(t, async (ctx) => {
    route(
      ctx,
      "GET",
      "/repos/alpha/one/releases/latest",
      { status: 429, json: { message: "rate limited" } },
      { status: 200, json: { tag_name: "v1.0.0" } },
    );
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trimEnd(), `${PREFIX} ok`);
    assert.equal(ctx.log.length, 2, "one attempt plus exactly one retry");
  });
});

test("GITHUB_TOKEN is sent as a Bearer header on the release GET", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const out = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
      GITHUB_TOKEN: "fixture-bearer-token",
    });
    assert.equal(out.status, 0);
    const releaseGets = requests(
      ctx,
      "GET",
      "/repos/alpha/one/releases/latest",
    );
    assert.equal(releaseGets.length, 1);
    assert.equal(
      releaseGets[0].authorization,
      "Bearer fixture-bearer-token",
    );
  });
});

// ── Fix mode ─────────────────────────────────────────────────────────────────

test("fix updates the README cell and the env line together", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.6.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        `hunk-diff: ${cell("jhochenbaum/herdr-hunk-diff", "v0.5.0")}`,
        "",
      ].join("\n"),
    );
    const workflow = fixture(
      ctx,
      "workflow.yml",
      [
        "name: fixture",
        "jobs:",
        "  check:",
        "    env:",
        "      EXPECT_REPOS: jhochenbaum/herdr-hunk-diff@v0.5.0",
        "",
      ].join("\n"),
    );
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0" },
    );
    assert.equal(out.status, 0);
    assert.ok(
      readFileSync(readme, "utf8").includes(
        cell("jhochenbaum/herdr-hunk-diff", "v0.6.0"),
      ),
      "the README cell carries the live tag in the anchored shape",
    );
    assert.ok(
      readFileSync(workflow, "utf8").includes(
        "EXPECT_REPOS: jhochenbaum/herdr-hunk-diff@v0.6.0",
      ),
      "the env line carries the live tag",
    );
  });
});

test("fix never touches matrix cells or the checked-date line", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.6.0" },
    });
    const dateLine =
      "All cells checked 2026-10-18 against the linked public sources.";
    const hunkCell = (tag) => cell("jhochenbaum/herdr-hunk-diff", tag);
    const original = [
      "# fixture readme",
      "",
      "| Axis | herdr-review | hunk-diff |",
      "| --- | --- | --- |",
      "| Delivers on close | ✅ | ⚠️ `s` → pane.send_text, unsubmitted |",
      "| Agent-agnostic | ✅ | ✅ |",
      "",
      dateLine,
      "",
      "| Delivery form | plugin | plugin |",
      "| Latest release | [releases](https://github.com/alexeyco/herdr-review/releases) | " +
        `${hunkCell("v0.5.0")} |`,
      "| License | MIT | MIT |",
      "",
      "Footnote: evidence at the [tag-pinned file]" +
        "(https://github.com/jhochenbaum/herdr-hunk-diff/blob/v0.5.0/src/herdr.ts).",
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: jhochenbaum/herdr-hunk-diff@v0.5.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0" },
    );
    assert.equal(out.status, 0);
    const fixed = readFileSync(readme, "utf8");
    assert.equal(
      fixed,
      original.replace(hunkCell("v0.5.0"), hunkCell("v0.6.0")),
      "every byte outside the anchored release cell is unchanged",
    );
    assert.ok(fixed.includes(dateLine));
    assert.ok(fixed.includes("⚠️"));
    assert.equal(
      readFileSync(workflow, "utf8"),
      workflowOriginal.replace(
        "jhochenbaum/herdr-hunk-diff@v0.5.0",
        "jhochenbaum/herdr-hunk-diff@v0.6.0",
      ),
    );
  });
});

test("fix skips a 404 entry and converges the others", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 404,
      json: { message: "Not Found" },
    });
    route(ctx, "GET", "/repos/beta/two/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    const original = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v0.5.0")}`,
      `beta: ${cell("beta/two", "v1.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: alpha/one@v0.5.0,beta/two@v1.0.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v0.5.0,beta/two@v1.0.0" },
    );
    assert.equal(out.status, 0);
    assert.ok(out.stdout.includes("skip"));
    assert.ok(out.stdout.includes("alpha/one"));
    assert.equal(
      readFileSync(readme, "utf8"),
      original.replace(cell("beta/two", "v1.0.0"), cell("beta/two", "v2.0.0")),
      "the 404 entry's cell stays untouched, the other converges",
    );
    assert.equal(
      readFileSync(workflow, "utf8"),
      workflowOriginal.replace(
        "alpha/one@v0.5.0,beta/two@v1.0.0",
        "alpha/one@v0.5.0,beta/two@v2.0.0",
      ),
    );
  });
});

test("fix skips an unmatchable cell and the run stays green", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    route(ctx, "GET", "/repos/beta/two/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    // beta/two has no anchored cell anywhere — check mode would call
    // this missing-cell; the fixer must skip it, not abort.
    const original = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v1.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: alpha/one@v1.0.0,beta/two@v1.0.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v1.0.0,beta/two@v1.0.0" },
    );
    assert.equal(out.status, 0);
    assert.ok(out.stdout.startsWith(PREFIX));
    assert.ok(out.stdout.includes("skip"));
    assert.ok(out.stdout.includes("beta/two"));
    assert.equal(
      readFileSync(readme, "utf8"),
      original.replace(cell("alpha/one", "v1.0.0"), cell("alpha/one", "v2.0.0")),
    );
    assert.equal(
      readFileSync(workflow, "utf8"),
      workflowOriginal.replace(
        "alpha/one@v1.0.0,beta/two@v1.0.0",
        "alpha/one@v2.0.0,beta/two@v1.0.0",
      ),
      "the unmatchable entry's env segment stays untouched",
    );
  });
});

test("fix is atomic when a live source is unreachable", async (t) => {
  await withFixture(t, async (ctx) => {
    // alpha/one would converge; beta/two's endpoint dies on transport
    // (the connection is reset mid-request — the per-entry flavor of a
    // closed port; COMPARISON_GITHUB_API is one base URL shared by all
    // entries, so a literal closed port kills every entry at once).
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    route(ctx, "GET", "/repos/beta/two/releases/latest", (req, res) => {
      res.socket.destroy();
    });
    const original = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v1.0.0")}`,
      `beta: ${cell("beta/two", "v1.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: alpha/one@v1.0.0,beta/two@v1.0.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);

    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v1.0.0,beta/two@v1.0.0" },
    );
    assert.equal(out.status, 2);
    assert.equal(readFileSync(readme, "utf8"), original);
    assert.equal(readFileSync(workflow, "utf8"), workflowOriginal);

    // The whole seam on a literal closed port: same atomic outcome.
    const deadApi = await closedPortApi();
    const deadOut = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      {
        EXPECT_REPOS: "alpha/one@v1.0.0,beta/two@v1.0.0",
        COMPARISON_GITHUB_API: deadApi,
      },
    );
    assert.equal(deadOut.status, 2);
    assert.equal(readFileSync(readme, "utf8"), original);
    assert.equal(readFileSync(workflow, "utf8"), workflowOriginal);
  });
});

test("a missing EXPECT_REPOS line (or workflow file) is loud", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    const readmeOriginal = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v1.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", readmeOriginal);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      OTHER: x",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);

    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v1.0.0" },
    );
    assert.equal(out.status, 2);
    assert.ok(out.stdout.includes("✗"));
    assert.ok(
      out.stdout.includes(workflow),
      "the ✗ line must name the workflow file",
    );
    assert.equal(readFileSync(readme, "utf8"), readmeOriginal);
    assert.equal(readFileSync(workflow, "utf8"), workflowOriginal);

    const missingWorkflow = join(ctx.tmp, "no-such-workflow.yml");
    const missingOut = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", missingWorkflow],
      { EXPECT_REPOS: "alpha/one@v1.0.0" },
    );
    assert.equal(missingOut.status, 2);
    assert.ok(missingOut.stdout.includes("✗"));
    assert.ok(missingOut.stdout.includes(missingWorkflow));
    assert.equal(readFileSync(readme, "utf8"), readmeOriginal);
  });
});

test("converged input leaves both files unchanged and exits zero", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    const original = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v2.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: alpha/one@v2.0.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v2.0.0" },
    );
    assert.equal(out.status, 0);
    assert.equal(readFileSync(readme, "utf8"), original);
    assert.equal(readFileSync(workflow, "utf8"), workflowOriginal);
  });
});

test("fix heals a hand-broken README cell when env and live agree", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v2.0.0" },
    });
    const original = [
      "# fixture readme",
      "",
      `alpha: ${cell("alpha/one", "v1.0.0")}`,
      "",
    ].join("\n");
    const readme = fixture(ctx, "README.md", original);
    const workflowOriginal = [
      "name: fixture",
      "jobs:",
      "  check:",
      "    env:",
      "      EXPECT_REPOS: alpha/one@v2.0.0",
      "",
    ].join("\n");
    const workflow = fixture(ctx, "workflow.yml", workflowOriginal);
    const out = await runScript(
      ctx,
      ["--fix", "--readme", readme, "--workflow", workflow],
      { EXPECT_REPOS: "alpha/one@v2.0.0" },
    );
    assert.equal(out.status, 0);
    assert.equal(
      readFileSync(readme, "utf8"),
      original.replace(cell("alpha/one", "v1.0.0"), cell("alpha/one", "v2.0.0")),
      "idempotent-convergent: a 200 answer rewrites the cell even when the env pin already matches",
    );
    assert.equal(readFileSync(workflow, "utf8"), workflowOriginal);
  });
});

// ── Issue mode ───────────────────────────────────────────────────────────────

const ISSUE_ENV = {
  GITHUB_TOKEN: "fixture-issue-token",
  GITHUB_REPOSITORY: "acme/widget",
};
const ISSUES_SEARCH = "/repos/acme/widget/issues?labels=comparison-drift&state=open";

test("repeat drift updates the same issue (PATCH, no POST)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", ISSUES_SEARCH, {
      status: 200,
      json: [{ number: 42 }],
    });
    route(ctx, "PATCH", "/repos/acme/widget/issues/42", {
      status: 200,
      json: { number: 42 },
    });
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const out = await runScript(ctx, ["--report-issue", report], ISSUE_ENV);
    assert.equal(out.status, 0);
    assert.equal(
      ctx.log.filter((entry) => entry.method === "POST").length,
      0,
      "no issue may be created when an open one exists",
    );
    const search = requests(ctx, "GET", ISSUES_SEARCH);
    assert.equal(search.length, 1);
    assert.equal(search[0].authorization, "Bearer fixture-issue-token");
    const patches = requests(ctx, "PATCH", "/repos/acme/widget/issues/42");
    assert.equal(patches.length, 1);
    assert.ok(ctx.log.indexOf(search[0]) < ctx.log.indexOf(patches[0]));
    assert.equal(JSON.parse(patches[0].body).body, REPORT_BODY);
    assert.equal(patches[0].authorization, "Bearer fixture-issue-token");
  });
});

test("issue creation ensures the label first (422 already-exists)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", ISSUES_SEARCH, { status: 200, json: [] });
    route(ctx, "POST", "/repos/acme/widget/labels", {
      status: 422,
      json: { message: "Validation Failed" },
    });
    route(ctx, "POST", "/repos/acme/widget/issues", {
      status: 201,
      json: { number: 7 },
    });
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const out = await runScript(ctx, ["--report-issue", report], ISSUE_ENV);
    assert.equal(out.status, 0);
    const labelPost = requests(ctx, "POST", "/repos/acme/widget/labels");
    const issuePost = requests(ctx, "POST", "/repos/acme/widget/issues");
    assert.equal(labelPost.length, 1);
    assert.equal(issuePost.length, 1);
    assert.ok(
      ctx.log.indexOf(labelPost[0]) < ctx.log.indexOf(issuePost[0]),
      "the label POST precedes the issue POST",
    );
    assert.deepEqual(JSON.parse(labelPost[0].body), {
      name: "comparison-drift",
    });
    const payload = JSON.parse(issuePost[0].body);
    assert.equal(payload.title, "README comparison drift");
    assert.deepEqual(payload.labels, ["comparison-drift"]);
    assert.equal(payload.body, REPORT_BODY);
  });
});

test("issue creation ensures the label first (201 created)", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", ISSUES_SEARCH, { status: 200, json: [] });
    route(ctx, "POST", "/repos/acme/widget/labels", {
      status: 201,
      json: { id: 99 },
    });
    route(ctx, "POST", "/repos/acme/widget/issues", {
      status: 201,
      json: { number: 8 },
    });
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const out = await runScript(ctx, ["--report-issue", report], ISSUE_ENV);
    assert.equal(out.status, 0);
    const labelPost = requests(ctx, "POST", "/repos/acme/widget/labels");
    const issuePost = requests(ctx, "POST", "/repos/acme/widget/issues");
    assert.equal(labelPost.length, 1);
    assert.equal(issuePost.length, 1);
    assert.ok(
      ctx.log.indexOf(labelPost[0]) < ctx.log.indexOf(issuePost[0]),
    );
    const payload = JSON.parse(issuePost[0].body);
    assert.equal(payload.title, "README comparison drift");
    assert.deepEqual(payload.labels, ["comparison-drift"]);
  });
});

test("a failing label POST exits 2 with no issue POST", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", ISSUES_SEARCH, { status: 200, json: [] });
    route(ctx, "POST", "/repos/acme/widget/labels", {
      status: 500,
      json: { message: "boom" },
    });
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const out = await runScript(ctx, ["--report-issue", report], ISSUE_ENV);
    assert.equal(out.status, 2);
    assert.equal(
      requests(ctx, "POST", "/repos/acme/widget/issues").length,
      0,
      "no issue may be created when the label POST fails",
    );
    assert.equal(
      requests(ctx, "POST", "/repos/acme/widget/labels").length,
      2,
      "a 5xx is retried exactly once",
    );
  });
});

test("missing GITHUB_TOKEN or GITHUB_REPOSITORY exits 2", async (t) => {
  await withFixture(t, async (ctx) => {
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const noToken = await runScript(ctx, ["--report-issue", report], {
      GITHUB_REPOSITORY: "acme/widget",
    });
    assert.equal(noToken.status, 2);
    assert.ok(noToken.stdout.includes("GITHUB_TOKEN"));
    const noRepository = await runScript(ctx, ["--report-issue", report], {
      GITHUB_TOKEN: "fixture-issue-token",
    });
    assert.equal(noRepository.status, 2);
    assert.ok(noRepository.stdout.includes("GITHUB_REPOSITORY"));
    assert.equal(
      ctx.log.length,
      0,
      "the environment is checked before any REST call",
    );
  });
});

test("a REST 500 on the issue write exits 2 after the single retry", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", ISSUES_SEARCH, { status: 200, json: [] });
    route(ctx, "POST", "/repos/acme/widget/labels", {
      status: 422,
      json: { message: "Validation Failed" },
    });
    route(ctx, "POST", "/repos/acme/widget/issues", {
      status: 500,
      json: { message: "boom" },
    });
    const report = fixture(ctx, "report.md", REPORT_BODY);
    const out = await runScript(ctx, ["--report-issue", report], ISSUE_ENV);
    assert.equal(out.status, 2);
    assert.equal(
      requests(ctx, "POST", "/repos/acme/widget/issues").length,
      2,
      "one attempt plus exactly one retry",
    );
  });
});

// ── Summary ──────────────────────────────────────────────────────────────────

test("a clean run's summary lists every check with verdicts", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    route(ctx, "HEAD", "/docs", { status: 200 });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
        `docs: ${ctx.api}/docs`,
        "",
      ].join("\n"),
    );
    const summary = join(ctx.tmp, "summary.md");
    const out = await runScript(ctx, ["--readme", readme, "--summary", summary], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
    });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trimEnd(), `${PREFIX} ok`);
    const content = readFileSync(summary, "utf8");
    assert.match(
      content,
      /^Checked at: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/m,
    );
    assert.ok(content.includes("alpha/one@v1.0.0"));
    const release = summarySection(content, "## Release checks");
    assert.ok(release.includes("alpha/one"));
    assert.ok(release.includes("v1.0.0"));
    const presence = summarySection(content, "## README presence");
    assert.ok(
      presence.includes("github.com/alpha/one/releases/tag/v1.0.0"),
    );
    assert.ok(presence.includes("yes"));
    const liveness = summarySection(content, "## Link liveness");
    assert.ok(liveness.includes(`${ctx.api}/docs`));
    assert.ok(liveness.includes("alive"));
    assert.ok(
      content.includes(
        "Classification: clean — 3 checks, 0 drift, 0 unavailable",
      ),
    );
  });
});

test("an unavailable run still writes its audit summary", async (t) => {
  await withFixture(t, async (ctx) => {
    const deadApi = await closedPortApi();
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const summary = join(ctx.tmp, "summary.md");
    const out = await runScript(ctx, ["--readme", readme, "--summary", summary], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
      COMPARISON_GITHUB_API: deadApi,
    });
    assert.equal(out.status, 2);
    assert.ok(existsSync(summary), "the audit survives the red run");
    const content = readFileSync(summary, "utf8");
    const release = summarySection(content, "## Release checks");
    assert.ok(release.includes("unavailable"));
    assert.ok(release.includes("fetch failed"));
    const liveness = summarySection(content, "## Link liveness");
    assert.ok(liveness.includes("_No checks ran._"));
    assert.ok(content.includes("Classification: unavailable"));
  });
});

test("the summary never contains credentials", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/alpha/one/releases/latest", {
      status: 200,
      json: { tag_name: "v1.0.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/alpha/one/releases/tag/v1.0.0",
        "",
      ].join("\n"),
    );
    const summary = join(ctx.tmp, "summary.md");
    const out = await runScript(ctx, ["--readme", readme, "--summary", summary], {
      EXPECT_REPOS: "alpha/one@v1.0.0",
      GITHUB_TOKEN: "super-secret-fixture-token",
      GITHUB_REPOSITORY: "secret-org/secret-repo",
    });
    assert.equal(out.status, 0);
    const releaseGets = requests(
      ctx,
      "GET",
      "/repos/alpha/one/releases/latest",
    );
    assert.equal(releaseGets.length, 1);
    assert.equal(
      releaseGets[0].authorization,
      "Bearer super-secret-fixture-token",
      "the token-bearing GET really ran",
    );
    const content = readFileSync(summary, "utf8");
    assert.ok(!content.includes("super-secret-fixture-token"));
    assert.ok(!content.includes("secret-org/secret-repo"));
  });
});

test("without --summary nothing changes", async (t) => {
  await withFixture(t, async (ctx) => {
    route(ctx, "GET", "/repos/jhochenbaum/herdr-hunk-diff/releases/latest", {
      status: 200,
      json: { tag_name: "v0.6.0" },
    });
    const readme = fixture(
      ctx,
      "README.md",
      [
        "# fixture readme",
        "",
        "latest: github.com/jhochenbaum/herdr-hunk-diff/releases/tag/v0.5.0",
        "",
      ].join("\n"),
    );
    const summary = join(ctx.tmp, "summary.md");
    const bare = await runScript(ctx, ["--readme", readme], {
      EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0",
    });
    assert.equal(bare.status, 1);
    assert.ok(!existsSync(summary), "no file without the flag");
    const flagged = await runScript(
      ctx,
      ["--readme", readme, "--summary", summary],
      { EXPECT_REPOS: "jhochenbaum/herdr-hunk-diff@v0.5.0" },
    );
    assert.equal(flagged.status, bare.status);
    assert.equal(flagged.stdout, bare.stdout);
    assert.ok(existsSync(summary));
  });
});

test("an unreadable README exits 2 with the zero-row summary still written", async (t) => {
  await withFixture(t, async (ctx) => {
    const summary = join(ctx.tmp, "summary.md");
    const out = await runScript(
      ctx,
      ["--readme", join(ctx.tmp, "missing.md"), "--summary", summary],
      { EXPECT_REPOS: "alpha/one@v1.0.0" },
    );
    assert.equal(out.status, 2);
    const content = readFileSync(summary, "utf8");
    assert.ok(content.includes("## Release checks"));
    assert.ok(content.includes("## README presence"));
    assert.ok(content.includes("## Link liveness"));
    assert.equal(
      (content.match(/_No checks ran\./g) ?? []).length,
      3,
      "every class heading keeps its no-checks-ran line",
    );
    assert.ok(content.includes("alpha/one@v1.0.0"));
    assert.ok(content.includes("Classification: unavailable"));
  });
});
