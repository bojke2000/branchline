# BMM

A local-first, keyboard-driven mind mapper written in Rust with egui/eframe.
Made for quick personal brainstorming: a quiet two-sided canvas, automatic
layout, inline editing, folding, searchable commands and a plain-text notes panel.
Dark mode is the default. The white reference-style editor remains available
under **More > Switch light / dark theme**. Both use an outlined oval root,
vivid curved branches, underlined topics and blue selection. An original
blue/teal branching logo anchors the grouped icon toolbar.

This is an original early implementation, not Freeplane feature parity or a
measured usability improvement. No Freeplane code or artwork is included.

## Run

Install Rust with Cargo, then:

```sh
cargo run --locked
```

On macOS, build a local app bundle:

```sh
sh scripts/bundle.sh
open target/bundle/BMM.app
```

If Cargo is not on PATH after a user-local installation, first run
`. "$HOME/.cargo/env"`. Building fetches the locked development dependencies;
the application has no telemetry, cloud service or automatic remote-content
loading. A user-selected HTTP/HTTPS/mailto link opens in the system handler.

Use **Open** to import `examples/brainstorm.mm` for an original sample map.
Import always previews compatibility and never edits the source file.

## Keyboard and canvas

| Input | Action |
| --- | --- |
| Tab | Add child; while editing, commit and continue with a child |
| Enter | Add sibling; while editing, commit without creating a node |
| F2 / double-click | Edit the selected idea |
| Start typing on a selected idea | Replace its label as you type; Enter commits, Escape restores |
| Escape | Cancel inline editing or close search/commands |
| Arrows | Navigate branches and siblings |
| Alt+Shift+arrows | Pan the entire map by 40 screen pixels without editing it |
| Space | Fold/unfold the selected branch |
| Shift+Up / Shift+Down (or Alt+Up / Alt+Down) | Reorder among visible siblings on the same side |
| Alt+Right / Alt+Left | Indent under preceding sibling / outdent |
| Shift+click | Add/remove a node from the selection |
| Delete / Backspace | Delete selected branches; multi-selection and nonempty subtrees require one confirmation |
| Cmd+Z / Cmd+Shift+Z | Undo / redo |
| Cmd+K / Cmd+F | Command palette / search labels and notes |
| Cmd+N / Cmd+O / Cmd+S | New / open / save |
| Cmd+Shift+S | Save as |
| Cmd+Q | Quit with unsaved-change protection |
| Drag blank canvas | Pan |
| Drag the central root | Pan the entire map without changing its structure or undo history |
| Drag a topic | Move its whole branch; preview before release |
| Right-click a topic / blank canvas | Node commands / commands for the current selection, with shortcut hints |
| Scroll / pinch | Zoom around pointer |
| Fit map / Focus | Fit all visible nodes / center selection |

Use Ctrl instead of Cmd on other platforms. Text fields retain cursor keys,
Shift+arrow text selection and local text undo. Click blank canvas after using another control to return
keyboard focus to the map. The root cannot be deleted or outdented; Enter on
the root creates a child. New branches alternate sides; descendants inherit
their root branch's layout side and color. Reordering keeps branch colors stable.
Folding, notes and metadata are undoable. The command palette, context menus and
toolbar use the same command definitions and dispatch as keyboard shortcuts.
Typing replaces only the active node, even when several nodes are selected.
The original label remains unchanged until the edit is committed; Undo restores
a committed replacement. Search, notes and dialogs retain their own text input.

### Drag-and-drop and selection

Drop in the middle of a topic to append a child, or near its top/bottom to insert
before/after it. A highlighted destination, insertion marker and pointer label
show the planned placement. Dropping into a folded topic unfolds that topic;
the moved subtree's own folding, text, notes and links are retained. Top-level
branches can be dragged across the root onto blank canvas to switch sides.
Layout remains automatic, not free positioning.

Branch reorganization commits only on release. Escape, loss of window focus or
release outside the canvas cancels it. Circular moves, reparenting the central
root and excessive tree depth are rejected. Each completed branch move is a
single undo step. Dragging the root instead pans every branch together, even
when the root passes outside the visible canvas; Escape restores the starting
view. Root panning does not edit the document or add undo steps.

Shift+click toggles individual nodes; an ordinary click starts a single selection.
Right-clicking a selected node preserves the group. Delete removes the union of
the selected subtrees, without counting overlapping selections twice, and one
undo restores the deletion. If the central root is included, the confirmation
explicitly says it will be kept. Other commands operate on the active node;
dragging selects and moves only the grabbed branch, not the whole group.

## Documents and safety

Native `.branchline` files are versioned JSON. The existing extension, internal
application identifier and recovery location are retained for compatibility
with earlier Branchline builds; the visible product and app bundle are BMM.
The source directory and Cargo executable remain named `branchline`.
Saves use a flushed temporary
sibling and atomic replacement. Save As and export explicitly confirm existing
destinations. New/Open/Quit ask Save, Discard or Cancel when there are changes;
failed/cancelled saves do not continue the pending action.

