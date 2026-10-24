use branchline::outline_format::{
    ClipboardPayload, OutlineFormat, OutlineWarningKind, PasteInterpretation, parse_clipboard,
    serialize_outline,
};
use branchline::{
    app::{DraftField, EditorAction, EditorEdit},
    document::{Command, MAX_DEPTH, MAX_FIELD_BYTES, MAX_NODES, MAX_TEXT_BYTES},
    outline::{OutlineForest, OutlineNode},
    session::{MapSession, SessionId},
    workspace::{PasteTarget, Workspace},
};
use std::collections::BTreeSet;

fn ready(payload: ClipboardPayload) -> OutlineForest {
    match parse_clipboard(&payload).unwrap() {
        PasteInterpretation::Ready(parsed) => parsed.forest,
        other => panic!("Expected unambiguous hierarchy, got {other:?}"),
    }
}

#[test]
fn html_and_plain_copy_have_equivalent_hierarchy() {
    let expected = OutlineForest {
        roots: vec![
            node(
                "Plan & ideas",
                vec![node("Résumé 🚀", vec![]), node("", vec![])],
            ),
            node("Next", vec![]),
        ],
    };
    for payload in [
        ClipboardPayload {
            html: Some(include_str!("fixtures/clipboard/nested.html").into()),
            plain: None,
        },
        ClipboardPayload {
            html: None,
            plain: Some("- Plan & ideas\n\t- Résumé 🚀\n\t- \n- Next".into()),
        },
        ClipboardPayload {
            html: None,
            plain: Some("1. Plan & ideas\n  * Résumé 🚀\n  * \n2. Next".into()),
        },
        ClipboardPayload {
            html: None,
            plain: Some("- Plan & ideas\n    - Résumé 🚀\n    - \n- Next".into()),
        },
    ] {
        assert_eq!(ready(payload), expected);
    }
    let copy = serialize_outline(&expected).unwrap();
    assert!(copy.warnings.is_empty());
    assert_eq!(
        copy.payload.plain.as_deref(),
        Some("- Plan & ideas\n\t- Résumé 🚀\n\t- \n- Next\n")
    );
    assert_eq!(ready(copy.payload.clone()), expected);
    assert_eq!(
        ready(ClipboardPayload {
            plain: copy.payload.plain,
            html: None
        }),
        expected
    );
}

#[test]
fn office_list_levels_are_source_facts() {
    let report = parse_clipboard(&ClipboardPayload {
        html: Some(include_str!("fixtures/clipboard/office.html").into()),
        plain: None,
    })
    .unwrap();
    let PasteInterpretation::Ready(parsed) = report else {
        panic!("supported Office metadata should be unambiguous")
    };
    assert_eq!(parsed.source, OutlineFormat::OfficeHtml);
    assert_eq!(
        parsed.forest,
        OutlineForest {
            roots: vec![
                node(
                    "Plan & ideas",
                    vec![node("Résumé 🚀", vec![]), node("", vec![])]
                ),
                node("Next", vec![])
            ]
        }
    );
    assert!(
        parsed
            .warnings
            .iter()
            .all(|warning| warning.category == OutlineWarningKind::IgnoredContent)
    );
}

#[test]
fn html_wrappers_do_not_become_topics_and_item_blocks_keep_line_breaks() {
    assert_eq!(
        ready(ClipboardPayload {
            plain: None,
            html: Some("<div><p>A</p><p>B</p></div>".into()),
        }),
        OutlineForest {
            roots: vec![node("A", vec![]), node("B", vec![])]
        }
    );
    assert_eq!(
        ready(ClipboardPayload {
            plain: None,
            html: Some("<ul><li><p>A</p><p>B</p></li></ul>".into()),
        }),
        OutlineForest {
            roots: vec![node("A\nB", vec![])]
        }
    );
}

