use branchline::document::{Command, Document, History, Node, Side};

fn execute(doc: &mut Document, history: &mut History, command: Command) -> String {
    history.execute(doc, command).unwrap().unwrap()
}

#[test]
fn editing_round_trip() {
    let mut doc = Document::new("Ideas");
    let initial = doc.clone();
    let root = doc.root.clone();
    let mut history = History::default();
    let first = execute(&mut doc, &mut history, Command::AddChild { parent: root.clone(), text: "First".into() });
    let second = execute(&mut doc, &mut history, Command::AddSibling { sibling: first.clone(), text: "Second".into() });
    execute(&mut doc, &mut history, Command::SetText { id: second.clone(), text: "研究 & ideas".into() });
    execute(&mut doc, &mut history, Command::Indent { id: second.clone() });
    assert_eq!(doc.parent(&second), Some(first.as_str()));
    execute(&mut doc, &mut history, Command::Outdent { id: second.clone() });
    execute(&mut doc, &mut history, Command::MoveSibling { id: second.clone(), delta: -1 });
    assert_eq!(doc.nodes[&root].children, vec![second.clone(), first.clone()]);
    execute(&mut doc, &mut history, Command::Delete { id: first.clone() });
    assert!(!doc.nodes.contains_key(&first));
    let final_doc = doc.clone();
    for _ in 0..7 { assert!(history.undo(&mut doc)); }
    assert_eq!(doc, initial);
    for _ in 0..7 { assert!(history.redo(&mut doc)); }
    assert_eq!(doc, final_doc);
}

#[test]
fn invalid_edits_are_transactional() {
    let mut doc = Document::new("Ideas");
    let mut history = History::default();
    let root = doc.root.clone();
    for command in [Command::Delete { id: root.clone() }, Command::Indent { id: root.clone() }, Command::Outdent { id: root.clone() }] {
        let before = doc.clone();
        assert!(history.execute(&mut doc, command).is_err());
        assert_eq!(doc, before);
        assert!(!history.undo(&mut doc));
    }
    let child = execute(&mut doc, &mut history, Command::AddChild { parent: root.clone(), text: "child".into() });
    doc.nodes.get_mut(&child).unwrap().children.push(root.clone());
    assert!(doc.validate().is_err());
}

#[test]
fn limits_and_deep_tree() {
    let mut doc = Document::new("root");
    let mut parent = doc.root.clone();
    for index in 0..255 {
        let id = format!("n{index}");
        doc.nodes.get_mut(&parent).unwrap().children.push(id.clone());
        doc.nodes.insert(id.clone(), Node { id: id.clone(), text: "x".into(), notes: String::new(), link: String::new(), children: vec![], side: Side::Right, folded: false });
        parent = id;
    }
    assert!(doc.validate().is_ok());
    let id = "too-deep".to_owned();
    doc.nodes.get_mut(&parent).unwrap().children.push(id.clone());
    doc.nodes.insert(id.clone(), Node { id, text: "x".into(), notes: String::new(), link: String::new(), children: vec![], side: Side::Right, folded: false });
    assert!(doc.validate().is_err());
}

#[test]
fn metadata_is_undoable_and_noop_does_not_consume_history() {
    let mut doc = Document::new("root");
    let root = doc.root.clone();
    let original = doc.clone();
    let mut history = History::default();
    history.execute(&mut doc, Command::SetText { id: root.clone(), text: "root".into() }).unwrap();
    assert!(!history.undo(&mut doc));
    for command in [
        Command::SetNotes { id: root.clone(), notes: "Remember this".into() },
        Command::SetLink { id: root.clone(), link: "https://example.org/a?b=1&c=2".into() },
        Command::ToggleFold { id: root },
    ] { history.execute(&mut doc, command).unwrap(); }
    assert_eq!(doc.nodes[&doc.root].notes, "Remember this");
    for _ in 0..3 { assert!(history.undo(&mut doc)); }
    assert_eq!(doc, original);
}
