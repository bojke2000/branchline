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

### Windows rendering

Windows now uses a CPU rasterizer with native bitmap presentation by default.
This renders the **existing egui interface**, not a replacement UI: the same
toolbar, artwork, curved branches, typography, themes, text controls, commands
and document/recovery workflows remain in use. Rendering stays at the native
display resolution; display scaling is not reduced.

```powershell
cargo build --release --locked
.\target\release\branchline.exe
# Explicit choices:
.\target\release\branchline.exe --renderer software
.\target\release\branchline.exe --renderer wgpu
```

Save and close an existing instance normally before rebuilding or launching
another against the same recovery store. `--renderer wgpu` retains the previous
rendering path for hardware-accelerated systems or troubleshooting. macOS and
Linux keep their existing wgpu frontend. Renderer errors are reported rather
than silently switching modes; recovery is checkpointed on a returned CPU
renderer failure.

The CPU path uses `hdrshot-egui-backend` (an egui 0.37-compatible software
rasterizer) and `softbuffer`; `egui-winit` supplies native input, IME integration,
clipboard handling and accessibility events. Primitive caching is disabled:
regression tests found stale pixels after texture updates with the dependency's
cache enabled. Tests cover color channels, texture updates, removed shapes,
100/150/200% rasterization and close-request cancellation. Native IME and
screen-reader interaction still require manual platform validation.
Native-window checks verified the original toolbar/branches, F2 text entry,
commit and the unsaved-work prompt on a Windows close request.
Window titles are now published only when their text changes; repeatedly
publishing the same title had requested a new frame indefinitely. A five-second
idle check with the bundled sample observed 1.25% of one CPU core after this fix,
instead of the continuous repaint loop. Save-path and dirty-title updates have
their own regression test.

A separate example measures the actual production UI using only a temporary
map/recovery directory:

```powershell
cargo build --release --locked --example renderer_benchmark
.\target\release\examples\renderer_benchmark.exe software
.\target\release\examples\renderer_benchmark.exe wgpu
# Interactive preview with the bundled sample map; changes are temporary:
.\target\release\examples\renderer_benchmark.exe software --preview
```

At 200% scaling, a 1280 x 840 logical window with scripted inline editing in
one topic measured **8.70 ms median / 13.79 ms p95** with CPU rendering, versus
**404.83 / 438.96 ms** with Microsoft Basic Render Driver (CPU, DX12).
Each run excludes 20 warm-up frames and records 220 frame intervals, counting
each egui frame once even when layout needs multiple passes. Runs reject a
display-scale change during measurement. These are native frame intervals,
not keyboard-to-display or RDP transport latency, and not a large-map or
pan/zoom benchmark. Native text metrics and available system fonts are retained.
A follow-up CPU run after eliminating redundant title updates measured
6.69 ms median / 9.23 ms p95 at the same scale.

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
| Ctrl/Cmd+A | Select the entire map, including root and folded descendants |
| Ctrl/Cmd+C / Ctrl/Cmd+V | Copy selected hierarchies / paste an outline |
| Ctrl/Cmd+Shift+V | Paste an outline as a new map tab |
| Delete / Backspace | Delete selected branches; multi-selection and nonempty subtrees require one confirmation |
| Cmd+Z / Cmd+Shift+Z | Undo / redo |
| Cmd+K / Cmd+F | Command palette / search labels and notes |
| Cmd+N / Cmd+O / Cmd+S | New / open / save |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous mind-map tab |
| Cmd/Ctrl+W | Close the targeted map tab with unsaved-work protection |
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
Label, notes and link edits retain complete valid text up to the 1 MiB UTF-8
field limit. Oversized edits show an error and keep the previous draft rather
than saving a clipped prefix.
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

### Hierarchical clipboard

Physical Ctrl and Cmd both work for A/C/V on macOS. Text fields keep their
ordinary local clipboard behavior. Whole-map selection includes hidden nodes;
Delete retains the root and confirms removal of its descendants. Copy includes
selected subtrees in document order, without duplicating overlapping selections.