Recovery is debounced by 700 ms after changes, including unfinished inline
text and notes. It is separate from the original document. On macOS it lives in
`~/Library/Application Support/org.Branchline.Branchline/recovery.json`
(the exact platform path is selected by the `directories` crate). A filesystem
lock allows one instance per recovery store. A second instance reports an error
rather than competing over recovery. On startup, choose Restore or Discard.
Malformed snapshots are retained and reported, not silently overwritten; move
the reported file aside manually if you want to preserve it and start afresh.

Errors remain visible in the status area. Recovery errors do not imply a
successful snapshot. Undo history uses bounded snapshot storage and reports
when older steps have been evicted. There is no crash-proof guarantee before
the debounce interval, during OS/storage failure, or before the last successful
snapshot.

## Freeplane `.mm` compatibility

Import/export supports ordered node trees, stable IDs, plain Unicode node text,
folding, branch side, plain-text notes and links. Missing IDs are generated;
duplicates, malformed XML, ambiguous trees and invalid values are rejected.
XML DTD/entity declarations are rejected; imported hooks, scripts and formulas
never run.

Unsupported attributes/elements, styles, icons, rich content and cross-links
are reported by category/count with up to eight location examples. Imported
documents have no save path and must be saved separately. Export previews the
supported subset; discarded import data cannot be reconstructed. HTML-rich
labels/notes are not supported as rich text and may be omitted with warnings.
**This is not a lossless `.mm` round trip.**

Bounds: 32 MiB input/output files, 100,000 nodes, tree depth 256, 1 MiB per
decoded text field and 64 MiB total decoded text. XML nesting has a small
additional envelope for containers. The UI limits typed field length to
262,144 characters; imported supported text may be larger within byte bounds.
Oversized input produces an error, never silent truncation.

## Development and verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
cargo run --release --example benchmark
/usr/bin/time -l target/release/examples/benchmark
```

The tests cover semantic edits/history, invalid graphs, XML loss warnings,
interchange round trips, atomic-save failures, recovery locks/identity,
dirty-state checkpoints, staged import cancellation, keyboard edit modes,
deterministic layout, zoom/focus and actual headless egui input routing.
Review regressions also cover offscreen inline creation, folded/undo selection
visibility, abandoned IME ownership and cancellation of recovered drafts.
The BMM interaction tests also cover the default canvas, shared context commands,
stable branch colors, dark startup with optional light mode, whole-map root
panning, keyboard panning without stealing text-field shortcuts, type-to-replace
editing with cancellation and undo, headless Unicode/IME input routing,
Shift+arrow reordering, Shift-click selection, atomic
batch deletion, drop previews, cycle rejection, side switching and cancellation
on Escape, outside release or an intervening unsaved-changes dialog.

Executed on this Mac: `cargo fmt --check` passed;
`cargo clippy --all-targets -- -D warnings` passed;
`cargo test --locked` passed (54 integration tests);
`cargo build --release --locked` and `sh scripts/bundle.sh` passed.
These results do not establish the unverified native scenarios listed below.

Measured on this development Mac, optimized generated-map operations plus layout:

| Operation | Nodes | First sample | Warm median (20) | Warm p95 |
| --- | --- | --- | --- | --- |
| Text edit + layout | 100 | 0.309 ms | 0.192 ms | 0.236 ms |
| Text edit + layout | 1,000 | 2.797 ms | 1.545 ms | 2.288 ms |
| Text edit + layout | 10,000 | 11.500 ms | 11.175 ms | 11.446 ms |
| Subtree move + layout | 100 | 0.244 ms | 0.190 ms | 0.300 ms |
| Subtree move + layout | 1,000 | 1.372 ms | 1.162 ms | 1.283 ms |
| Subtree move + layout | 10,000 | 13.022 ms | 12.173 ms | 12.530 ms |

The sampled process used 0.80 s elapsed, 0.57 s user CPU and 0.01 s system
CPU, with maximum resident size 62,717,952 bytes. Edit and move phases use
separate histories; the old history is released before starting the next phase.
The benchmark process had no children; this is not UI renderer memory.
First samples are not guaranteed cold OS-cache measurements. Rendering, text
measurement, drag hit-testing, selection reconciliation, dialogs and real
interaction are excluded. The additional move phase means whole-process totals
are not equivalent to the earlier edit-only benchmark.
There is no equivalent Freeplane benchmark or
agent-token measurement, so no relative speed/context claim is made.

The macOS ARM64 BMM bundle was launched; its exact executable, launch time and
1280x872 native window were observed, then the owned verification instance was
closed. This establishes window readiness, not interactive usability.
REA runtime screenshot
and accessibility capture were blocked by missing macOS Screen Recording and
Accessibility permissions; no permission settings were changed or bypassed.
Automated UI tests run real egui frames but are not a human usability study. Real IME, VoiceOver,
trackpad behavior, Windows and Linux remain unverified. This first release
does not register file associations or implement Finder document-open events;
use the in-app Open button. The bundle is local and not signed/notarized.

Design decisions and the approved implementation plan are in `docs/`.
Local REA evidence from installed Freeplane 1.12.14 is preserved in the
ignored `.evidence/freeplane-evidence.json`; package observations are static,
not runtime performance findings.
