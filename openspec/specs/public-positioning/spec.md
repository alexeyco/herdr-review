# Public Positioning

## Purpose

The public-positioning capability is the README `## Comparison` section:
what it contains, where it sits, how every factual cell is sourced, and
what the plugin may claim about itself relative to the five competitor
columns. It exists so the positioning stays factual, neutral, and
auditable — and so the comparison-freshness mechanism has a
machine-checkable surface to guard.

## Requirements

### Requirement: README carries a sourced, dated Comparison section

The README SHALL place `## Comparison` after Quickstart and before The
flow and Why. It SHALL contain an intro and legend, a seven-axis matrix
over six columns, numbered source footnotes, a dated audit line, a
Detailed comparison table, and a positioning paragraph, in that order.
Competitor release cells SHALL use the tag from
`.github/comparison-expected-repos.txt` and link to that release without
a date; the subject cell SHALL link to its unpinned releases page.

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

Both tables SHALL source every factual cell through a uniquely numbered
footnote, a direct link, or the legend's dated not-found rule for bare ❌
cells. Footnote markers SHALL be unique and ordered by appearance. The
dated audit line SHALL follow the footnotes, cover both tables, and remain
untouched by the freshness workflow.

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

The section SHALL use neutral, sourced language and avoid evaluative
claims. Its positioning SHALL claim uniqueness only for the combination
of delivery on review close without another keypress and user-selectable
review tools, as supported by the matrix.

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