Copy publishes tab-indented bullets and escaped nested HTML lists. Paste supports
plain bullets/numbered outlines, nested HTML lists and inline Office `mso-list`
levels. It transfers labels/order/hierarchy, not IDs, notes, links or folding.
Multiline/leading-whitespace plain-text losses warn before copy. Ambiguous
indentation, unsupported Office metadata, malformed HTML or disagreeing
representations require an explicit preview; errors do not silently flatten maps.
Synthetic Office fixtures are not proof of Word/OneNote interoperability.

Paste appends below the selected topic and unfolds it. An untouched blank map
is replaced instead. One top-level topic becomes its root; a forest receives a
“Pasted ideas” root. Paste as new map always creates a tab. Each paste is one
Undo step; unrelated unfinished notes remain with their original topic.
History steps that would orphan unfinished notes are rejected explicitly.

Bounds are 100,000 nodes, depth 256, 1 MiB per field, 64 MiB decoded text and
32 MiB combined clipboard formats, including encoding expansion. Preview shows
only the first 200 topics/160 characters per label, but inserts the full admitted
outline. Admission preserves workspace/history bounds.

macOS reads text/HTML from exactly one pasteboard item, checking its generation
and native UTF-8 lengths before Rust copies. Initial OS/library allocations
are not bounded by that check. Publication uses one dual-format library call,
then verifies both representations; the library clears before writing, so a
failed publication can lose the previous clipboard and is not rollback-safe.
Other platforms currently read plain text only, with an explicit limitation
preview; their native behavior is unverified. No clipboard monitoring occurs.
Native clipboard races, actual Word/OneNote/editor round trips and native IME
were not exercised. Tests inject a transport and never overwrite your clipboard.

Save existing maps and quit the running app normally before opening the rebuilt
`target/bundle/BMM.app`; a second instance cannot acquire its workspace lock.
The build does not quit or relaunch your running app.

## Documents and safety

Native `.branchline` files are versioned JSON. The existing extension, internal
application identifier and recovery location are retained for compatibility
with earlier Branchline builds; the visible product and app bundle are BMM.
The source directory and Cargo executable remain named `branchline`.
Ordinary Save updates the current native file without a replacement prompt;
externally replaced files still produce a conflict rather than being overwritten.
Saves use a flushed temporary
sibling and atomic replacement. Save As and export explicitly confirm existing
destinations. New and Open add independent tabs without replacing an existing
map or committing its unfinished text. Failed/cancelled saves retain that work.
Cmd/Ctrl+W closes the targeted tab; Ctrl+Tab / Ctrl+Shift+Tab cycle tabs.
Closing a dirty tab offers Save / Discard / Cancel. Quit asks about dirty maps
in tab order without removing them: canceling a later prompt keeps every tab,
including earlier pending discards, while successful earlier saves stay saved.
Recovery/session decisions are published before a tab is removed or the window
closes. A failed saved tab offers Retry / Locate / Close rather than a fabricated
empty map. Closing the final tab creates a fresh blank map.

Recovery tracks each tab independently with a 700 ms idle delay, including
unfinished inline text and notes without committing them to undo history.
On macOS, `~/Library/Application Support/org.Branchline.Branchline/` contains
`workspace.json` and immutable UUID-named `snapshot-*.json` files (the exact
platform path is selected by the `directories` crate). One filesystem lock
protects the entire workspace. Saved tabs reopen from current disk contents;
unsaved snapshots and unfinished text restore automatically without a startup
confirmation. Restored unsaved work stays dirty until explicitly saved.
Missing files and individual
snapshot failures remain explicit error tabs while valid siblings survive.

Snapshots are written and flushed before atomic manifest replacement. Only
superseded snapshots with verified filesystem ownership are removed afterward;
failed cleanup remains recorded for retry. Unknown or replaced files are
retained, not glob-deleted. Malformed global manifests block destructive writes.
Version-1 `recovery.json` migrates only after a verified nonoverwriting
`recovery-v1.backup.json`; both legacy files are retained. Each native file,
snapshot and manifest is bounded at 32 MiB, including JSON escaping.

Persistent snapshot ownership uses Unix device/inode and creation time, or
Windows volume serial number, 128-bit file ID and creation time. On Windows,
recovery is stored in `%LOCALAPPDATA%\Branchline\Branchline\data`.
Platforms/filesystems without that proof report
unresolved ownership rather than deleting files. File flushing and atomic
replacement are not a guarantee against every OS, filesystem, storage or
power-loss failure; parent-directory changes are not separately fsynced.
Snapshots from older Windows builds that recorded ownership as unavailable
remain unverifiable and are retained, not automatically trusted or deleted.
Save any still-open maps to native files before restarting an older build.

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

