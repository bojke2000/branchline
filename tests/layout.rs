use branchline::{
    document::{Command, Document, History},
    layout::*,
};
use std::collections::BTreeMap;

#[test]
fn branches_do_not_overlap() {
    let mut doc = Document::new("root");
    let root = doc.root.clone();
    let mut history = History::default();
    for text in ["a very wide label", "left", "tall", "left again"] {
        history
            .execute(
                &mut doc,
                Command::AddChild {
                    parent: root.clone(),
                    text: text.into(),
                },
            )
            .unwrap();
    }
    let sizes = doc
        .nodes
        .keys()
        .map(|id| {
            (
                id.clone(),
                Size {
                    width: 240.0,
                    height: 90.0,
                },
            )
        })
        .collect();
    let placed = layout(&doc, &sizes).unwrap();
    assert_eq!(placed, layout(&doc, &sizes).unwrap());
    for (a_id, a) in &placed.nodes {
        for (b_id, b) in &placed.nodes {
            if a_id != b_id {
                assert!(!a.intersects(*b), "{a_id} overlaps {b_id}");
            }
        }
    }
    assert_eq!(placed.nodes[&doc.root].center(), Point { x: 0.0, y: 0.0 });
    assert_eq!(placed.edges.len(), 4);
}

#[test]
fn folded_descendants_are_absent() {
    let mut doc = Document::new("root");
    let root = doc.root.clone();
    let mut history = History::default();
    let child = history
        .execute(
            &mut doc,
            Command::AddChild {
                parent: root.clone(),
                text: "child".into(),
            },
        )
        .unwrap()
        .unwrap();
    let grandchild = history
        .execute(
            &mut doc,
            Command::AddChild {
                parent: child.clone(),
                text: "hidden".into(),
            },
        )
        .unwrap()
        .unwrap();
    history
        .execute(&mut doc, Command::ToggleFold { id: child.clone() })
        .unwrap();
    let placed = layout(&doc, &BTreeMap::new()).unwrap();
    assert!(placed.nodes.contains_key(&root));
    assert!(placed.nodes.contains_key(&child));
    assert!(!placed.nodes.contains_key(&grandchild));
    history
        .execute(&mut doc, Command::ToggleFold { id: root })
        .unwrap();
    assert_eq!(layout(&doc, &BTreeMap::new()).unwrap().nodes.len(), 1);
}

#[test]
fn invalid_geometry_is_rejected() {
    let doc = Document::new("root");
    let sizes = BTreeMap::from([(
        doc.root.clone(),
        Size {
            width: f32::NAN,
            height: -1.0,
        },
    )]);
    assert!(layout(&doc, &sizes).is_err());
}
