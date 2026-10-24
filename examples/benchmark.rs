use branchline::{
    app::{DraftField, Editor, EditorAction, EditorEdit},
    document::{Command, Document, History, MovePlacement, Node, Side},
    layout::layout,
    outline_format::{PasteInterpretation, parse_clipboard, serialize_outline},
    recovery::{RecoveryCoordinator, RestoreContent},
    session::{MapSession, SessionId},
    workspace::{PasteTarget, Workspace},
};
use std::{collections::BTreeMap, time::Instant};

fn generated(count: usize) -> Document {
    let mut doc = Document::new("Benchmark");
    let mut ids = vec![doc.root.clone()];
    for index in 1..count {
        let id = format!("node-{index}");
        let parent = ids[(index - 1) / 4].clone();
        let side = if index % 2 == 0 {
            Side::Left
        } else {
            Side::Right
        };
        doc.nodes
            .get_mut(&parent)
            .unwrap()
            .children
            .push(id.clone());
        doc.nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                text: format!("Idea {index}"),
                notes: String::new(),
                link: String::new(),
                children: vec![],
                side,
                folded: false,
            },
        );
        ids.push(id);
    }
    doc
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args
        .get(1)
        .is_some_and(|mode| matches!(mode.as_str(), "clipboard-rich" | "clipboard-plain"))
    {
        let count = args.get(2).expect("node count").parse().unwrap();
        assert!([100, 1_000, 10_000].contains(&count));
        clipboard_workflow(count, args[1] == "clipboard-rich");
        return;
    }
    if args.get(1).is_some_and(|mode| mode == "one-map") {
        edit_workflow();
        return;
    }
    if args.get(1).is_some_and(|mode| mode == "tabs") {
        let count: usize = args.get(2).expect("tab count").parse().unwrap();
        assert!([2, 5, 10].contains(&count));
        tabs_workflow(count);
        return;
    }
    for count in [100, 1_000, 10_000] {
        let mut doc = generated(count);
        let mut history = History::default();
        let sizes = BTreeMap::new();
        let mut times = Vec::new();
        for index in 0..21 {
            let start = Instant::now();
            let root = doc.root.clone();
            history
                .execute(
                    &mut doc,
                    Command::SetText {
                        id: root,
                        text: format!("Iteration {index}"),
                    },
                )
                .unwrap();
            std::hint::black_box(layout(&doc, &sizes).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        report("edit_layout", "nodes", count, times);
        history = History::default();
        let mut times = Vec::new();
        for index in 0..21 {
            let start = Instant::now();
            history
                .execute(
                    &mut doc,
                    Command::MoveNode {
                        id: "node-5".into(),
                        placement: MovePlacement::Child(
                            if index % 2 == 0 { "node-2" } else { "node-1" }.into(),
                        ),
                    },
                )
                .unwrap();
            std::hint::black_box(layout(&doc, &sizes).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        report("move_layout", "nodes", count, times);
    }
}

fn edit_workflow() {
    let mut workspace = Workspace::new();
    workspace.map_mut().editor = Editor::from_document(generated(1_000), None, true, None).unwrap();
    let id = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    let mut times = Vec::new();
    for sample in 0..21 {
        let expected = format!("Completed edit {sample}");
        let start = Instant::now();
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Edit))
            .unwrap();
        workspace
            .apply(
                id,
                EditorEdit::SetDraft {
                    field: DraftField::Inline(root.clone()),
                    text: expected.clone(),
                },
            )
            .unwrap();
        workspace.apply(id, EditorEdit::CommitDrafts).unwrap();
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Undo))
            .unwrap();
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Redo))
            .unwrap();
        assert_eq!(workspace.active_id(), id);
        assert_eq!(workspace.map().editor.document.nodes[&root].text, expected);
        assert!(workspace.map().editor.editing.is_none());
        times.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    report("completed_edit_undo_redo", "nodes", 1_000, times);
}

