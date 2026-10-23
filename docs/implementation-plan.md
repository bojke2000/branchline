# Branchline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a runnable keyboard-first Rust mind mapper with explicit .mm
compatibility, safe local persistence and a calm macOS-first interface.

**Architecture:** One Cargo application with document/command, interchange,
storage, layout and egui modules. Document semantics are independent of widgets;
commands own edits and history, while adapters own side effects.

**Tech Stack:** Rust, eframe/egui, serde/serde_json, quick-xml, thiserror, uuid,
rfd, directories, tempfile and fs2. Resolve compatible published versions during
bootstrap and commit Cargo.lock; do not guess SDK APIs from older versions.

**Spec:** [design.md](design.md)

## Global Constraints

- Project destination: `/Users/macbook/examples/branchline`.
- Do not modify the REA repository or `/Applications/Freeplane.app`.
- Use egui/eframe, target macOS first, and avoid architectural barriers to Windows/Linux.
- No telemetry or cloud calls.
- Never advertise lossless Freeplane round trips.
- Imported documents have no native save path.
- No public repository, publication, installation over Freeplane or user-map mutation.
- Use original code/assets; do not copy Freeplane implementation or artwork.
- Record unavailable runtime/platform/accessibility coverage explicitly.
- Rust is absent from PATH and checked standard locations. Obtain installation
  approval before installing a user-local Rust toolchain; do not install Homebrew
  or unrelated software. If declined, execution cannot meet build acceptance.
- Stage plans in the session workspace until approved; then copy spec/plan into
  the new project's `docs/` using editing tools.
- Use Conventional Commits with the required Copilot co-author trailer.

## Review Focus

- Editing an IME composition or notes field must not trigger destructive canvas shortcuts.
- Cancelling a lossy import must leave content, history, selection and path unchanged.
- A destination becoming unwritable must keep the document dirty and abort pending close.
- Two running instances must not race over or discard each other's recovery data.
- Repeated deep/wide map edits must neither create cycles nor panic from recursion.

These are acceptance risks, not findings; owning tasks below contain their checks.

## File Map

`Cargo.toml`, `Cargo.lock`, `.gitignore`: application/dependencies and build exclusions.
`src/lib.rs`: exports core modules for tests/benchmarks.
`src/main.rs`: native window entry point.
`src/document.rs`: types, validation, command application and history.
`src/interchange.rs`: versioned JSON and Freeplane XML parsing/export/warnings.
`src/storage.rs`: atomic publication, recovery and instance lock.
`src/layout.rs`: deterministic visible geometry.
`src/app.rs`: editor controller, dialogs and file lifecycle.
`src/ui.rs`: canvas, input routing, command palette and notes.
`tests/document.rs`, `tests/interchange.rs`, `tests/storage.rs`,
`tests/layout.rs`, `tests/editor.rs`: behavioral checks.
`examples/benchmark.rs`: generated-map benchmark.
`packaging/Info.plist`, `scripts/bundle.sh`: local macOS bundle creation.
`README.md`: run/build, shortcuts, bounds, compatibility and verification scope.

## Task 1: Validated documents and undoable editing

**Files:** manifest, ignore file, library entry, document module, document tests.

**Produces:** `NodeId = String`; `Side::{Left, Right}`;
`Node { id, text, notes, link, children: Vec<NodeId>, side, folded }`;
`Document { version: u32, root: NodeId, nodes: BTreeMap<NodeId, Node> }`.
`Document::new(text: &str) -> Self`,
`Document::validate(&self) -> Result<(), DocumentError>`,
`Document::parent(&self, id: &str) -> Option<&str>`.
`Command::{AddChild, AddSibling, SetText, SetNotes, SetLink, Delete,
ToggleFold, MoveSibling, Indent, Outdent}` with explicit target IDs/values.
`History::execute(&mut self, doc: &mut Document, command: Command)
-> Result<Option<NodeId>, DocumentError>`, `undo`, `redo`.
History mutation is transactional. Start with bounded document snapshots for
history, cap retained history bytes and disclose eviction; optimize only if
benchmarks show a problem. Do not clone documents on every render frame.

- [ ] Bootstrap with Cargo only after approval/toolchain readiness, initialize
  git in the new empty directory, add runtime and test dependencies, create
  failing integration tests before semantic implementation.
- [ ] Add `editing_round_trip`: execute add, Unicode text edit, indent/outdent,
  reorder and delete; undo all must equal the initial document; redo all must
  equal the final document.
- [ ] Add `invalid_edits_are_transactional`: deleting/reparenting root and a
  cycle-producing move must return errors and leave document/history unchanged.
  Add `limits_and_deep_tree`: validate near-limit deep and wide maps iteratively.
