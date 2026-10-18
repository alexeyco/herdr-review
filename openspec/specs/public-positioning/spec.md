# public-positioning Specification

## Purpose

The public-positioning capability is the README `## Comparison` section:
what it contains, where it sits, how every factual cell is sourced, and
what the plugin may claim about itself relative to the five competitor
columns. It exists so the positioning stays factual, neutral, and
auditable — and so the comparison-freshness mechanism has a
machine-checkable surface to guard.

## Requirements

### Requirement: README carries a sourced, dated Comparison section

The README SHALL carry a `## Comparison` section placed directly after
the `## Quickstart` section and before `## The flow` and `## Why` — the
`##` heading order is Quickstart, Comparison, The flow, Why, with every
other heading unchanged. Inside the section, in this order:

1. one intro sentence and the legend line
   `✅ documented in the linked sources · ⚠️ partial or implemented differently · ❌ not found in the public sources checked`;
2. the at-a-glance matrix: 7 axis rows × 6 columns (this plugin plus the
   five competitors reviewr, hunk-diff, differ, quantk, lasso), each
   competitor column header linking its GitHub repo, cells carrying
   ✅/⚠️/❌;
3. the numbered footnote block whose definitions carry the public source
   links, markers unique and numbered in reading order;
4. the dated line
   `All cells checked 2026-10-18 against the linked public sources.`;
5. a `<details>` block summarized `Detailed comparison` holding the
   detailed table with the rows Delivery form, Underlying tool, Latest
   release, and License / language over the same six columns;
6. the positioning paragraph.

The seven matrix axes SHALL be, in order: ships a herdr-plugin manifest
(installs via `herdr plugin install`); human-authored line annotations;
dedicated git-diff review surface; delivers on close, no extra keypress
(annotations reach the agent pane when the review closes — key- or
action-triggered delivery is ⚠️); agent-agnostic delivery; no LLM keys or
accounts in the core flow; user-selectable underlying review tool.

In the detailed table's Latest release row, each of the five competitor
cells SHALL carry exactly the shape
`[<tag>](https://github.com/<owner>/<repo>/releases/tag/<tag>)`, the cell
tag equal to the tag currently pinned for that repo in the workflow env's
`EXPECT_REPOS` line — the dual-update keeps cell and env in step; the cell
SHALL carry no release date (dates are not machine-checked and would rot
silently). The subject column's cell SHALL link the repo's releases page
without a pinned tag — the subject is deliberately unpinned, so there is
no machine-checkable self release fact and none is a freshness
expectation.

#### Scenario: Heading order puts Comparison after Quickstart

- **WHEN** the rendered README's `##` headings are read top to bottom
- **THEN** `## Comparison` appears after the `## Quickstart` section and
  before `## The flow` and `## Why`, and no other heading moved

#### Scenario: Section anatomy in fixed order

- **WHEN** a reader opens the `## Comparison` section
- **THEN** its parts appear in the order: intro + legend line, matrix,
  footnote block, dated `All cells checked 2026-10-18 …` line,
  `<details>` "Detailed comparison" table, positioning paragraph

#### Scenario: Seven axes across six columns

- **WHEN** the matrix is read
- **THEN** it holds exactly the seven axis rows named above over six
  columns — this plugin plus reviewr, hunk-diff, differ, quantk, lasso —
  each competitor header a link to its GitHub repo, and every cell one of
  ✅/⚠️/❌ (with a footnote marker where the requirement "Every factual
  cell is sourced and the legend defines the states" demands one)

#### Scenario: Latest-release cells carry the machine-checkable shape

- **WHEN** the detailed table's Latest release row is inspected
- **THEN** each competitor cell is exactly
  `[<tag>](https://github.com/<owner>/<repo>/releases/tag/<tag>)` — one
  case-sensitive substring per entry carrying repo and tag together —
  with no release date in the cell, and the subject cell links
  `https://github.com/alexeyco/herdr-review/releases` with no pinned tag

### Requirement: Every factual cell is sourced and the legend defines the states

Every factual cell of both tables SHALL carry a source that supports it:
a uniquely numbered footnote whose definition carries a public link (the
competitor's GitHub repo, or — for file-level evidence — a tag-pinned
blob URL at that repo), or, for bare ❌ matrix cells, the legend's
`not found in the public sources checked` rule: a bare ❌ states that the
feature is not documented in the project's public sources, negative
searches performed 2026-10-18, and is a sourced negative claim under the
dated line. Every ⚠️ cell and every ✅/❌ cell whose evidence is not
self-evident from the linked repo root SHALL carry a footnote; markers
SHALL be unique and numbered in reading order, none reused — identical
evidence used at two positions yields two numbered definitions with
identical text. The dated line SHALL sit directly under the footnote
block and cover both tables; it records the human audit and is never
edited by the freshness bot.

#### Scenario: Legend defines the three states

- **WHEN** a reader sees ✅, ⚠️, or ❌ cells in the matrix
- **THEN** the legend line directly above the matrix defines all three
  states, and the ❌ wording states the bare-❌ rule that keeps
  unfootnoted ❌ cells sourced negative claims — the search date
  (2026-10-18) is carried by the dated line under the footnote block,
  not by the legend text itself

#### Scenario: Questioned cell carries its source

- **WHEN** any factual cell — matrix or detailed — is questioned
- **THEN** it carries a public source: a uniquely numbered footnote whose
  definition links the competitor's GitHub repo or a tag-pinned blob URL,
  a direct external link in the detailed cell, or — for a bare ❌ matrix
  cell — the legend's not-found rule with the dated line; no cell rests
  on an uncited claim

### Requirement: Comparison stays neutral and claims only the defensible combination

The section SHALL use neutral, factual language — no evaluative verdicts
("better", "worse", "inferior"); absences are stated as sourced facts
linked to the competitor's public sources. The positioning paragraph's
uniqueness claim SHALL be limited to the combination of (1) annotation
delivery into the agent pane on review close with no extra keypress and
(2) the user-selectable underlying review tool — the two axes where the
matrix shows this plugin as the only ✅ (all five competitors deliver by
key or action and ship a single built-in viewer). No broader "only tool
that …" statement SHALL appear, and no single-axis uniqueness SHALL be
claimed beyond what the matrix cells evidence.

#### Scenario: Unique-combination claim only

- **WHEN** the positioning paragraph asserts what distinguishes this
  plugin
- **THEN** the claim names exactly the two-part combination above, is
  phrased as a combination, and no other axis is claimed as unique

#### Scenario: Absence stated neutrally

- **WHEN** the tables record what a competitor lacks — a ❌ or ⚠️ cell,
  or a detailed cell such as differ shipping no herdr-plugin manifest
- **THEN** the absence is stated as a fact linked to the competitor's
  public source or covered by the legend's not-found rule, and no
  evaluative adjective appears anywhere in the section
