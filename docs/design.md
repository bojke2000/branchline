# Branchline design

## Agreed outcome

Build an independent, local-first Rust desktop mind-mapping application for
personal brainstorming and fast keyboard-driven idea capture. Use egui/eframe,
target macOS first, and avoid architectural barriers to Windows/Linux.

Project destination: `/Users/macbook/examples/branchline`.
Do not modify the REA repository or `/Applications/Freeplane.app`.
The implementation and artwork are original; Freeplane is a behavioral and
interchange reference, not a source of copied implementation or visual assets.

## Investigation evidence and limits

REA opened `/Applications/Freeplane.app` and identified its ARM64 launcher:
SHA-256 `0f34c1b6aac1aaa863dfeadddb54dbbd6f1625993ff37b792ca1c061c83b343d`.
Plist Evidence
`ev_546997976fe7588d96eb5e25cee1b337ad85fe1d96dfaaecbf65f49c20a12764`
reports Freeplane 1.12.14, bundle identifier `org.freeplane.launcher`, .mm
document registration and the `freeplane` URL scheme.

REA artifact inspection parent
`ev_f49db45e0a4f704e4ab77858071864292c2db06b1b414841eeb3f846d997525a`
retains the package inventory: 869 file occurrences and 153 directories,
without integrity contradictions. This inventories package files, not Java
classes inside JARs. The directory inventory identity is not the launcher hash.

Direct read-only inspection of shipped resources established:

- `Contents/app/Freeplane.cfg` declares a Java launcher, framework JARs and
  Swing-related JVM exports.
- `Contents/app/resources/xml/mindmapmodemenu.xml` declares child/sibling
  creation, folding, search, undo/redo, and extensive styling commands.
- `Contents/app/resources/templates/standard-1.6.mm` demonstrates XML node
  IDs, fold state, nested styles, hooks and rich-content declarations.

These are static declarations, not observed UI behavior. No Freeplane runtime
scenario, Java decompilation, latency measurement or usability study was
performed. Do not claim a measured improvement over Freeplane.

## First-release scope

Include a two-sided mind map, automatic layout, node creation and inline
editing, deletion, folding, navigation, sibling reordering, reparenting,
undo/redo, search, a command palette, notes, links, themes, native file dialogs,
local persistence and recovery.

Exclude cloud accounts, collaboration, plugin execution, scripting, formulas,
full Freeplane compatibility, images, arbitrary cross-links, rich-text editing,
drag-and-drop reparenting and a separate outline editor.

## Structure and ownership

Use a single Cargo application with focused modules, not multiple crates:

- Document: stable opaque IDs, exactly one root, ordered children, plain text,
  optional plain notes/links, root-child side and fold state.
- Commands: validated semantic edits and history. A completed inline edit is
  one undo step. Failed/no-op commands do not create history entries.
- Layout: deterministic two-sided subtree placement with variable label sizes;
  descendants inherit their root branch side. Only visible nodes are drawn.
- Storage: versioned native documents, XML interchange, atomic writes and
  recovery. Parsed document semantics have one owner.
- Application: egui interaction, selection, viewport, command dispatch, dialogs
  and notifications.

Enforce unique IDs, a connected acyclic tree, valid child references, and root
invariants on input. Never delete or reparent the root. Reparenting must reject
cycles and preserve ordering and subtree identity. A new document starts with
one selected root and an explicit dirty state after changes.

## Interaction design

Use a spacious canvas, readable labels, restrained branch colors, curved
connectors and a clear focus/selection outline. Provide light and dark themes,
a compact toolbar and a toggleable notes panel. Show shortcuts in the command
palette and an accessible help panel; use labels as well as icons.

Keyboard navigation and text editing are distinct modes:

| Input | Navigation mode | Inline editing |
| --- | --- | --- |
| Tab | Create child and edit it | Commit, create child and edit it |
| Enter | Create sibling and edit it | Commit text |
| Escape | Close transient UI | Cancel the current edit |
| Arrow keys | Navigate tree/siblings | Normal text-cursor movement |
| Cmd+Z / Shift+Cmd+Z | Undo / redo | Text widget owns its local editing undo |
| Cmd+K | Open command palette | Commit before opening palette |

Double-click or F2 edits the selected node. Backspace/Delete requests deletion
outside text fields; nonempty subtrees require confirmation. Space toggles
folding outside text fields. Cmd+F opens search; search can reveal folded
ancestors and center a result without changing document text.

