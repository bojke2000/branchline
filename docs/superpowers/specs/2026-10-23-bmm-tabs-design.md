# BMM multi-map tabs

Status: written specification approved by the user; tab implementation has not
started and requires implementation-plan review and execution-method approval.
Implementation baseline: `0108246`, including root-drag panning and dark startup.

## Agreed intent

Work on several independent mind maps in one BMM window, switching without
losing unsaved work, unfinished edits, navigation state or undo history.
New and Open create tabs instead of replacing the current map.

The user selected reopening saved tabs on restart, with recovery of unsaved
work after an interruption. Normal quitting retains explicit Save / Discard /
Cancel protection; it is not an implicit "keep every unsaved draft" hot exit.

The implementation remains original Rust/egui, local-only and macOS-first,
without preventing future Windows/Linux support. Keep the existing `.branchline`
document format, application identity and application-data directory.
Do not modify Freeplane or the REA repository.

## Approach

Use independent in-memory document sessions and one workspace owner.
Reloading a map on each switch would lose editor state and history. Native
macOS window grouping would not provide the requested portable in-window tabs.
Neither alternative is selected.

Extract session state from the current single-document `NativeApp`; do not
create a `NativeApp` or acquire a recovery-store lock for every tab.

## Tab interface and commands

- Place a horizontally scrollable tab strip between the toolbar and editor.
  Show a map name, an unsaved marker, a close button and an accessible `+`.
  Untitled names are distinguishable within the workspace.
- `+`, New and Cmd/Ctrl+N create and activate a blank map in a new tab.
  Open and Cmd/Ctrl+O add and activate a map only after successful loading.
  Existing maps remain unchanged, including unfinished inline and notes edits.
- Importing `.mm` retains its compatibility preview. Acceptance creates a new
  dirty tab without a native save path. Canceling adds no tab and changes no map.
- Opening an already-open native file activates its existing tab, rather than
  creating competing editable copies. Identity lookup is separate from the
  caller-selected path used for display and saving.
- Clicking a tab activates it. Ctrl+Tab / Ctrl+Shift+Tab cycle through tabs.
  Cmd/Ctrl+W and a tab's close button request closing that specific tab.
- Save, Save As, export, search, notes, map commands and undo/redo target the
  active map. The command catalog also supplies the tab commands and hints.
- Closing the final tab creates a fresh blank map. Closing the window or
  Cmd/Ctrl+Q instead requests quitting the whole workspace.
- Dark mode remains the startup default; the theme is global, not per-tab.
  Root dragging continues to pan the entire active map. Branch dragging,
  Shift+arrow reordering and Shift-click multi-selection retain their behavior.

Tab reordering, split views, cross-map drag/drop, cross-map copy/paste, multiple
windows and Finder document-open integration are outside this change.

## Ownership and state isolation

The workspace owns tab order, the active stable tab ID, session creation and
removal, application-wide dialogs, the theme and the recovery coordinator.
Array indices and filenames are not tab identities.

Each document session owns its `Editor`, document identity, save path and
checkpoint, undo/redo history, primary and multiple selection, inline buffer,
notes/link drafts, search state and viewport. Switching tabs does not commit
drafts merely to make them easier to move between owners.

The renderer owns one active-map layout/galley cache. Its key includes stable
tab ID, document revision and theme, so equal revisions or equal imported node
IDs cannot reuse another tab's data. Discard inactive render caches rather than
retaining one full text/layout cache per tab.

Namespace node and text-widget IDs by tab ID. A tab switch cancels map gestures
and closes transient menus/palettes. Keyboard focus and IME ownership cannot
transfer implicitly to another map. Finish or explicitly cancel an active IME
composition before completing a switch; retain the switch request while it is
pending. Late commit/cancellation events must not edit a newly active map.
Uncommitted ordinary text remains with its original session.

Defer workspace transitions until the current frame has finished processing
the source session's widgets. Dialogs and queued actions capture stable tab IDs;
switching or removing a neighboring tab must not redirect a save or deletion.

The workspace owns pending close/quit operations. Remove superseded
single-document replacement/pending-action machinery from the UI/application
boundary once its consumers have migrated; do not layer index-based tab logic
over the old "replace the current document" lifecycle.

## File identity and safe transitions

Native-file deduplication uses filesystem-resolved identity for existing files,
including aliases where the platform can establish identity. Keep the original
selected path separately. An identity/permission failure is reported, not
treated as proof that a file is different or absent.

Save As must not silently write over a file owned by another open tab. Surface
the conflict and require another destination or cancellation. Existing
overwrite confirmation and atomic publication remain in force.

Clean tabs close immediately. A dirty tab, including one with an unfinished
draft, offers Save / Discard / Cancel. Save failure, canceled file selection,
or canceled overwrite leaves that tab open. Discard requires successful
publication of the recovery/session state that removes the discarded draft.

Quitting processes dirty tabs in visible order, identifying each map in its
prompt. Do not remove tabs while collecting decisions. Canceling or failing a
later save aborts quitting and keeps every tab open; earlier successful saves
remain saved. Keep quit-time discard decisions pending until all tabs approve;
aborting quit must not erase recovery for drafts that are still open. Only
after all decisions and required recovery/session writes succeed may the
window close.

