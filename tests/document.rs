use branchline::document::{Command, Document, History, MovePlacement, Node, Side};

fn execute(doc: &mut Document, history: &mut History, command: Command) -> String {
    history.execute(doc, command).unwrap().unwrap()
}

#[test]
fn editing_round_trip() {
    let mut doc = Document::new("Ideas");
    let initial = doc.clone();
    let root = doc.root.clone();
    let mut history = History::default();
    let first = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "First".into(),
        },
    );
    let second = execute(
        &mut doc,
        &mut history,
        Command::AddSibling {
            sibling: first.clone(),
            text: "Second".into(),
        },
    );
    execute(
        &mut doc,
        &mut history,
        Command::SetText {
            id: second.clone(),
            text: "研究 & ideas".into(),
        },
    );
    execute(
        &mut doc,
        &mut history,
        Command::Indent { id: second.clone() },
    );
    assert_eq!(doc.parent(&second), Some(first.as_str()));
    execute(
        &mut doc,
        &mut history,
        Command::Outdent { id: second.clone() },
    );
    execute(
        &mut doc,
        &mut history,
        Command::MoveSibling {
            id: second.clone(),
            delta: -1,
        },
    );
    assert_eq!(
        doc.nodes[&root].children,
        vec![second.clone(), first.clone()]
    );
    execute(
        &mut doc,
        &mut history,
        Command::Delete { id: first.clone() },
    );
    assert!(!doc.nodes.contains_key(&first));
    let final_doc = doc.clone();
    for _ in 0..7 {
        assert!(history.undo(&mut doc));
    }
    assert_eq!(doc, initial);
    for _ in 0..7 {
        assert!(history.redo(&mut doc));
    }
    assert_eq!(doc, final_doc);
}

#[test]
fn invalid_edits_are_transactional() {
    let mut doc = Document::new("Ideas");
    let mut history = History::default();
    let root = doc.root.clone();
    for command in [
        Command::Delete { id: root.clone() },
        Command::Indent { id: root.clone() },
        Command::Outdent { id: root.clone() },
    ] {
        let before = doc.clone();
        assert!(history.execute(&mut doc, command).is_err());
        assert_eq!(doc, before);
        assert!(!history.undo(&mut doc));
    }
    let child = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "child".into(),
        },
    );
    doc.nodes
        .get_mut(&child)
        .unwrap()
        .children
        .push(root.clone());
    assert!(doc.validate().is_err());
}

#[test]
fn limits_and_deep_tree() {
    let mut doc = Document::new("root");
    let mut parent = doc.root.clone();
    for index in 0..255 {
        let id = format!("n{index}");
        doc.nodes
            .get_mut(&parent)
            .unwrap()
            .children
            .push(id.clone());
        doc.nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                text: "x".into(),
                notes: String::new(),
                link: String::new(),
                children: vec![],
                side: Side::Right,
                folded: false,
            },
        );
        parent = id;
    }
    assert!(doc.validate().is_ok());
    let root = doc.root.clone();
    let spare = execute(
        &mut doc,
        &mut History::default(),
        Command::AddChild {
            parent: root,
            text: "Spare branch".into(),
        },
    );
    let before = doc.clone();
    let mut history = History::default();
    assert!(
        doc.plan_move(&spare, &MovePlacement::Child(parent.clone()))
            .is_err()
    );
    assert!(
        history
            .execute(
                &mut doc,
                Command::MoveNode {
                    id: spare,
                    placement: MovePlacement::Child(parent.clone())
                }
            )
            .is_err()
    );
    assert_eq!(
        doc, before,
        "Depth rejection must leave the original graph untouched"
    );
    assert!(!history.undo(&mut doc));
    let id = "too-deep".to_owned();
    doc.nodes
        .get_mut(&parent)
        .unwrap()
        .children
        .push(id.clone());
    doc.nodes.insert(
        id.clone(),
        Node {
            id,
            text: "x".into(),
            notes: String::new(),
            link: String::new(),
            children: vec![],
            side: Side::Right,
            folded: false,
        },
    );
    assert!(doc.validate().is_err());
}

