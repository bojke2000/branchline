use branchline::{
    document::{Document, MAX_FIELD_BYTES, MAX_FILE_BYTES, Node, Side},
    mermaid::export_mermaid,
};

fn hierarchy(labels: &[&str]) -> Document {
    let mut document = Document::new(labels[0]);
    let root = document.root.clone();
    for (index, label) in labels.iter().enumerate().skip(1) {
        let id = format!("source-{index}");
        document.nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                text: (*label).into(),
                notes: String::new(),
                link: String::new(),
                children: Vec::new(),
                side: Side::Right,
                folded: false,
            },
        );
        let parent = if index == 2 { "source-1" } else { &root };
        document.nodes.get_mut(parent).unwrap().children.push(id);
    }
    document
}

#[test]
fn mermaid_preserves_order_and_folded_hierarchy() {
    let mut doc = hierarchy(&["Root", "First", "Hidden", "Last"]);
    doc.nodes.get_mut("source-1").unwrap().folded = true;
    assert_eq!(
        String::from_utf8(export_mermaid(&doc).unwrap()).unwrap(),
        "mindmap\n  bmm_0[\"Root\"]\n    bmm_1[\"First\"]\n      bmm_2[\"Hidden\"]\n    bmm_3[\"Last\"]\n"
    );
}

#[test]
fn mermaid_labels_cannot_escape_their_node() {
    let doc = hierarchy(&[
        "Root",
        "\"]\n%%{init:bad}%%\n:::class ::icon(x) [`tick`] &amp; 😀",
    ]);
    let text = String::from_utf8(export_mermaid(&doc).unwrap()).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.contains("bmm_1[\""));
    assert!(!text.contains("%%{"));
    assert!(!text.contains("::icon"));
    assert!(!text.contains("<"));
    assert!(!text.contains("&amp;"));
    for label in ["", "\0", "before\u{1}after", "\t", "\r"] {
        assert!(
            export_mermaid(&Document::new(label))
                .unwrap_err()
                .to_string()
                .contains("label")
        );
    }
}

#[test]
fn mermaid_output_capacity_is_preflighted() {
    let label = "\"".repeat(MAX_FIELD_BYTES);
    let mut doc = hierarchy(&[
        "Root", &label, &label, &label, &label, &label, &label, &label, &label,
    ]);
    assert!(export_mermaid(&doc).is_err());
    // 32 MiB exactly, including syntax, is accepted; one extra ASCII byte is not.
    let mut exact = hierarchy(&["Root"]);
    for index in 0..32 {
        let id = format!("wide-{index}");
        exact
            .nodes
            .get_mut(&exact.root.clone())
            .unwrap()
            .children
            .push(id.clone());
        exact.nodes.insert(
            id.clone(),
            Node {
                id,
                text: "x".repeat(MAX_FIELD_BYTES - 64),
                notes: String::new(),
                link: String::new(),
                children: Vec::new(),
                side: Side::Right,
                folded: false,
            },
        );
    }

    let initial = export_mermaid(&exact).unwrap().len();
    let missing = MAX_FILE_BYTES - initial;
    let available = exact.nodes.get_mut(&exact.root.clone()).unwrap();
    available.text.push_str(&"x".repeat(missing));
    assert_eq!(export_mermaid(&exact).unwrap().len(), MAX_FILE_BYTES);
    exact
        .nodes
        .get_mut(&exact.root.clone())
        .unwrap()
        .text
        .push('x');
    assert!(export_mermaid(&exact).is_err());
    doc.nodes.get_mut("source-1").unwrap().text = "\0".into();
    assert!(export_mermaid(&doc).is_err());
}

#[test]
#[ignore = "requires the pinned development-only Mermaid dependencies and Node >=22.12"]
fn actual_mermaid_parser_roundtrip() {
    let labels = [
        "Root",
        "\"]\n%%{init:{securityLevel:'loose'}}%%\n:::class ::icon(x) [`tick`] &amp; #34; 😀",
        " leading <script>alert(1)</script> [link](https://example.invalid) trailing ",
        "é 中文 ﬂ°°34¶ß &lt; \nlast",
    ];
    let document = hierarchy(&labels);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("synthetic.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "text": String::from_utf8(export_mermaid(&document).unwrap()).unwrap(),
            "nodes": [
                {"id":"bmm_0","label":labels[0],"parent":null},
                {"id":"bmm_1","label":labels[1],"parent":"bmm_0"},
                {"id":"bmm_2","label":labels[2],"parent":"bmm_1"},
                {"id":"bmm_3","label":labels[3],"parent":"bmm_0"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tools/mermaid/verify.mjs"
        ))
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