fn clipboard_workflow(count: usize, rich: bool) {
    let mut source = Editor::from_document(generated(count), None, true, None).unwrap();
    let mut times = Vec::new();
    for _ in 0..21 {
        let mut destination = Workspace::new();
        let id = destination.active_id();
        let before = destination.map().editor.document.clone();
        let target = PasteTarget::Append {
            session: id,
            node: before.root.clone(),
            revision: destination.map().editor.revision,
        };
        let start = Instant::now();
        source.select_all();
        let forest = source.copy_forest().unwrap();
        let mut copy = serialize_outline(&forest).unwrap();
        assert!(copy.warnings.is_empty());
        if !rich {
            copy.payload.html = None;
        }
        let PasteInterpretation::Ready(parsed) = parse_clipboard(&copy.payload).unwrap() else {
            panic!("generated single-line hierarchy must round-trip unambiguously");
        };
        destination.paste_outline(target, parsed.forest).unwrap();
        let document = &destination.map().editor.document;
        assert_eq!(document.nodes.len(), count);
        document.validate().unwrap();
        let mut pending = vec![(source.document.root.as_str(), document.root.as_str(), 1)];
        let mut visited = 0;
        let mut max_depth = 0;
        while let Some((original, copied, depth)) = pending.pop() {
            let original = &source.document.nodes[original];
            let copied = &document.nodes[copied];
            assert_eq!(copied.text, original.text);
            assert_eq!(copied.children.len(), original.children.len());
            assert!(!source.document.nodes.contains_key(&copied.id));
            pending.extend(
                original
                    .children
                    .iter()
                    .zip(&copied.children)
                    .map(|(left, right)| (left.as_str(), right.as_str(), depth + 1)),
            );
            visited += 1;
            max_depth = max_depth.max(depth);
        }
        assert_eq!(visited, count);
        assert!(max_depth > 1 && max_depth <= branchline::document::MAX_DEPTH);
        let pasted = document.clone();
        destination
            .apply(id, EditorEdit::Action(EditorAction::Undo))
            .unwrap();
        assert_eq!(destination.map().editor.document, before);
        destination
            .apply(id, EditorEdit::Action(EditorAction::Redo))
            .unwrap();
        assert_eq!(destination.map().editor.document, pasted);
        assert!(destination.map().editor.is_dirty());
        times.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    report(
        if rich {
            "select_copy_dual_parse_rich_paste_undo_redo"
        } else {
            "select_copy_dual_parse_plain_paste_undo_redo"
        },
        "nodes",
        count,
        times,
    );
}

fn tabs_workflow(count: usize) {
    let directory = tempfile::tempdir().unwrap();
    let mut recovery = RecoveryCoordinator::open(directory.path().into()).unwrap();
    let mut workspace = Workspace::new();
    workspace.map_mut().editor = Editor::from_document(generated(1_000), None, true, None).unwrap();
    let mut ids = vec![workspace.active_id()];
    for _ in 1..count {
        let mut map = MapSession::new_blank(SessionId::new());
        map.editor = Editor::from_document(generated(1_000), None, true, None).unwrap();
        ids.push(workspace.insert_session(map).unwrap());
    }
    let mut times = Vec::new();
    for sample in 0..21 {
        let start = Instant::now();
        for (index, id) in ids.iter().copied().enumerate() {
            workspace.request_activate(id).unwrap();
            workspace.apply_pending_activation().unwrap();
            workspace
                .apply(id, EditorEdit::Action(EditorAction::Edit))
                .unwrap();
            let root = workspace.map().editor.selected.clone();
            workspace
                .apply(
                    id,
                    EditorEdit::SetDraft {
                        field: DraftField::Inline(root),
                        text: format!("Map {index}, sample {sample}"),
                    },
                )
                .unwrap();
        }
        assert_eq!(workspace.active_id(), *ids.last().unwrap());
        for (index, id) in ids.iter().copied().enumerate() {
            assert_eq!(
                workspace
                    .session(id)
                    .unwrap()
                    .editor
                    .editing
                    .as_ref()
                    .unwrap()
                    .text,
                format!("Map {index}, sample {sample}")
            );
        }
        assert!(
            recovery
                .publish(&workspace.recovery_state())
                .unwrap()
                .cleanup_error
                .is_none()
        );
        let restore = recovery.load().unwrap();
        assert_eq!(restore.active, ids.last().copied());
        for (index, restored) in restore.sessions.iter().enumerate() {
            let RestoreContent::Recovered(document) = &restored.content else {
                panic!("expected recovered draft");
            };
            assert_eq!(
                document.nodes[&document.root].text,
                format!("Map {index}, sample {sample}")
            );
        }
        let restored = Workspace::from_restore(restore, true).unwrap();
        assert_eq!(restored.order(), ids);
        assert_eq!(restored.active_id(), *ids.last().unwrap());
        for (index, id) in ids.iter().copied().enumerate() {
            let map = restored.session(id).unwrap();
            assert_eq!(
                map.editor.document.nodes[&map.editor.document.root].text,
                format!("Map {index}, sample {sample}")
            );
            assert!(map.editor.is_dirty());
        }
        drop(restored);
        times.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    report("switch_drafts_publish_restore", "maps", count, times);
}

fn report(operation: &str, dimension: &str, count: usize, mut times: Vec<f64>) {
    let first = times.remove(0);
    times.sort_by(f64::total_cmp);
    println!(
        "operation={operation} {dimension}={count} first_sample_ms={first:.3} warm_n=20 median_ms={:.3} p95_ms={:.3}",
        times[10], times[18]
    );
    if count == 1_000 {
        assert!(
            times[18] < 100.0,
            "1,000-node {operation} p95 exceeds 100 ms goal"
        );
    }
}