## Mermaid export

Choose **More > Export Mermaid...**, or find it in the command palette.
The scope preview captures the active map, including unfinished labels and all
folded descendants. Choose a UTF-8 `.mmd` destination; existing files require
confirmation. Export preserves the native path, document, drafts, dirty state
and undo history, even if another tab becomes active after capture. Open native
map destinations, including same-file aliases, are rejected.

This is **mindmap export only**, not Mermaid import or flowchart support.
Generated identifiers are separate from labels. Quotes, punctuation, newlines,
Unicode and emoji use numeric entities so text cannot become diagram syntax.
Empty labels and control characters other than LF fail explicitly; nothing is
written. Encoded output is bounded at 32 MiB before output allocation.
Only labels, root, hierarchy and child order transfer; notes, links, IDs,
formatting and BMM layout do not. Mermaid chooses presentation.

Actual local Mermaid **12.2.0** parsing and parsed database hierarchy/order/label
checks passed on synthetic hostile labels using Node **22.21.1** and jsdom
**26.1.0**, with real sanitization and entity decoding. This does not establish
rendered appearance or other Mermaid versions. Large exports may require a
consumer to raise its own text-size/node limits. No user maps go to a remote
renderer. Node is development-only, never a BMM runtime requirement.

```sh
npm ci --prefix tools/mermaid
cargo test --test mermaid actual_mermaid_parser_roundtrip -- --ignored
```

Opening the command palette no longer commits drafts. It owns text input while
open; selecting an editing command still uses that command's ordinary admission.

## Development and verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
cargo run --release --example benchmark
/usr/bin/time -l target/release/examples/benchmark
/usr/bin/time -l target/release/examples/benchmark one-map
/usr/bin/time -l target/release/examples/benchmark tabs 2
/usr/bin/time -l target/release/examples/benchmark tabs 5
/usr/bin/time -l target/release/examples/benchmark tabs 10
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
`cargo test --locked` passed (153 tests; one optional parser test ignored);
`cargo build --release --locked` and `sh scripts/bundle.sh` passed.
These results do not establish the unverified native scenarios listed below.

Measured completed workflows on this development Mac in release mode, with
1,000 generated nodes per map and 21 samples per process:

| Workflow | Maps | First sample | Warm median (20) | Warm p95 |
| --- | --- | --- | --- | --- |
| Edit, commit, Undo, Redo: pre-tabs `c39b19c` | 1 | 1.801 ms | 1.274 ms | 1.752 ms |
| Edit, commit, Undo, Redo: admitted workspace | 1 | 2.296 ms | 1.491 ms | 1.765 ms |
| Switch each tab, edit drafts, publish, reload and restore | 2 | 17.312 ms | 20.025 ms | 22.888 ms |
| Same completed switch/recovery workflow | 5 | 35.398 ms | 41.496 ms | 43.578 ms |
| Same completed switch/recovery workflow | 10 | 57.390 ms | 74.498 ms | 77.820 ms |

Every sample asserts the final active identity, every draft label, all restored
documents and dirty state before reporting. The equivalent one-map comparison
uses the old Editor entry points versus the new admitted Workspace entry
points, with the same generated input and completed edit/undo/redo sequence.
The workspace warm median was 0.217 ms slower in these samples (about 17%):
admission/isolation is additional work, not an established speed improvement.
There is no pre-tabs multi-map recovery equivalent.

Whole-process measurements include setup and all 21 samples, not only timed
workflow bodies:

| Workflow | Elapsed | User CPU | System CPU | Peak process-family RSS |
| --- | --- | --- | --- | --- |
| Pre-tabs one-map baseline | 0.69 s | 0.03 s | 0.00 s | 16,744,448 bytes |
| Admitted one-map editing | 0.36 s | 0.03 s | 0.00 s | 17,629,184 bytes |
| Two-map switch/recovery | 0.42 s | 0.10 s | 0.03 s | 26,624,000 bytes |
| Five-map switch/recovery | 0.87 s | 0.25 s | 0.05 s | 52,346,880 bytes |
| Ten-map switch/recovery | 1.57 s | 0.49 s | 0.09 s | 95,584,256 bytes |

