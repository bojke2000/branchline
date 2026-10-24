use branchline::{
    app::{DraftField, EditorAction, EditorEdit},
    document::{Command, MAX_DEPTH, MAX_FIELD_BYTES, MAX_NODES, MAX_TEXT_BYTES},
    outline::{OutlineForest, OutlineNode},
    session::{MapSession, SessionId},
    workspace::{PasteTarget, Workspace},
};
use std::collections::BTreeSet;

fn node(label: &str, children: Vec<OutlineNode>) -> OutlineNode {
    OutlineNode {
        label: label.into(),
        children,
    }
}

fn tree() -> OutlineForest {
    OutlineForest {
        roots: vec![node("A", vec![node("B", vec![]), node("C", vec![])])],
    }
}

fn target(workspace: &Workspace) -> PasteTarget {
    let map = workspace.map();
    PasteTarget::Append {
        session: map.id(),
        node: map.editor.selected.clone(),
        revision: map.editor.revision,
    }
}

#[test]
fn outline_append_is_one_undo_step() {
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    let root = workspace.map().editor.document.root.clone();
    workspace
        .apply(
            id,
            EditorEdit::Command(Command::SetText {
                id: root.clone(),
                text: "Existing".into(),
            }),
        )
        .unwrap();
    workspace
        .apply(
            id,
            EditorEdit::Command(Command::ToggleFold { id: root.clone() }),
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let old: BTreeSet<_> = before.nodes.keys().cloned().collect();
    let generation = workspace.map().editor.history.generation;
    workspace.paste_outline(target(&workspace), tree()).unwrap();
    let editor = &workspace.map().editor;
    let inserted = editor.document.nodes[&root].children[0].clone();
    assert_eq!(editor.selected, inserted);
    assert!(!editor.document.nodes[&root].folded);
    assert_eq!(editor.document.nodes[&inserted].text, "A");
    let labels: Vec<_> = editor.document.nodes[&inserted]
        .children
        .iter()
        .map(|id| editor.document.nodes[id].text.as_str())
        .collect();
    assert_eq!(labels, ["B", "C"]);
    let mut new: BTreeSet<_> = editor.document.nodes[&inserted]
        .children
        .iter()
        .cloned()
        .collect();
    new.insert(inserted);
    assert_eq!(new.len(), 3);
    assert_eq!(editor.document.nodes.len(), old.len() + new.len());
    assert!(new.is_disjoint(&old));
    assert_eq!(editor.history.generation, generation + 1);
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert_eq!(workspace.map().editor.document, before);
}

#[test]
fn blank_replacement_clears_both_unchanged_placeholder_drafts() {
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    workspace.apply(id, EditorEdit::BeginDetails(root)).unwrap();
    assert!(workspace.map().editor.editing.is_some());
    assert!(workspace.map().editor.details.is_some());
    assert!(workspace.map().is_pristine_blank());
    workspace.paste_outline(target(&workspace), tree()).unwrap();
    assert!(workspace.map().editor.editing.is_none());
    assert!(workspace.map().editor.details.is_none());
    assert_eq!(
        workspace.map().editor.document.nodes[&workspace.map().editor.document.root].text,
        "A"
    );
}

#[test]
fn outline_paste_does_not_flush_an_unrelated_owned_notes_draft() {
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(id, EditorEdit::BeginDetails(root.clone()))
        .unwrap();
    workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Notes(root.clone()),
                text: "Still drafting notes".into(),
            },
        )
        .unwrap();
    let generation = workspace.map().editor.history.generation;
    workspace.paste_outline(target(&workspace), tree()).unwrap();
    let editor = &workspace.map().editor;
    assert_eq!(editor.document.nodes[&root].notes, "");
    assert_eq!(
        editor.details.as_ref().unwrap().notes,
        "Still drafting notes"
    );
    assert_eq!(editor.history.generation, generation + 1);
}