Provide explicit palette commands and shortcuts for sibling up/down and
indent/outdent. Indent reparents under the preceding sibling; outdent moves
after the parent, but cannot outdent a root child. Each is one undoable edit.

Pan via canvas drag, zoom around the pointer, fit map and focus selection.
Auto-center a newly created node if needed without resetting user zoom.
Do not let global shortcuts steal text input in notes, search or dialogs.
Cmd is the macOS modifier; use Ctrl on other platforms.

## File semantics and compatibility

The native `.branchline` format is versioned JSON. Reject unsupported versions,
malformed documents and invalid graph structure with actionable errors. Store
document content independently of transient egui widget state.

Freeplane `.mm` interchange initially supports ordered node trees, IDs, plain
text, folding, branch side, plain-text notes and links. Preserve supported
values, including Unicode and XML escaping. Missing IDs may receive generated
IDs; duplicate IDs are an error rather than ambiguous silent rewriting.

Parse XML with external entity/network resolution disabled. Bound input bytes,
node count, depth and text growth before unbounded allocation; surface precise
limit errors. Choose documented bounds during implementation according to the
parser and application resource model, not hidden truncation.

Report every unsupported attribute/element category with counts and useful node
locations. Unsupported rich content must not disappear without a warning.
Import stages a candidate document and a warning preview before replacing the
current document. The user must acknowledge loss; cancellation leaves the
current document and history unchanged.

Imported documents have no native save path. Export .mm through Save As, with a
preview of fields that cannot round-trip. Never advertise lossless Freeplane
round trips. Do not open links, execute scripts, evaluate formulas or load remote
assets on import. Opening a user-selected link requires an explicit action.

Save using a temporary sibling, flush and atomic replacement where supported.
An existing destination requires confirmation. Set the clean checkpoint only
after successful publication. Failure leaves edits dirty and reports the path
and cause; retain recoverable temporary data when appropriate.

Prompt Save / Discard / Cancel before New, Open or Quit when dirty. Failed or
cancelled saving cancels the pending destructive action. Track saved content
identity so undo back to the saved document clears dirty state correctly.

Use a separate versioned recovery snapshot in the platform application-data
directory, never the original file. Debounce recovery writes and surface errors.
On launch offer Restore / Discard without silently replacing another document.
Only remove a snapshot after explicit discard or verified successful save of
its matching document. Keep recovery provenance, original path and identity.
Support one running instance per recovery store or isolate per-instance stores
to prevent competing writers; document the implemented policy.

## Errors and safety

Represent expected failures with typed errors and show concise, actionable UI
messages. Do not report success after parser, persistence or recovery failure.
Keep filesystem/network side effects in adapters. No telemetry or cloud calls.
Dependency acquisition for development is separate from runtime behavior.

## Verification and acceptance

Automated checks cover semantic commands and undo/redo; root/cycle invariants;
Unicode edits; malformed native/XML inputs; import cancellation and unsupported
data warnings; supported-field XML round trips; atomic save failures; dirty
checkpoint transitions; recovery ownership and failed-write behavior.

Build and test with the installed Rust toolchain. Record exact commands and
actual outcomes in the eventual project README or handoff. Missing host tools
and unavailable runtime coverage remain explicit.

Exercise the real macOS application for keyboard-only creation, edit commit and
cancel, folding, search, reorder/reparent, undo/redo, file reopen, close prompts,
recovery, trackpad zoom, text focus, Unicode/IME and VoiceOver. Distinguish
manual observation from automated model tests. Do not declare accessibility
conformance based on widget-library support alone.

Benchmark deterministic generated maps of 100, 1,000 and 10,000 nodes.
Initial goal: edit plus layout under 100 ms at 1,000 nodes on the tested host.
Record cold/warm elapsed time, CPU time and peak process-family memory where
available; label unmeasured costs. Navigation/rendering needs a real runtime
check, not merely a layout benchmark. No relative performance or usability
claim about Freeplane without an equivalent measured workflow.

## Delivery

Deliver a runnable Cargo project with focused tests, README run/build commands,
documented keybindings, compatibility limits and verified/unverified platform
coverage. Produce a macOS app bundle if the installed toolchain permits it
without unrelated installations. Do not create a public repository, publish,
replace the installed Freeplane app or modify user maps.

This specification is staged in the session workspace to avoid adding an
unrelated app design to REA. The new external project is not scaffolded until
the written specification and implementation plan receive their required
reviews. No git commit in REA is appropriate for this artifact.
