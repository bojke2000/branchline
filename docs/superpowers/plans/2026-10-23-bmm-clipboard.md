# BMM Hierarchical Clipboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy map hierarchies as nested bullets and paste Word/OneNote/editor outlines into editable branches.

**Architecture:** A native adapter owns clipboard representations; a format boundary produces one validated semantic forest; Editor/Workspace publish one admitted transaction. Whole-map selection is explicit, distinct from ordinary visible multi-selection.

**Tech Stack:** Existing Rust stack; add `clipboard-rs = { version = "0.3.5", default-features = false }`, `html5ever` tokenizer and `cssparser`, with compatible parser versions resolved and locked in Task 2.

**Spec:** `docs/superpowers/specs/2026-10-23-bmm-clipboard-export-design.md`

## Global Constraints

- Requires the completed tabs plan and its `Workspace`, stable `SessionId`, `EditorEdit`, admission and draft-snapshot interfaces.
- Ctrl and Cmd are both supported on canvas; fields retain native/local editing.
- Copy labels/order/nesting, including folded descendants; no metadata or old IDs.
- One top-level pasted item becomes a blank map's root; forests get "Pasted ideas".
- Nonblank maps append under the active node. Paste as new map creates a dirty tab only after validation.
- 32 MiB combined clipboard payload; 100,000 nodes, depth 256 with root depth 1, 1 MiB per field, 64 MiB decoded text.
- Account for representation expansion before output allocation; no silent truncation or hierarchy guesses.
- Explicit user paste only; no clipboard polling, persistence or remote resource loading.
- Clipboard/Office simulation is not actual Word/OneNote verification.
- No implementation until this plan and its execution method are approved.

## Review Focus

- A flat fallback must not silently override richer nesting facts: Task 2.
- Copied multiline labels must not turn into extra topics without warning: Task 2.
- A clipboard change during multi-format read must not combine unrelated items: Task 4.
- A paste preview accepted after a tab closes must not edit its neighbor: Task 4.
- Whole-map Delete must include hidden descendants but retain the root: Task 3.

---

## File map and shared interfaces

- Create `src/outline.rs`: semantic `OutlineForest`, `OutlineNode` and validation, independent of clipboard/HTML.
- Create `src/outline_format.rs`: plain/HTML/Office parsing and copy serialization.
- Create `src/clipboard.rs`: native transport and the narrow injected test interface.
- Create `src/ui/paste.rs`: stable-target preview and acceptance.
- Modify document/editor/session/workspace for selection and transactions; UI/commands for dispatch.
- Create `tests/outline.rs`, `tests/clipboard_ui.rs`, `tests/fixtures/clipboard/`;
  extend existing document/editor/UI tests and benchmark.

Define `OutlineNode { label: String, children: Vec<OutlineNode> }` and
`OutlineForest { roots: Vec<OutlineNode> }`. `validate() ->
Result<(), DocumentError>` counts nodes, decoded bytes and depth iteratively.
No parser provenance enters document semantics.

`ClipboardPayload { plain: Option<String>, html: Option<String> }` is a
transport-neutral DTO in `src/outline_format.rs`.
`OutlineFormat::{Plain, Html, OfficeHtml}`,
`OutlineWarning { category: OutlineWarningKind, count: usize, locations: Vec<SourceLocation> }` and
`ParsedOutline { forest, source, warnings }` live in `src/outline_format.rs`.
`SourceLocation` contains line/column offsets, not copied source paragraphs.
Use fixed warning kinds AmbiguousIndentation, UnsupportedOfficeMetadata,
OutsideListText, RepresentationLoss, IgnoredContent and ConflictingFormats,
with at most eight example locations, matching existing import reports.

`parse_clipboard(payload: &ClipboardPayload) -> Result<PasteInterpretation, FormatError>`
returns `PasteInterpretation::{Ready(ParsedOutline), Preview(Vec<ParsedOutline>)}`.
At most two candidates: supported rich facts and an explicit plain alternative.
No supported interpretation is an error, not an empty success.

`serialize_outline(forest: &OutlineForest) -> Result<OutlineCopy, FormatError>`
returns `OutlineCopy { payload: ClipboardPayload, warnings: Vec<OutlineWarning> }`.
Count combined payload bytes before constructing both strings.

## Task 1: Semantic forest and atomic insertion

**Files:** Create `src/outline.rs`, `tests/outline.rs`; modify
`src/document.rs`, `src/app.rs`, `src/session.rs`, `src/workspace.rs`, `src/lib.rs`.

**Produces:** Semantic types; `Command::PasteOutline { parent: NodeId, forest: OutlineForest }`,
`Command::ReplaceWithOutline { forest: OutlineForest }`;
`Workspace::paste_outline(target: PasteTarget, forest: OutlineForest) ->
Result<SessionId, WorkspaceError>`.
Define `PasteTarget::{Append { session: SessionId, node: NodeId, revision: u64 },
NewMap}` in `src/workspace.rs`. Pristine-blank replacement is decided by the target session owner.

