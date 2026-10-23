# Branchline

A local-first, keyboard-driven mind mapper written in Rust with egui/eframe.
Made for quick personal brainstorming: a quiet two-sided canvas, automatic
layout, inline editing, folding, searchable commands and a plain-text notes panel.

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
open target/bundle/Branchline.app
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
| Escape | Cancel inline editing or close search/commands |
| Arrows | Navigate branches and siblings |
| Space | Fold/unfold the selected branch |
| Alt+Up / Alt+Down | Reorder among siblings |
| Alt+Right / Alt+Left | Indent under preceding sibling / outdent |
| Delete / Backspace | Delete; nonempty subtrees require confirmation |
| Cmd+Z / Cmd+Shift+Z | Undo / redo |
| Cmd+K / Cmd+F | Command palette / search labels and notes |
| Cmd+N / Cmd+O / Cmd+S | New / open / save |
| Cmd+Shift+S | Save as |
| Cmd+Q | Quit with unsaved-change protection |
| Drag blank canvas | Pan |
| Scroll / pinch | Zoom around pointer |
| Fit map / Focus | Fit all visible nodes / center selection |

Use Ctrl instead of Cmd on other platforms. Text fields retain cursor keys
and local text undo. Click blank canvas after using another control to return
keyboard focus to the map. The root cannot be deleted or outdented; Enter on
the root creates a child. New branches alternate sides; descendants inherit
their root branch's layout side. Folding, notes and metadata are undoable.

## Documents and safety

Native `.branchline` files are versioned JSON. Saves use a flushed temporary
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

Executed on this Mac: `cargo fmt --check` passed;
`cargo clippy --all-targets -- -D warnings` passed;
`cargo test --locked` passed (30 integration tests);
`cargo build --release --locked` and `sh scripts/bundle.sh` passed.
These results do not establish the unverified native scenarios listed below.

Measured on this development Mac, optimized generated-map edit plus layout:

| Nodes | First sample | Warm median (20) | Warm p95 |
| --- | --- | --- | --- |
| 100 | 0.115 ms | 0.077 ms | 0.085 ms |
| 1,000 | 1.041 ms | 0.931 ms | 0.958 ms |
| 10,000 | 11.262 ms | 11.381 ms | 11.475 ms |

The sampled process used 0.27 s elapsed, 0.26 s user CPU and <0.01 s system
CPU, with maximum resident size 61,571,072 bytes. The benchmark process had no
children; this is not UI renderer memory. First samples are not guaranteed
cold OS-cache measurements. Rendering, text measurement, dialogs and real
interaction are excluded. There is no equivalent Freeplane benchmark or
agent-token measurement, so no relative speed/context claim is made.

macOS ARM64 build and native window creation were observed. REA runtime screenshot
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
