use branchline::{document::{Command, Document, History}, interchange::*};

#[test]
fn supported_fields_round_trip() {
    let mut doc = Document::new("Ideas < & \"研究\"");
    let root = doc.root.clone();
    let mut history = History::default();
    history.execute(&mut doc, Command::AddChild { parent: root.clone(), text: "right".into() }).unwrap();
    history.execute(&mut doc, Command::AddChild { parent: root.clone(), text: "left".into() }).unwrap();
    history.execute(&mut doc, Command::SetNotes { id: root.clone(), notes: "line one\nline two & <three>".into() }).unwrap();
    history.execute(&mut doc, Command::SetLink { id: root.clone(), link: "https://example.org/?a=1&b=2".into() }).unwrap();
    history.execute(&mut doc, Command::ToggleFold { id: root }).unwrap();
    assert_eq!(read_native(&write_native(&doc).unwrap()).unwrap(), doc);
    let exported = export_mm(&doc).unwrap();
    assert!(exported.warnings.is_empty());
    let imported = import_mm(&exported.bytes).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    assert_eq!(imported.document, doc);
}

#[test]
fn unsupported_content_is_reported() {
    let report = import_mm(br#"<map version="freeplane 1.9.8"><node ID="root" TEXT="hello" CREATED="1"><font SIZE="20"/><icon BUILTIN="yes"/><arrowlink DESTINATION="x"/><hook NAME="something"><node ID="not-a-child" TEXT="ignored"/></hook><richcontent TYPE="NOTE"><html><body><b>rich</b></body></html></richcontent><node ID="child" TEXT="child"/></node></map>"#).unwrap();
    assert_eq!(report.document.nodes.len(), 2);
    for category in ["attribute:CREATED", "element:font", "element:icon", "element:arrowlink", "element:hook", "richcontent:NOTE"] {
        assert!(report.warnings.iter().any(|warning| warning.category == category && warning.count > 0 && !warning.locations.is_empty()), "{category}: {:?}", report.warnings);
    }
}

#[test]
fn hostile_xml_is_rejected() {
    for input in [
        "<!DOCTYPE map [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><map><node TEXT='&x;'/></map>".to_owned(),
        "<map><node ID='a'><node ID='a'/></node></map>".to_owned(),
        "<map><node/><node/></map>".to_owned(),
        "<map><node FOLDED='maybe'/></map>".to_owned(),
        format!("<map>{}{} </map>", "<node>".repeat(257), "</node>".repeat(257)),
        "<map><node TEXT='&notDefined;'/></map>".to_owned(),
        "<map><node></map>".to_owned(),
        "<map><node POSITION='elsewhere'/></map>".to_owned(),
    ] { assert!(import_mm(input.as_bytes()).is_err(), "accepted {}", &input[..input.len().min(120)]); }
    assert!(import_mm(&[0xff, 0xfe]).is_err());
    assert!(read_native(br#"{"version":99,"root":"r","nodes":{}}"#).is_err());
    let mut value = serde_json::to_value(Document::new("x")).unwrap();
    value["unexpected"] = true.into();
    assert!(read_native(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn plain_notes_and_missing_ids_are_supported_without_loss() {
    let report = import_mm(br#"<map><node TEXT="one"><richcontent TYPE="NOTE" CONTENT-TYPE="plain/auto">a &amp; b</richcontent><node TEXT="two"/></node></map>"#).unwrap();
    assert!(report.warnings.is_empty());
    assert_eq!(report.document.nodes.len(), 2);
    assert_eq!(report.document.nodes[&report.document.root].notes, "a & b");
}
