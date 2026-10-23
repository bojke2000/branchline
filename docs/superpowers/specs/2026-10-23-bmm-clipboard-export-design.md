# BMM hierarchical clipboard and Mermaid export

Status: written specification approved by the user; clipboard and Mermaid
implementation has not started. Implementation requires written-plan review
and execution-method approval.

Implementation baseline: `c39b19c`, including keyboard panning and
type-to-replace editing.

## Agreed intent and scope

Make BMM useful for moving brainstorming hierarchies between mind maps,
Word, OneNote and plain-text editors. Copying out must retain labels, sibling
order and nesting; pasting an outline back must produce editable branches.
The user explicitly requested Control+A/C/V; support both physical Ctrl and
platform Cmd shortcuts on the map, including on macOS.

Mermaid is a separate, one-way export: BMM to Mermaid `mindmap`.
Mermaid import, flowcharts, arbitrary graphs and diagram rendering are not
requested. Mermaid export does not replace bidirectional bullet interchange.

The user selected pasting beneath the active node, or building the map directly
when starting from an untouched blank map. A separate Paste as new map command
creates a tab. One pasted top-level item becomes a blank map's root; multiple
top-level items get a new root named "Pasted ideas".

This extends the separately reviewed tabs design. Cross-map clipboard
interchange becomes supported through this feature, although it was excluded
from the original tabs-only scope. Cross-map dragging remains excluded.
Implement tabs and its session ownership first; do not create a second,
competing document/recovery owner for Paste as new map.

Only labels and ordered hierarchy transfer. Notes, links, styles, fold state,
positions and BMM node IDs are not part of external outline interchange.
All descendants transfer even when folded; pasted nodes get fresh IDs.
This is not a native-document backup or a lossless styling round trip.

## Approach and boundaries

Use semantic rich HTML lists with a readable indented plain-text alternative.
Plain text alone is simpler but cannot reliably recover Office nesting.
An RTF-first implementation adds another substantial parser without proving
better Word/OneNote compatibility, so it is not the initial approach.

Separate three owners:

- A platform clipboard adapter reads and publishes clipboard representations.
  Keep native APIs and clipboard ownership out of document/editor code.
- An outline boundary parses formats into one validated ordered forest of
  labels and children, retaining format identity, warnings and source locations.
  Consumers use this normalized structure, not their own raw HTML/text parsing.
- Document/editor commands insert the forest or build a map as one validated
  transaction. The UI routes commands and presents failures or previews.

Use a maintained format-aware HTML parser. Do not parse HTML or Office list
metadata with a collection of regular expressions. Choose and verify the
native clipboard API/library during implementation planning, including whether
it can read and publish HTML and text together on macOS. Do not claim
Windows/Linux rich clipboard support from macOS or in-memory tests.

Native clipboard publication should make HTML and text available in one
logical copy. Do not clear the clipboard before payload construction succeeds.
Report acquisition/publication failures; do not claim a successful copy after
publishing only an unintended format. Document any platform publication limits.

## Selection and command behavior

Ctrl/Cmd+A selects the entire active map, including its root and folded
descendants. Highlight visible nodes and show the total selection count,
including how many are currently hidden.

Represent whole-map coverage explicitly in the Editor selection model. Do not
put invisible node IDs into the ordinary multi-selection set: existing folding
and undo reconciliation must continue removing hidden secondary selections.
Keep the active node identity separate from selection coverage.

Whole-map selection makes the root active. Copy traverses the complete map.
Delete still retains the root and requires the existing explicit confirmation
for deleting its descendants; the confirmation counts hidden descendants too.
Ordinary clicks, navigation, starting an edit and single-node structural
commands leave whole-map mode. Folding can preserve whole-map coverage.
Undo/redo reconcile back to an ordinary visible selection.

Ctrl/Cmd+C copies selected branches and their descendants. Deduplicate selected
ancestors/descendants and traverse document order, not ID-map order or click
order. Copying a selected root produces the entire hierarchy. Copy does not
change the document, history or selection.

Ctrl/Cmd+V pastes beneath the active node without replacing existing children.
Append roots in source order, unfold the destination and reveal the inserted
nodes. For an untouched blank session, use the root-building rules above.
Track that blank-session state explicitly; a loaded or intentionally named
single-node map must not be mistaken for a placeholder by matching its label.

Paste as new map uses the same parser and builder, and creates a tab only after
successful validation and admission. Cancellation or failure adds no tab.
The new map is dirty and has no native save path.

Beginning a paste captures its stable target session/node identity. A preview,
tab switch or late clipboard result must not redirect the operation. Check the
target again before publishing; explain a stale destination instead of guessing.
Select the first inserted root after a successful append. Each append or
blank-map replacement is one undoable transaction; failure leaves the old
document, history and selection intact.

Map commands share the command catalog, palette and context-menu dispatch.
Text fields retain native/local Select All, Copy and Paste. In particular,
pasting into an inline label edits text rather than importing a forest.
Do not route Paste events through type-to-replace handling.

## Clipboard representations and parsing

Copy publishes escaped label text in nested HTML `ul`/`li` lists and a UTF-8
plain outline using tab indentation and a `- ` marker. Both represent the same
forest in the same order. Encode line breaks inside HTML labels without
turning them into sibling nodes.

Plain outlines cannot unambiguously preserve every possible multiline or
leading-whitespace label. Disclose this limitation rather than silently
claiming plain-text losslessness; retain the full label in the HTML alternative.
Warn before publishing a plain alternative that necessarily changes label
representation. Ordinary single-line labels require no warning.