- [ ] **Step 1: Add failing tests** `outline_append_is_one_undo_step`,
  `blank_outline_roots_are_built_without_placeholder_guessing`,
  `paste_depth_and_capacity_failure_is_atomic`. Use A with children B/C:
  `assert_eq!(child_labels, ["B", "C"]);`
  `assert!(new_ids.is_disjoint(&old_ids));`
  `assert_eq!(history_generation, previous_generation + 1);`
  undo restores the exact original Document. Multiple roots create
  `assert_eq!(root.text, "Pasted ideas")`; a loaded one-node map keeps its root.
- [ ] **Step 2: Run** `cargo test --test outline --test document`; expect missing outline interfaces.
- [ ] **Step 3: Implement** validated candidate construction with fresh UUIDs,
  ordered insertion, inherited sides and unfolded destination. Blank-map
  replacement and append use existing admitted transactions. New-map paste
  admits before creating the tab. Reject stale revision/owner/node identities
  instead of redirecting to the active session.
- [ ] **Step 4: Run** `cargo test --test outline --test document --test workspace`; expect pass at exact count/depth/text limits and no partial history on rejection.
- [ ] **Step 5: Commit** `feat: insert normalized outline forests atomically`.

## Task 2: Bounded plain and Office HTML interchange

**Files:** Create `src/outline_format.rs`, `tests/fixtures/clipboard/`;
modify `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `tests/outline.rs`.

**Produces:** `parse_clipboard`, `serialize_outline` and the named report types above.

- [ ] **Step 1: Add failing tests** `html_and_plain_copy_have_equivalent_hierarchy`,
  `office_list_levels_are_source_facts`, `ambiguous_outline_requires_preview`,
  `copy_expansion_is_checked_before_allocation`.
  Fixtures cover nested ul/ol, inline `mso-list:l0 level2 lfo1`,
  `mso-list:Ignore` marker spans, tabs, consistent 2/4-space indentation,
  mixed/skipped indentation, Unicode/emoji, empty items, HTML entities,
  multiline labels and plain/rich disagreement. Preserve visible label text;
  unsupported class-only Office metadata requires preview.
- [ ] **Step 2: Run** `cargo test --test outline`; expect absent format functions or semantic failures.
- [ ] **Step 3: Implement** HTML tokenization through `html5ever`, using a
  streaming semantic list/paragraph builder rather than a full unbounded DOM.
  Feed bounded chunks and stop with a recorded error when budgets fail;
  handle valid omitted li end-tags, StartFragment/EndFragment comment boundaries
  and nested list transitions explicitly.
  Parse Office style declarations with `cssparser`, not regex. Suppress
  script/style and ignored marker contents; never follow resource URLs.
- [ ] **Step 4: Implement** plain indentation-stack parsing and escaped nested
  HTML/tab-bullet output. Indentation jumps or mixed interpretations require
  preview. Empty separator lines are distinct from explicit empty list items.
  Warn about plain-only multiline/leading-whitespace normalization.
  Every retained label/warning and parser stack is bounded by transfer bytes,
  document counts/depth and the eight-location report cap; do not retain an
  unbounded token/event list. Check exact 32 MiB output and one byte above.
- [ ] **Step 5: Run** `cargo test --test outline --test interchange`; expect real fixtures and existing interchange pass.
- [ ] **Step 6: Commit** `feat: parse and copy bounded rich bullet outlines`.

## Task 3: Explicit whole-map selection

**Files:** Modify `src/app.rs`, `src/session.rs`, `src/document.rs`,
`src/commands.rs`, `src/ui.rs`; extend `tests/editor.rs`, `tests/bmm_ui.rs`,
and create `tests/clipboard_ui.rs`.

**Produces:** `SelectionCoverage::{VisibleNodes, WholeMap}`,
`Editor::select_all()`, `selection_coverage() -> SelectionCoverage`,
`Editor::copy_forest() -> Result<OutlineForest, EditorError>`.
Add `EditorEdit::SelectAll` and shared SelectAll command.

- [ ] **Step 1: Add failing tests** `whole_map_selection_includes_folded_root_tree`,
  `copy_selection_deduplicates_in_document_order`,
  `whole_map_delete_retains_root_and_one_undo_restores_hidden_nodes`.
  Assert selection count includes hidden nodes, root is active, ordinary
  multi-selection still drops hidden secondary IDs, and folding preserves
  whole-map coverage. Click/navigation/edit/undo leave whole-map mode.
- [ ] **Step 2: Run** `cargo test --test editor --test bmm_ui --test clipboard_ui`; expect missing explicit coverage behavior.
- [ ] **Step 3: Implement** explicit coverage owned by Editor; reuse deletion
  planning and tree traversal. Do not allocate invisible IDs into the old
  visible selection set. UI highlights visible coverage and reports hidden
  count; copy deduplicates covered subtrees in document order.
- [ ] **Step 4: Run** the same command plus `--test review_regressions`; expect old invisible-selection safety and new whole-map semantics pass.
- [ ] **Step 5: Commit** `feat: select and copy complete maps including folded nodes`.

## Task 4: Native clipboard, commands and paste preview

**Files:** Create `src/clipboard.rs`, `src/ui/paste.rs`;
modify `tests/clipboard_ui.rs`, `Cargo.toml`, `Cargo.lock`, `src/ui.rs`,
`src/commands.rs`, `src/main.rs`, `src/lib.rs`.

**Consumes:** Format reports, `PasteTarget`, whole-map coverage and workspace admission.
**Produces:** `trait OutlineClipboard { fn read(&mut self) -> Result<ClipboardPayload, ClipboardError>;
fn write(&mut self, payload: ClipboardPayload) -> Result<(), ClipboardError>; }`,
`NativeClipboard`, typed `ClipboardError`, and `PendingPaste { target, interpretation }`.
Inject the transport into NativeApp while keeping production native and tests in-memory.

- [ ] **Step 1: Add failing frame tests** `physical_ctrl_and_cmd_copy_the_map`,
  `paste_builds_blank_or_appends_under_active_node`,
  `paste_as_new_map_and_cancel_are_non_destructive`,
  `preview_keeps_its_original_session_target`,
  `changed_clipboard_does_not_mix_with_backend_paste`,
  `text_fields_keep_native_clipboard_and_type_replace_ignores_paste`.
  Fake transport records calls/payloads and can fail acquisition/publication.
  Assert no clipboard reads on idle frames, no map copy in fields, one payload
  read per paste, and no writes/history on cancellation or stale destination.
- [ ] **Step 2: Run** `cargo test --test clipboard_ui`; expect missing command/adapter behavior.
- [ ] **Step 3: Implement** `clipboard-rs` with image support disabled. Read
  available formats explicitly and distinguish absence from acquisition
  failures; never use a failed query as proof a format is absent. Capture a
  coherent payload using native change-count checks on macOS; a changed
  clipboard is an explicit retry error, not mixed content. Use one
  `Clipboard::set` with Html/Text contents after all payload validation.
  Add macOS-only direct `objc2 = "0.6.3"` and
  `objc2-app-kit = { version = "0.3.2", default-features = false, features = ["NSPasteboard"] }`
  dependencies for the NSPasteboard change-count query. Other platforms'
  coherence/native behavior remains explicitly unverified until exercised;
  without coherent rich reads, offer an explicit plain-text preview with its
  limitation, never a mixed rich/plain payload or an asserted absent hierarchy.
  Document any native clear-then-write or initial-allocation limitations;
  success requires both intended representations. Do not read unrelated old
  clipboard formats merely to simulate rollback.
- [ ] **Step 4: Implement** canvas-only Ctrl/Cmd A/C/V and PasteAsNewMap catalog
  commands, including context menu/palette. Route backend Copy/Paste events
  and key events as one explicit gesture to prevent duplicate paste. Check
  backend Paste sizes before cloning event strings; require its plain payload
  to agree with the native snapshot instead of substituting newer clipboard
  text. Show
  proposed trees/warnings for ambiguous input, capture payload and target once,
  and publish only after acceptance and fresh admission.
- [ ] **Step 5: Run** `cargo test --test clipboard_ui --test bmm_ui --test tabs_ui --test review_regressions`; expect no text-field, panning, selection or ownership regression.
- [ ] **Step 6: Commit** `feat: integrate hierarchical clipboard commands and previews`.

## Task 5: Verify real boundaries and deliver

**Files:** Extend benchmark and README; add opt-in native clipboard tests to
`tests/clipboard_ui.rs` only if they can preserve unrelated clipboard state.

- [ ] **Step 1: Run** synthetic production-boundary small/large copy/paste
  workflows, asserting final labels/order/depth before recording first/warm
  elapsed/CPU and peak process-family memory with `/usr/bin/time -l`.
- [ ] **Step 2: Exercise** actual Word/OneNote/editor round trips only where
  available and authorized. Record application versions, representations and
  observed hierarchy. If blocked, explicitly mark this coverage unavailable;
  fixture success is not a replacement and "perfect" remains unsupported.
- [ ] **Step 3: Run** formatting, strict all-target Clippy, locked full tests,
  bundle build and `git diff --check`; require passing results.
- [ ] **Step 4: Update** README with verified formats, resource/representation
  limitations, native coverage and how to save/reopen the rebuilt BMM bundle.
- [ ] **Step 5: Commit** `docs: document verified hierarchical clipboard workflows`.

All commits include `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
No native clipboard monitoring or data collection is part of execution.