#[test]
fn metadata_is_undoable_and_noop_does_not_consume_history() {
    let mut doc = Document::new("root");
    let root = doc.root.clone();
    let original = doc.clone();
    let mut history = History::default();
    history
        .execute(
            &mut doc,
            Command::SetText {
                id: root.clone(),
                text: "root".into(),
            },
        )
        .unwrap();
    assert!(!history.undo(&mut doc));
    for command in [
        Command::SetNotes {
            id: root.clone(),
            notes: "Remember this".into(),
        },
        Command::SetLink {
            id: root.clone(),
            link: "https://example.org/a?b=1&c=2".into(),
        },
        Command::ToggleFold { id: root },
    ] {
        history.execute(&mut doc, command).unwrap();
    }
    assert_eq!(doc.nodes[&doc.root].notes, "Remember this");
    for _ in 0..3 {
        assert!(history.undo(&mut doc));
    }
    assert_eq!(doc, original);
}

#[test]
fn moving_a_folded_subtree_preserves_content_and_is_one_undo_step() {
    let mut doc = Document::new("root");
    let mut history = History::default();
    let root = doc.root.clone();
    let first = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "First".into(),
        },
    );
    let child = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: first.clone(),
            text: "Descendant".into(),
        },
    );
    let second = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root,
            text: "Second".into(),
        },
    );
    doc.nodes.get_mut(&first).unwrap().folded = true;
    doc.nodes.get_mut(&second).unwrap().folded = true;
    doc.nodes.get_mut(&child).unwrap().notes = "Keep notes".into();
    doc.nodes.get_mut(&child).unwrap().link = "https://example.org".into();
    let before = doc.clone();
    let generation = history.generation;
    execute(
        &mut doc,
        &mut history,
        Command::MoveNode {
            id: first.clone(),
            placement: MovePlacement::Child(second.clone()),
        },
    );
    assert_eq!(doc.parent(&first), Some(second.as_str()));
    assert!(doc.nodes[&first].folded);
    assert!(!doc.nodes[&second].folded);
    for id in [&first, &child] {
        assert_eq!(doc.nodes[id].side, Side::Left);
    }
    assert_eq!(doc.nodes[&child].notes, "Keep notes");
    assert_eq!(doc.nodes[&child].link, "https://example.org");
    assert_eq!(history.generation, generation + 1);
    let after = doc.clone();
    assert!(history.undo(&mut doc));
    assert_eq!(doc, before);
    assert!(history.redo(&mut doc));
    assert_eq!(doc, after);
}

#[test]
fn moves_reject_cycles_roots_and_unknown_destinations_transactionally() {
    let mut doc = Document::new("root");
    let mut setup = History::default();
    let root = doc.root.clone();
    let parent = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: root.clone(),
            text: "Parent".into(),
        },
    );
    let child = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: parent.clone(),
            text: "Child".into(),
        },
    );
    let original = doc.clone();
    let mut history = History::default();
    for (id, placement) in [
        (root, MovePlacement::Child(child.clone())),
        (parent.clone(), MovePlacement::Child(child.clone())),
        (parent.clone(), MovePlacement::Before(child.clone())),
        (parent.clone(), MovePlacement::After(parent.clone())),
        (child, MovePlacement::Child("missing".into())),
    ] {
        assert!(
            history
                .execute(&mut doc, Command::MoveNode { id, placement })
                .is_err()
        );
        assert_eq!(doc, original);
        assert!(!history.undo(&mut doc));
    }
}