- [ ] Run `cargo test --test document`; confirm failure from absent functionality.
- [ ] Implement the interfaces and iterative validation. Limits shared by all
  producers: 32 MiB document bytes, 100,000 nodes, depth 256, 1 MiB per text field,
  64 MiB total decoded text, 32 MiB history snapshots; reject before expansion.
- [ ] Run `cargo test --test document`; all cases pass. Commit the focused change.

## Task 2: Explicit, bounded document interchange

**Files:** interchange module and tests.
**Consumes:** Task 1 document types and validation.
**Produces:** `ImportReport { document: Document, warnings: Vec<ImportWarning> }`;
`ImportWarning { category: String, count: usize, locations: Vec<String> }`.
`read_native(bytes: &[u8]) -> Result<Document, FormatError>`,
`write_native(doc: &Document) -> Result<Vec<u8>, FormatError>`,
`import_mm(bytes: &[u8]) -> Result<ImportReport, FormatError>`,
`export_mm(doc: &Document) -> Result<ExportReport, FormatError>`,
`ExportReport { bytes: Vec<u8>, warnings: Vec<ImportWarning> }`.

- [ ] Add `supported_fields_round_trip`: use two sides, ordered Unicode labels,
  quotes/ampersands, notes, link, IDs and folding; native and XML round trips
  preserve supported values. Use original synthetic fixtures.
- [ ] Add `unsupported_content_is_reported`: rich HTML, styles, hooks, icons,
  timestamps, arrow links and unknown fields each produce category/count/location;
  do not misclassify text nested in an unsupported element as a supported node.
- [ ] Add `hostile_xml_is_rejected`: DTD/entity declaration, duplicate ID,
  multiple roots, malformed UTF-8, excessive depth, decoded text/size expansion
  and invalid booleans fail explicitly. Unknown native version/fields fail
  rather than silently discard data.
- [ ] Run `cargo test --test interchange` and confirm red.
- [ ] Implement event-driven quick-xml parsing; disable/reject DTD and external
  entities. Bound file reads before parsing; validate every produced document.
  Support plain note content explicitly and warn for formatted HTML notes.
  Aggregate warning counts with bounded location samples, reporting omissions.
- [ ] Run `cargo test --test interchange`; confirm green and commit.

## Task 3: Atomic saves, dirty checkpoints and owned recovery

**Files:** storage module and tests.
**Consumes:** validated native serializer.
**Produces:** `atomic_write(path: &Path, bytes: &[u8])
-> Result<(), StorageError>`, `RecoveryStore::open(directory: PathBuf)
-> Result<RecoveryStore, StorageError>`,
`RecoveryStore::{load, write, discard}` with document UUID and expected identity.
`RecoveryRecord { version, document_id, original_path, document }`.
`SaveCheckpoint` compares validated content, not history index or node counts.

Choose one exclusive advisory lock per application-data recovery store; hold
the handle for the full application lifetime. A second instance returns an
actionable error and does not read/write recovery. Use filesystem lock ownership,
not stale PID-file heuristics.

- [ ] Add `publication_failure_preserves_original`: successful overwrite is
  complete; failed staging/publication keeps old bytes and returns an error.
  Use deterministic filesystem obstacles, not permission bits under root.
- [ ] Add `recovery_lock_and_identity`: a second opener is rejected; wrong
  document identity cannot discard a snapshot; malformed snapshots stay on disk.
- [ ] Add `checkpoint_tracks_undo`: edit marks dirty, undo to saved content
  clears dirty, failed save does not advance checkpoint.
- [ ] Run `cargo test --test storage`; confirm red.
- [ ] Implement sibling-temp staging, flush/sync and publication using tempfile;
  preserve actionable error context and clean up only owned temporary resources.
  Application-data setup failures are visible, never converted to success.
- [ ] Run `cargo test --test storage`; confirm green and commit.

## Task 4: Deterministic two-sided layout

**Files:** layout module, layout tests, benchmark example.
**Consumes:** document tree.
**Produces:** `layout(doc: &Document, sizes: &BTreeMap<NodeId, Size>)
-> Result<Layout, LayoutError>`; independent `Point`, `Size`, `Rect`,
`Layout { nodes: BTreeMap<NodeId, Rect>, edges, bounds }`.
UI measures labels; layout owns geometry, not text metrics.

- [ ] Add `branches_do_not_overlap`: asymmetric left/right and multiline labels
  have non-overlapping sibling subtree bounds and deterministic output.
- [ ] Add `folded_descendants_are_absent`: hidden descendants are not in layout;
  root remains visible; zero-branch and deep maps succeed without recursion panic.