The processes had no children, so their peak RSS is also process-family peak
RSS, not UI renderer memory. First samples separate first use from 20 warm
samples; they are not guaranteed cold OS/filesystem-cache measurements.
The tab scenario includes real local snapshot writes, file flushing, manifest
publication, cleanup, parsing and rehydration. It excludes the intentional
700 ms idle delay, renderer/text layout, native focus/IME, dialogs and human
interaction. The default benchmark still measures document edit/move plus
layout separately. No Freeplane comparison or agent-token measurement was
performed; lower whole-process launch time is not a workflow speedup claim.

Clipboard measurements use release-mode production select/copy, dual-format
serialization, parsing, admitted blank-map replacement, structural assertions,
Undo and Redo. Both channels produce the same independently checked labels,
child order and depth; the plain channel simulates an editor retaining only
the plain representation after BMM's dual-format copy.

| Channel / nodes | First / warm median / warm p95 | Process elapsed / user / system | Peak family RSS |
| --- | --- | --- | --- |
| Rich / 100 | 1.275 / 0.747 / 0.930 ms | 0.23 / 0.02 / 0.00 s | 10,371,072 bytes |
| Rich / 10,000 | 53.166 / 49.014 / 49.672 ms | 1.06 / 0.91 / 0.14 s | 35,569,664 bytes |
| Plain / 100 | 0.263 / 0.206 / 0.216 ms | 0.00 / 0.00 / 0.00 s | 10,190,848 bytes |
| Plain / 10,000 | 28.077 / 26.986 / 27.466 ms | 0.60 / 0.45 / 0.14 s | 34,701,312 bytes |

Reproduce with `cargo build --release --locked --example benchmark`, then
`/usr/bin/time -l target/release/examples/benchmark clipboard-rich 100`
(also `clipboard-plain`, and 10,000 nodes). Each process runs 21 samples;
workflow timers exclude fixture/workspace setup, process figures include it.
The first sample is first-use, not guaranteed OS-cold. No child processes run.
The native clipboard, renderer, external applications and agent tokens are
unmeasured. No prefeature equivalent exists; these are channel comparisons,
not a claimed historical speedup. Earlier tab measurements above remain the
separately recorded tabs-stage measurements.

Completed Mermaid exports additionally measure draft snapshot, bounded encoding,
atomic write/flush, readback and independent source-order/depth/label assertions:

| Nodes | First / warm median / warm p95 | Process elapsed / user / system | Peak family RSS |
| --- | --- | --- | --- |
| 100 | 6.489 / 3.935 / 5.345 ms | 0.31 / 0.01 / 0.02 s | 9,961,472 bytes |
| 10,000 | 12.606 / 10.821 / 11.857 ms | 0.24 / 0.16 / 0.01 s | 26,804,224 bytes |

Use `/usr/bin/time -l target/release/examples/benchmark mermaid 100`
(also 10,000). Each process runs 21 samples, without children. Setup is outside
workflow timers but inside process figures. First-use is not guaranteed cold;
OS/cache/flush variation affects these samples. Optional Node parser costs,
native save dialogs, rendering and agent tokens are unmeasured. There is no
prefeature equivalent, so no historical speedup is claimed.

An earlier pre-tabs macOS ARM64 BMM bundle was launched; its exact executable, launch time and
1280x872 native window were observed, then the owned verification instance was
closed. That established its window readiness, not interactive usability.
The clipboard/Mermaid bundle was rebuilt but was not relaunched here, preserving any running
application and unsaved maps.
REA runtime screenshot
and accessibility capture were blocked by missing macOS Screen Recording and
Accessibility permissions; no permission settings were changed or bypassed.
Automated UI tests run real egui frames but are not a human usability study. At
that earlier macOS stage, real IME, VoiceOver, trackpad behavior, Windows and
Linux were unverified; newer Windows rendering checks are documented above. This first release
does not register file associations or implement Finder document-open events;
use the in-app Open button. The bundle is local and not signed/notarized.

Design decisions and the approved implementation plan are in `docs/`.
Local REA evidence from installed Freeplane 1.12.14 is preserved in the
ignored `.evidence/freeplane-evidence.json`; package observations are static,
not runtime performance findings.
