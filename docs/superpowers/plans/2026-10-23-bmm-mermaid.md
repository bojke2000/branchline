# BMM Mermaid Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export the active BMM hierarchy as a local Mermaid mindmap file.

**Architecture:** A pure bounded serializer consumes a validated draft-inclusive session snapshot. The existing command/save-dialog/atomic-write path publishes the captured output without changing the native document.

**Tech Stack:** Rust/serde/existing storage; optional local Node development-only parser verification with Mermaid 12.2.0, never an application runtime dependency.

**Spec:** `docs/superpowers/specs/2026-10-23-bmm-clipboard-export-design.md`, Mermaid export section.

## Global Constraints

- Requires tabs plan session snapshots and stable-target destination ownership checks; independent of clipboard implementation.
- Export only: `.mmd`, UTF-8, `mindmap`; no import, flowcharts, network rendering or metadata/layout preservation.
- Preserve root, source order, labels and folded descendants; generated identifiers are not user labels.
- 32 MiB encoded output; existing 100,000-node, depth-256, field/text bounds.
- Do not commit drafts, clear dirty state, change native paths or add undo steps.
- Retain atomic writes, explicit overwrite and visible errors/cancellations.
- Node >=22.12.0 is required only for the optional Mermaid 12.2.0 parser check; do not install/upgrade Node or unrelated software.
- No implementation until plan and execution method approval.

## Review Focus

- A quote/newline/directive-looking label must remain one topic: Task 1.
- Emoji, literal entity strings and Markdown-like text must remain labels, not disappear during sanitization: Task 1.
- Empty/control labels must not become exposed generated IDs or silent substitutions: Task 1.
- Exporting an unfinished edit must include the draft without committing it: Task 2.
- Choosing an open map's path must not overwrite its native document with Mermaid text: Task 2.

---

## File map and shared interfaces

- Create `src/mermaid.rs`: `export_mermaid(doc: &Document) -> Result<Vec<u8>, FormatError>`.
- Create `tests/mermaid.rs`, including an ignored opt-in actual-parser test;
  extend UI tests for snapshot/nonmutation behavior.
- Modify `src/commands.rs`, `src/ui.rs`, `src/lib.rs`, README and benchmark.
- If the compatible existing Node toolchain is available, create dev-only
  `tools/mermaid/package.json`, `package-lock.json` and `verify.mjs`; never
  bundle their dependencies or add Node to BMM runtime requirements.

## Task 1: Bounded serializer and actual syntax checks

**Files:** Create serializer/tests and optional dev-only parser check above;
modify `src/lib.rs`.

**Produces:** `export_mermaid`; UTF-8 output beginning `mindmap\n`, generated
`bmm_0`, `bmm_1`, ... identifiers in preorder, and two-space indentation per
level. Use explicit quoted square-node syntax with escaped labels.

- [ ] **Step 1: Add failing tests** `mermaid_preserves_order_and_folded_hierarchy`,
  `mermaid_labels_cannot_escape_their_node`,
  `mermaid_output_capacity_is_preflighted`.
  Fixtures include quotes, brackets, parentheses, backticks, literal entities,
  `%%{init:...}%%`, `:::class`, `::icon(...)`, newlines, Unicode and emoji.
  Assert exact IDs/indentation and one node per source node; oversized output
  errors before allocation. Empty/control labels must either be represented
  without semantic substitution or produce a specific error before writing.
- [ ] **Step 2: Run** `cargo test --test mermaid`; expect absent serializer failure.
- [ ] **Step 3: Implement** document-order iteration and a checked byte-count
  pass accounting for indentation, quoting and entity expansion. Quote labels
  in the parser's NSTR mode; encode delimiter/directive characters safely
  without interpreting original entity-looking text. Use the same encoding
  logic for counting and writing, not parallel approximations.
- [ ] **Step 4: Verify** against local Mermaid 12.2.0 where the existing Node
  >=22.12.0 toolchain is present. Add dev-only `mermaid: "12.2.0"` and
  `jsdom: "26.1.0"` manifests, then `npm install --prefix tools/mermaid`;
  subsequent runs use `npm ci --prefix tools/mermaid`.
  `verify.mjs` sets up a local non-executing DOM and invokes Mermaid parsing
  and its parsed mindmap database on synthetic fixtures. The ignored Rust test
  writes actual `export_mermaid` output and expected source hierarchy to a
  temporary JSON file, then passes that path to `verify.mjs`; do not re-create
  the Rust encoder in JavaScript or verify hand-written substitutes. Assert
  the parsed parent/order/labels, not just parse success. Never replace sanitization with
  an identity stub or send maps to a remote service.
  If prerequisites are absent, record parser verification as unavailable,
  not passed. Resolve any syntax/label mismatch before claiming verified
  interoperability; fail explicitly for unrepresentable labels.
- [ ] **Step 5: Run** `cargo test --test mermaid` and, when available,
  `cargo test --test mermaid actual_mermaid_parser_roundtrip -- --ignored`;
  expect all fixtures/actual parsed structures pass. Report an unrun ignored
  check as unavailable, not passed.
- [ ] **Step 6: Commit** `feat: serialize bounded Mermaid mindmap exports`.

## Task 2: Safe export command, delivery and measurement

**Files:** Modify `src/commands.rs`, `src/ui.rs`, README and benchmark;
extend `tests/tabs_ui.rs`, `tests/mermaid.rs`.

**Consumes:** `export_mermaid`, `Editor::snapshot_with_drafts`,
`Workspace::destination_owner`, existing atomic writer/overwrite UI.

- [ ] **Step 1: Add failing tests** `mermaid_export_uses_uncommitted_session_snapshot`
  and `mermaid_export_does_not_replace_or_save_the_map`.
  Assert draft text is exported while document/history/path/dirty/draft state
  remain unchanged. Exercise cancel, rejected overwrite, write failure, tab
  switch after snapshot and an output path owned by another/open source tab.
- [ ] **Step 2: Run** `cargo test --test mermaid --test tabs_ui`; expect missing command/nonmutation behavior.
- [ ] **Step 3: Implement** catalog/menu/palette ExportMermaid with
  "Export Mermaid..." and native `.mmd` save filter. Capture stable session and
  validated snapshot before the dialog. Explain labels/hierarchy-only scope.
  Publish the resulting bytes atomically after overwrite and destination-owner
  checks. Do not reuse a helper that commits drafts just to export.
- [ ] **Step 4: Run** formatting, strict all-target Clippy, `cargo test --locked`,
  `sh scripts/bundle.sh`, `git diff --check`; all must pass.
- [ ] **Step 5: Measure** representative small/large completed exports with
  first/warm elapsed/CPU and peak process-family memory. Keep optional Node
  parser costs separate from production Rust export measurements.
- [ ] **Step 6: Update** README with actual parser/render/native verification,
  unsupported labels if any and metadata/layout omissions. Commit
  `feat: expose safe Mermaid export in BMM`.

All commits include `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
These steps do not authorize enabling native capture permissions or closing a
user's running BMM/Branchline application.