- [ ] Run `cargo test --test layout`; confirm red.
- [ ] Implement bottom-up subtree heights and top-down placement; inherit branch
  side, preserve order and use fixed inter-node gaps. No per-frame full relayout
  unless document visibility/content or measured sizes change.
- [ ] Run tests; add generated maps at 100/1,000/10,000 nodes to
  `cargo run --release --example benchmark`. Report edit-plus-layout elapsed
  distributions and sample count; measure whole process separately with
  `/usr/bin/time -l`. Goal under 100 ms at 1,000 nodes. Commit.

## Task 5: Editor lifecycle and keyboard state machine

**Files:** app module, editor tests, initial main/UI wiring.
**Consumes:** Tasks 1-4.
**Produces:** `Editor` containing document/history, selected ID, editing buffer,
saved path/checkpoint and pending action. `EditorAction` is the named dispatch
contract shared by shortcuts/palette/toolbar.
`Editor::dispatch(&mut self, action: EditorAction) -> Result<(), EditorError>`.
Separate navigation, inline editing and auxiliary text-input focus.

- [ ] Add `edit_modes_route_keys`: Enter/Tab/Escape follow the spec; notes,
  search and active IME/text input consume their keys without canvas mutation.
- [ ] Add `cancelled_import_is_transactional`: stage warnings, cancel, and
  assert document/history/path/selection/checkpoint are unchanged.
- [ ] Add `failed_save_blocks_close`: Save / Discard / Cancel prompts own New,
  Open and Quit; failed/cancelled save cannot execute the pending action.
  Explicit discard transitions to a fresh matching recovery record or removes
  only its owned record, never leaves stale discarded content restorable.
- [ ] Run `cargo test --test editor`; confirm red.
- [ ] Implement one action dispatcher and explicit staged import/close/save
  states. Clear or reconcile stale selection after delete/undo. Confirm existing
  destination replacement, including export, immediately before publication.
  Show recovery restore/discard before a new document can overwrite the snapshot.
- [ ] Run `cargo test --test editor`; confirm green and commit.

## Task 6: Usable native desktop interface

**Files:** main/UI/app modules and README keyboard section.
**Consumes:** editor dispatcher and layout geometry.
**Produces:** runnable eframe application and real command palette/canvas.

- [ ] Add controller tests for palette filtering/action dispatch, search revealing
  ancestors, zoom anchor preservation, fit-to-bounds and selected-node focus.
  Run corresponding integration tests and confirm red before implementation.
- [ ] Implement a calm light/dark canvas, compact labeled toolbar, help, palette,
  search, notes/link panel, inline node editor, fold affordance and selection.
  Use egui shapes for curved edges, viewport culling, cached text/layout.
  Route keyboard input only after widget focus/consumption decisions.
- [ ] Wire native dialogs, errors, loss previews, subtree deletion confirmation,
  recovery debounce and close interception to the tested editor state machine.
  External links require a user action; never auto-load imported resources.
- [ ] Run `cargo test` and `cargo build`; then launch the app and observe window
  readiness. Perform real keyboard/mouse actions when host tooling permits;
  do not infer interactive success from a process remaining alive.
- [ ] Check IME/VoiceOver/trackpad explicitly or label them unverified. Resolve
  any directly caused failures, record limitations, and commit.

## Task 7: Bundle, documentation and final verification

**Files:** README, packaging plist/script, approved spec/plan copies in docs.
**Consumes:** working app.
**Produces:** reproducible local macOS app bundle without installing packagers.

- [ ] Write README with `cargo run`, test/build/benchmark commands, every actual
  shortcut, .mm supported fields/warnings, resource limits, recovery location and
  lock policy. Document known UI/accessibility/platform limitations.
- [ ] Add a bundle script that builds release output and creates
  `target/bundle/Branchline.app` with a correctly named executable and plist.
  Do not remove a pre-existing directory recursively or replace installed apps.
- [ ] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, `cargo build --release`, benchmark and bundle script; record actual
  results. Inspect the produced bundle and launch readiness when possible.
- [ ] Review the complete diff against the spec with one independent reviewer,
  resolve meaningful defects and rerun affected checks. Do not request a specific
  reviewer model absent explicit user permission.
- [ ] Commit final deliverables; keep generated binaries under ignored target/.
  Handoff the runnable path, concrete behavior, limitations and unmeasured claims.

## Self-review

Tasks cover all spec sections: 1 document invariants/history, 2 interchange,
3 save/recovery, 4 layout/performance, 5 interaction/lifecycle, 6 interface and
runtime checks, 7 packaging/docs. Review Focus conditions each have an owning
test task. Per-instance recovery ambiguity is resolved by an exclusive OS lock.
GUI/IME/accessibility observations remain host-dependent, not automated claims.
No product implementation begins before this plan is reviewed and an execution
method is selected.