#[test]
fn same_parent_drop_and_side_changes_keep_correct_positions() {
    let mut doc = Document::new("root");
    let mut history = History::default();
    let root = doc.root.clone();
    let first = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "A".into(),
        },
    );
    let second = execute(
        &mut doc,
        &mut history,
        Command::AddSibling {
            sibling: first.clone(),
            text: "B".into(),
        },
    );
    let third = execute(
        &mut doc,
        &mut history,
        Command::AddSibling {
            sibling: second.clone(),
            text: "C".into(),
        },
    );
    execute(
        &mut doc,
        &mut history,
        Command::MoveNode {
            id: third.clone(),
            placement: MovePlacement::Before(first.clone()),
        },
    );
    assert_eq!(
        doc.nodes[&root].children,
        [third.clone(), first.clone(), second.clone()]
    );
    execute(
        &mut doc,
        &mut history,
        Command::MoveNode {
            id: third.clone(),
            placement: MovePlacement::After(second.clone()),
        },
    );
    assert_eq!(
        doc.nodes[&root].children,
        [first, second.clone(), third.clone()]
    );
    let generation = history.generation;
    execute(
        &mut doc,
        &mut history,
        Command::MoveNode {
            id: third.clone(),
            placement: MovePlacement::After(second),
        },
    );
    assert_eq!(
        history.generation, generation,
        "Dropping in the existing position must not use undo history"
    );
    execute(
        &mut doc,
        &mut history,
        Command::MoveNode {
            id: third.clone(),
            placement: MovePlacement::RootSide(Side::Left),
        },
    );
    assert_eq!(doc.nodes[&third].side, Side::Left);
    assert_eq!(doc.parent(&third), Some(root.as_str()));
}

#[test]
fn root_reorder_uses_visible_siblings_on_the_same_side() {
    let mut doc = Document::new("root");
    let mut history = History::default();
    let root = doc.root.clone();
    let right = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "Right first".into(),
        },
    );
    let left = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "Left".into(),
        },
    );
    let last = execute(
        &mut doc,
        &mut history,
        Command::AddChild {
            parent: root.clone(),
            text: "Right last".into(),
        },
    );
    execute(
        &mut doc,
        &mut history,
        Command::MoveSibling {
            id: last.clone(),
            delta: -1,
        },
    );
    let visible: Vec<_> = doc.nodes[&root]
        .children
        .iter()
        .filter(|id| doc.nodes[*id].side == Side::Right)
        .cloned()
        .collect();
    assert_eq!(visible, [last, right]);
    assert_eq!(doc.nodes[&left].side, Side::Left);
}

#[test]
fn batch_delete_deduplicates_subtrees_retains_root_and_undoes_atomically() {
    let mut doc = Document::new("root");
    let mut setup = History::default();
    let root = doc.root.clone();
    let parent = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: root.clone(),
            text: "Parent".into(),
        },
    );
    let child = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: parent.clone(),
            text: "Child".into(),
        },
    );
    let other = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: root.clone(),
            text: "Other".into(),
        },
    );
    let retained = execute(
        &mut doc,
        &mut setup,
        Command::AddChild {
            parent: root.clone(),
            text: "Keep".into(),
        },
    );
    let ids = vec![root.clone(), parent.clone(), child, other.clone(), parent];
    let plan = doc.plan_deletion(&ids).unwrap();
    assert_eq!(plan.roots.len(), 2);
    assert_eq!(plan.node_count, 3);
    assert!(plan.root_retained);
    let before = doc.clone();
    let mut history = History::default();
    execute(&mut doc, &mut history, Command::DeleteMany { ids });
    assert_eq!(doc.nodes.len(), 2);
    assert!(doc.nodes.contains_key(&retained));
    assert_eq!(doc.nodes[&root].children, [retained]);
    assert!(history.undo(&mut doc));
    assert_eq!(doc, before);
    assert!(!history.undo(&mut doc));
    assert!(
        history
            .execute(
                &mut doc,
                Command::DeleteMany {
                    ids: vec![other, "missing".into()]
                }
            )
            .is_err()
    );
    assert_eq!(doc, before);
    assert!(
        history
            .execute(&mut doc, Command::DeleteMany { ids: vec![root] })
            .is_err()
    );
}