On explicit map paste, prefer supported rich hierarchy facts when available.
Accept semantic nested HTML lists and recognize supported Office list-level
metadata. Numbered lists may supply hierarchy, but numbering becomes bullets
on export because BMM has no ordered-list numbering semantics.

For plain text, recognize bullet/number markers, tabs and consistent
space-indented nesting. Derive a consistent indentation unit rather than
assuming every application uses four spaces. Do not infer depth from a bullet
glyph alone. Treat unindented separate items as a forest, not an invented tree.
Blank separator lines do not generate nodes; explicit empty list items may.

Mixed or skipped indentation, unsupported Office list metadata, paragraphs
mixed with lists, and competing rich/plain interpretations require a preview
with the proposed tree and specific limitations. Allow explicit selection of
the plain-text interpretation where useful. Never silently flatten rich input
or guess a hierarchy while presenting it as preserved.

The preview can accept an unambiguous interpretation or cancel. If no
supported interpretation exists, retain the clipboard and destination and show
the reason. Do not require a new outline-repair editor for this release.

Read only when the user explicitly invokes paste. Never poll or persist the
ambient clipboard. Parse offline: no scripts, images, remote stylesheets,
external resources, URL opening or target execution. Ignore formatting not
needed for label text or list structure; report meaningful omissions.

## Mermaid export

Add Export Mermaid... alongside existing Freeplane export, using the shared
command catalog and native save dialog. Save UTF-8 `.mmd` beginning with
`mindmap`, then indentation-based parent/child structure in document order.
Include the root and all descendants, including folded ones.

Use generated safe identifiers independently of labels. Quote/escape labels
according to verified Mermaid mindmap syntax, including Unicode, quotes,
delimiters, newlines and text resembling directives. Labels must not introduce
new nodes, configuration directives, links or executable HTML. Never paste
untrusted labels directly into syntax positions.

Export a validated snapshot of the active session, including unfinished label
text, without committing drafts merely to export. Reuse the session-owned
draft-snapshot mechanism used for recovery instead of duplicating buffer
patching. Export itself does not modify history, clear dirty state, change the
native save path or replace a map.

Use existing atomic-write and overwrite-confirmation behavior. A canceled
dialog, rejected overwrite, invalid draft or failed write is not success.
Explain in the export UI that labels/hierarchy transfer but BMM metadata and
layout do not. Mermaid determines its own presentation.

Syntax reference: <https://mermaid.js.org/syntax/mindmap.html>.
Rendered compatibility must be characterized with a local Mermaid parser or
renderer where available, using synthetic non-sensitive diagrams. Do not send
user maps to a remote renderer.

## Resource and failure policy

Apply existing document bounds: 100,000 nodes, depth 256, 1 MiB per decoded
field and 64 MiB total decoded text. Include the destination's existing
contents, insertion depth and synthetic root in admission checks.

Reuse the 32 MiB file budget for total application-read/published clipboard
payload across formats and for Mermaid output. This is an explicit transfer
resource policy, not a Mermaid/Office format restriction. Reject oversize with
an explanation rather than truncating nodes or labels.

Account for HTML/entity escaping, indentation and simultaneous representations
before allocating output. Bound normalized structures, parser work, nesting
and Rust-controlled temporary storage. Where native APIs expose sizes, check
before copying payloads into Rust; report native allocation limits honestly.
Do not claim these bounds constrain another application's or the OS's memory.

Reuse tab workspace document/history admission and capacity reporting. Build
and validate the prospective mutation before publication; do not mutate nodes
and then attempt to repair an invalid or over-budget document. Preserve drafts
and old clipboard contents where the platform permits on a failed operation.
Expected failures use the existing typed Result/error conventions.

## Verification and delivery

Add a few focused document and real-egui command-routing regressions, plus
format-boundary fixtures covering:

- Full-map selection with folded descendants/root, overlapping subsets and
  source sibling order; root-safe batch deletion and visible selection after
  undo or leaving whole-map mode.
- Ctrl and Cmd canvas shortcuts versus search, notes and inline editing;
  Paste events remaining distinct from typed replacement.
- Copy/paste of nested bullets, numbered lists, supported Office HTML,
  non-ASCII labels, empty items and source text resembling markup.
- Single-root and forest paste into a blank map; append beneath a folded node;
  one-step undo; fresh IDs; canceled previews and stale tab/node destinations.
- Ambiguous/malformed input, external-resource attempts, clipboard failures,
  exact capacity edges and output representation expansion before allocation.
- Mermaid root/order/folded coverage, hostile delimiter/directive labels,
  escaping, draft snapshots, file cancellation/overwrite and write failures.

Fixtures and simulated clipboard adapters do not establish real Word/OneNote
compatibility. Use opt-in native round trips where available and authorized;
do not overwrite unrelated clipboard content as part of the default test suite.
Report applications/versions actually exercised and any remaining limitations.
Do not advertise "perfect" Office interoperability without that evidence.

Measure representative completed copy and paste tasks, including a nested
small outline and larger bounded maps, and Mermaid export. Report elapsed
time, CPU time and peak process-family memory, separating first and warm runs.
Label unmeasured native clipboard and Office costs.

Run formatting, strict all-target Clippy, the locked test suite and bundle
build. Update README with exact format scope and verified coverage. Preserve
root panning, keyboard panning, type-to-replace, selection, recovery and tab
lifecycle regressions. Do not close an existing user app or bypass macOS
permissions to obtain native verification.