#[test]
fn blank_outline_roots_are_built_without_placeholder_guessing() {
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    let old = workspace.map().editor.document.clone();
    workspace.paste_outline(target(&workspace), tree()).unwrap();
    let editor = &workspace.map().editor;
    assert_eq!(editor.document.nodes[&editor.document.root].text, "A");
    let labels: Vec<_> = editor.document.nodes[&editor.document.root]
        .children
        .iter()
        .map(|id| editor.document.nodes[id].text.as_str())
        .collect();
    assert_eq!(labels, ["B", "C"]);
    assert!(
        editor
            .document
            .nodes
            .keys()
            .all(|id| !old.nodes.contains_key(id))
    );
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert_eq!(workspace.map().editor.document, old);

    let forest = OutlineForest {
        roots: vec![node("One", vec![]), node("Two", vec![])],
    };
    let created = workspace
        .paste_outline(PasteTarget::NewMap, forest)
        .unwrap();
    let map = workspace.session(created).unwrap();
    assert_eq!(
        map.editor.document.nodes[&map.editor.document.root].text,
        "Pasted ideas"
    );
    assert!(map.editor.is_dirty());
    assert!(map.editor.path.is_none());
    assert_eq!(workspace.len(), 2);

    let mut loaded = MapSession::new_blank(SessionId::new());
    loaded.editor = branchline::app::Editor::from_document(
        branchline::document::Document::new("Central idea"),
        None,
        true,
        None,
    )
    .unwrap();
    let loaded_id = workspace.insert_session(loaded).unwrap();
    workspace.apply_pending_activation().unwrap();
    workspace.paste_outline(target(&workspace), tree()).unwrap();
    let doc = &workspace.session(loaded_id).unwrap().editor.document;
    assert_eq!(doc.nodes[&doc.root].text, "Central idea");
    assert_eq!(doc.nodes.len(), 4);
}

#[test]
fn paste_depth_and_capacity_failure_is_atomic() {
    let mut chain = node("Deep", vec![]);
    for _ in 1..MAX_DEPTH {
        chain = node("Deep", vec![chain]);
    }
    let forest = OutlineForest { roots: vec![chain] };
    forest.validate().unwrap();
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(
            id,
            EditorEdit::Command(Command::SetText {
                id: root,
                text: "Named map".into(),
            }),
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    assert!(workspace.paste_outline(target(&workspace), forest).is_err());
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(workspace.map().editor.history.generation, generation);
    let too_many = OutlineForest {
        roots: vec![node("", vec![]); MAX_NODES],
    };
    assert!(
        workspace
            .paste_outline(PasteTarget::NewMap, too_many)
            .is_err()
    );
    assert_eq!(workspace.len(), 1);
    assert_eq!(workspace.map().editor.document, before);
}

#[test]
fn exact_outline_limits_and_synthetic_root_overhead_are_enforced() {
    let mut chain = node("Deep", vec![]);
    for _ in 1..MAX_DEPTH {
        chain = node("Deep", vec![chain]);
    }
    let mut workspace = Workspace::new();
    workspace
        .paste_outline(
            PasteTarget::NewMap,
            OutlineForest {
                roots: vec![chain.clone()],
            },
        )
        .unwrap();
    let count = workspace.len();
    assert!(
        workspace
            .paste_outline(
                PasteTarget::NewMap,
                OutlineForest {
                    roots: vec![chain, node("", vec![])]
                }
            )
            .is_err()
    );
    assert_eq!(workspace.len(), count);
    let forest = OutlineForest {
        roots: vec![node("Root", vec![node("", vec![]); MAX_NODES - 1])],
    };
    let id = workspace
        .paste_outline(PasteTarget::NewMap, forest)
        .unwrap();
    assert_eq!(
        workspace.session(id).unwrap().editor.document.nodes.len(),
        MAX_NODES
    );
    let mut text = OutlineForest {
        roots: vec![node(&"x".repeat(MAX_FIELD_BYTES), vec![]); MAX_TEXT_BYTES / MAX_FIELD_BYTES],
    };
    text.validate().unwrap();
    text.roots.push(node("x", vec![]));
    assert!(text.validate().unwrap_err().to_string().contains("64 MiB"));
    text.roots.pop();
    text.roots[0].label.push('x');
    assert!(text.validate().unwrap_err().to_string().contains("label"));
}

#[test]
fn stale_paste_targets_are_not_redirected_to_another_session_or_revision() {
    let mut workspace = Workspace::new();
    let captured = target(&workspace);
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(
            workspace.active_id(),
            EditorEdit::Command(Command::SetText {
                id: root,
                text: "Changed".into(),
            }),
        )
        .unwrap();
    workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    let before = workspace.map().editor.document.clone();
    assert!(workspace.paste_outline(captured, tree()).is_err());
    assert_eq!(workspace.map().editor.document, before);
}