#[test]
fn html_fragment_scripts_and_escaped_office_css_are_parsed_offline() {
    assert_eq!(ready(ClipboardPayload {
        plain: None, html: Some("<p>Outside</p><!--StartFragment--><li>A<ul><li>B</ul><!--EndFragment--><p>Outside</p>".into()),
    }), OutlineForest { roots: vec![node("A", vec![node("B", vec![])])] });
    assert_eq!(ready(ClipboardPayload {
        plain: None, html: Some(r#"<p style="mso-list:l0 lev\65l1 lfo1">A</p><p style="mso-list:l0 lev\65l2 lfo1"><span style="mso-list:Ignore">•</span>B<script>window.location="https://invalid.example";</script></p>"#.into()),
    }), OutlineForest { roots: vec![node("A", vec![node("B", vec![])])] });
    let many = "<p class=MsoListParagraph>A</p>".repeat(64);
    let PasteInterpretation::Preview(candidates) = parse_clipboard(&ClipboardPayload {
        plain: None,
        html: Some(many),
    })
    .unwrap() else {
        panic!("unsupported class metadata must be disclosed")
    };
    let warning = &candidates[0].warnings[0];
    assert_eq!(warning.count, 64);
    assert_eq!(warning.locations.len(), 8);
}

#[test]
fn ambiguous_outline_requires_preview() {
    for payload in [
        ClipboardPayload {
            plain: Some("- A\n  - B\n      - C".into()),
            html: None,
        },
        ClipboardPayload {
            plain: Some("- A\n\t- B\n    - C".into()),
            html: None,
        },
        ClipboardPayload {
            plain: None,
            html: Some("<p class=MsoListParagraph>A</p><p class=MsoListParagraph>B</p>".into()),
        },
        ClipboardPayload {
            plain: None,
            html: Some("<p>Intro</p><ul><li>A</li></ul>".into()),
        },
        ClipboardPayload {
            plain: Some("- A\n- B".into()),
            html: Some("<ul><li>A<ul><li>B</li></ul></li></ul>".into()),
        },
    ] {
        let PasteInterpretation::Preview(candidates) = parse_clipboard(&payload).unwrap() else {
            panic!("ambiguous hierarchy must not silently import")
        };
        assert!(!candidates.is_empty() && candidates.len() <= 2);
        assert!(
            candidates
                .iter()
                .any(|candidate| !candidate.warnings.is_empty())
        );
        assert!(
            candidates
                .iter()
                .flat_map(|candidate| &candidate.warnings)
                .all(|warning| warning.locations.len() <= 8)
        );
    }
}

#[test]
fn skipped_office_levels_keep_same_level_siblings_in_the_preview() {
    let payload = ClipboardPayload { plain: None, html: Some(
        "<p style='mso-list:l0 level1 lfo1'>A</p><p style='mso-list:l0 level3 lfo1'>B</p><p style='mso-list:l0 level3 lfo1'>C</p>".into()
    ) };
    let PasteInterpretation::Preview(choices) = parse_clipboard(&payload).unwrap() else {
        panic!("missing Office levels require preview")
    };
    assert_eq!(
        choices[0].forest,
        OutlineForest {
            roots: vec![node("A", vec![node("B", vec![]), node("C", vec![])])]
        }
    );
}

#[test]
fn rich_multiline_labels_survive_and_plain_normalization_is_disclosed() {
    let forest = OutlineForest {
        roots: vec![node("  First\nSecond\r<&\"", vec![node("  ", vec![])])],
    };
    let copy = serialize_outline(&forest).unwrap();
    assert!(
        copy.warnings
            .iter()
            .any(|warning| warning.category == OutlineWarningKind::RepresentationLoss)
    );
    assert_eq!(
        ready(ClipboardPayload {
            html: copy.payload.html.clone(),
            plain: None
        }),
        forest
    );
    let PasteInterpretation::Preview(choices) = parse_clipboard(&copy.payload).unwrap() else {
        panic!("lossy plain alternative must not hide its disagreement")
    };
    assert_eq!(choices[0].forest, forest);
    assert_eq!(choices[1].forest.roots[0].label, "  First Second <&\"");
}

#[test]
fn invalid_plain_alternatives_retain_the_reason_in_a_rich_preview() {
    let payload = ClipboardPayload {
        html: Some("<ul><li>A</li></ul>".into()),
        plain: Some("x".repeat(MAX_FIELD_BYTES + 1)),
    };
    let PasteInterpretation::Preview(choices) = parse_clipboard(&payload).unwrap() else {
        panic!("an invalid alternative must not disappear")
    };
    assert_eq!(choices[0].forest.roots[0].label, "A");
    assert!(choices[0].warnings.iter().any(|warning| {
        warning
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("1 MiB"))
    }));
}

#[test]
fn malformed_html_and_unfinished_fragments_are_not_presented_as_preserved() {
    for html in [
        "<ul><li>A</wrong></ul>",
        "<!--StartFragment--><ul><li>A</li></ul>",
        "<ul><li>A &unknown;</li></ul>",
    ] {
        let PasteInterpretation::Preview(candidates) = parse_clipboard(&ClipboardPayload {
            plain: None,
            html: Some(html.into()),
        })
        .unwrap() else {
            panic!("recovered malformed structure requires review: {html}");
        };
        assert_eq!(candidates[0].forest.roots[0].children.len(), 0);
        assert!(
            candidates[0]
                .warnings
                .iter()
                .any(|warning| warning.category == OutlineWarningKind::RepresentationLoss)
        );
    }
}

#[test]
fn copy_expansion_is_checked_before_allocation() {
    use branchline::document::MAX_FILE_BYTES;
    let mut forest = OutlineForest {
        roots: vec![node("", vec![]); 16],
    };
    let empty = serialize_outline(&forest).unwrap().payload;
    let overhead = empty.plain.unwrap().len() + empty.html.unwrap().len();
    let mut remaining = MAX_FILE_BYTES - overhead;
    if remaining % 2 == 1 {
        forest.roots[0].label.push('<');
        remaining -= 5;
    }
    let mut fill = remaining / 2;
    for root in &mut forest.roots {
        let bytes = fill.min(MAX_FIELD_BYTES - root.label.len());
        root.label.extend(std::iter::repeat_n('x', bytes));
        fill -= bytes;
    }
    assert_eq!(fill, 0);
    let output = serialize_outline(&forest).unwrap().payload;
    assert_eq!(
        output.plain.unwrap().len() + output.html.unwrap().len(),
        MAX_FILE_BYTES
    );
    assert!(forest.roots[0].label.starts_with('<'));
    forest.roots[0].label.replace_range(..1, "&");
    assert!(serialize_outline(&forest).is_err());
    let line = format!("- {}\n", "x".repeat(MAX_FIELD_BYTES - 3));
    let mut plain = line.repeat(MAX_FILE_BYTES / line.len());
    assert_eq!(plain.len(), MAX_FILE_BYTES);
    assert_eq!(
        ready(ClipboardPayload {
            plain: Some(plain.clone()),
            html: None
        })
        .roots
        .len(),
        32
    );
    plain.push(' ');
    assert!(
        parse_clipboard(&ClipboardPayload {
            plain: Some(plain),
            html: None
        })
        .is_err()
    );
}

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