After normal quit, retain references to saved tabs and their order/active tab.
A saved map whose newer edits were explicitly discarded reopens from disk.
Discarded untitled/imported maps do not reopen. A normally closed tab is not
part of the next workspace.

Startup reads saved files afresh, not a stale clean-content snapshot. A missing,
changed-format or unreadable saved map is represented by an explicit failed
restore tab with Retry / Locate / Close, not an empty substitute map. Other
valid saved tabs can still open. With no retained tabs, start one blank map.
Selection/view restoration must validate against the freshly loaded document.

## Recovery and compatibility

Keep one exclusive application-data lock for the entire workspace lifetime.
Use one coordinator with independently identified per-document snapshots and
an atomically published versioned workspace manifest. The manifest records tab
order, active identity, saved-file references and recovery snapshot references.

Snapshots include pending text/notes drafts without inserting them into undo
history. A snapshot belongs to its stable session/document identity, not the
active tab at the time the write completes. Debounce per changed session and
reuse unchanged snapshots.

Publish immutable, uniquely named snapshots first, flush them, then atomically
replace the manifest. A failed manifest publication leaves the prior committed
manifest and its referenced snapshots usable. Only clean up superseded,
provably owned snapshots after successful publication; retain cleanup failures
for retry and report them. Never derive a recovery path from a map title or
accept an unchecked path from a manifest.

On interrupted startup, offer Restore / Discard for available unsaved work
without writing into original map files. Restored content is dirty. Discarding
recovery for a saved map reloads its current on-disk content; discarded untitled
maps disappear. Preserve valid recovered tab order and the active identity.

Read the current version-1 `recovery.json` as a legacy single-map candidate.
Before migrating it, create and verify a non-overwriting backup, then publish
the new workspace state. Do not remove or overwrite the legacy record until
the replacement is durable. Migration failure retains the old recovery and
reports the actual reason. Retrying migration reuses a verified matching backup
rather than overwriting an unrelated file or producing unbounded backups.
Document the platform/filesystem durability limits; atomic publication is not
a guarantee against every OS, storage or power-loss failure.

Malformed manifests/snapshots remain on disk and are reported. Do not silently
reset or overwrite an unresolved recovery store. A valid manifest can expose
an individual failed snapshot as a recovery error while retaining other valid
recoveries; invalid global ownership information blocks destructive writes.

## Resource policy

Keep the existing per-document validation, including 100,000 nodes, 64 MiB
decoded text and the 32 MiB file/snapshot limit. Add a 256 MiB estimated
workspace document/session-storage budget, including documents, save
checkpoints, drafts and session metadata, rather than multiplying per-map
limits without a workspace bound. Keep history under the separate bound below.
Check admission before adding/replacing a session or accepting growth; report
capacity without discarding an existing map or draft. Reject over-budget text
or paste explicitly while retaining the previous field contents. A supported
maximum-sized individual map must still be openable alongside the initial
pristine blank tab. A capacity-blocked undo/redo keeps its history entry intact.
Unloaded error tabs carry no document.

Preserve each history stack's existing 32 MiB estimated retention limit and add
a shared 64 MiB estimated undo/redo budget across all tabs, equivalent to the
old single-map worst-case pair of stacks. Evict oldest retained snapshots when
needed, with a notice on affected tabs. This is a retention estimate, not a
promise about total renderer/process RSS. Tab count has no separate arbitrary
limit. These are explicit resource policies, not document-format restrictions
or claims of exact allocator/RSS accounting. Bound manifest input and
serialization before unbounded allocation.

## Verification and delivery

Use existing Rust checks and production-boundary/headless egui tests. Include:

- New/Open/import adding tabs without changing another map's drafts/history;
  canceled dialogs/imports adding nothing.
- Independent undo, notes, selection, search and pan/zoom; equal node IDs and
  equal revision counters across maps; late text/IME events after switching.
- Tab cycling and closing by stable identity, including a close button on an
  inactive tab, first/last-tab transitions and preserving active focus.
- Dirty close Save/Discard/Cancel; failed/canceled Save As; cross-tab save-path
  conflicts; quit cancellation on a later tab without closing earlier tabs.
- Saved-tab restart from fresh disk contents, missing/unreadable file handling,
  multi-map interruption recovery and version-1 recovery migration.
- Failed snapshot/manifest writes, partial publication, malformed recovery,
  cleanup retry, exclusive ownership and aggregate capacity/history eviction.
- Existing root pan, branch drag, multi-delete, IME, document, interchange and
  persistence regressions.

Measure representative one-map editing and two-/five-/ten-map switching and
recovery workflows, separating first and warm runs. Report elapsed time, CPU
time and peak process-family memory; identify unmeasured costs. The old
edit/layout microbenchmark is not a tab-switch or recovery measurement.

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --locked` and `sh scripts/bundle.sh`. Update the README and rebuild
`target/bundle/BMM.app`. Native window readiness can be checked without capture;
real screenshot/accessibility/IME verification remains explicitly limited by
the current macOS permissions. Do not bypass or change those permissions.
