use crate::document::*;
use quick_xml::{Reader, XmlVersion, events::Event};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct FormatError(pub String);

fn error(value: impl std::fmt::Display) -> FormatError {
    FormatError(value.to_string())
}

#[derive(Debug, Clone)]
pub struct ImportWarning {
    pub category: String,
    pub count: usize,
    pub locations: Vec<String>,
}

pub struct ImportReport {
    pub document: Document,
    pub warnings: Vec<ImportWarning>,
}

pub struct ExportReport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<ImportWarning>,
}

pub fn read_native(bytes: &[u8]) -> Result<Document, FormatError> {
    check_size(bytes)?;
    let doc: Document = serde_json::from_slice(bytes).map_err(error)?;
    doc.validate().map_err(error)?;
    Ok(doc)
}

pub fn write_native(doc: &Document) -> Result<Vec<u8>, FormatError> {
    doc.validate().map_err(error)?;
    let bytes = serde_json::to_vec(doc).map_err(error)?;
    check_size(&bytes)?;
    Ok(bytes)
}

fn check_size(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(error("Document exceeds 32 MiB"));
    }
    Ok(())
}

#[derive(Clone)]
enum Frame {
    Map,
    Node(String),
    Note(String),
    Unsupported,
}

fn warn(warnings: &mut BTreeMap<String, ImportWarning>, category: String, location: &str) {
    let entry = warnings.entry(category.clone()).or_insert(ImportWarning {
        category,
        count: 0,
        locations: Vec::new(),
    });
    entry.count += 1;
    if entry.locations.len() < 8 {
        entry.locations.push(location.into());
    }
}

pub fn import_mm(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    check_size(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(error)?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().expand_empty_elements = true;
    let mut frames = Vec::new();
    let mut nodes = BTreeMap::<String, Node>::new();
    let mut warnings = BTreeMap::new();
    let mut root = None;
    let mut map_seen = false;
    let mut text_bytes = 0usize;
    loop {
        match reader.read_event().map_err(error)? {
            Event::Start(element) => {
                if frames.len() >= MAX_DEPTH + 16 {
                    return Err(error("XML nesting exceeds the supported depth"));
                }
                let name = element.name().0.to_owned();
                let mut attrs = BTreeMap::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(error)?;
                    if attribute.value.len() > MAX_FIELD_BYTES {
                        return Err(error("XML attribute exceeds 1 MiB"));
                    }
                    let value = attribute
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(error)?
                        .into_owned();
                    text_bytes += value.len();
                    if text_bytes > MAX_TEXT_BYTES {
                        return Err(error("Decoded text exceeds 64 MiB"));
                    }
                    attrs.insert(attribute.key.0.to_owned(), value);
                }
                let parent = frames.last().cloned();
                let location = match &parent {
                    Some(Frame::Node(id) | Frame::Note(id)) => id.as_str(),
                    _ => "map",
                };
                let frame = match (name.as_str(), &parent) {
                    ("map", None) if !map_seen => {
                        map_seen = true;
                        for key in attrs.keys().filter(|key| key.as_str() != "version") {
                            warn(&mut warnings, format!("attribute:{key}"), "map");
                        }
                        Frame::Map
                    }
                    ("node", Some(Frame::Map | Frame::Node(_))) => {
                        if nodes.len() >= MAX_NODES {
                            return Err(error("Map exceeds 100,000 nodes"));
                        }
                        let id = attrs
                            .remove("ID")
                            .unwrap_or_else(|| Uuid::new_v4().to_string());
                        if id.is_empty() || nodes.contains_key(&id) {
                            return Err(error(format!("Empty or duplicate node ID: {id}")));
                        }
                        let folded = match attrs.remove("FOLDED").as_deref() {
                            None | Some("false") => false,
                            Some("true") => true,
                            _ => return Err(error("FOLDED must be true or false")),
                        };
                        let inherited = match &parent {
                            Some(Frame::Node(parent)) => nodes[parent].side,
                            _ => Side::Right,
                        };
                        let side = match attrs.remove("POSITION").as_deref() {
                            None => inherited,
                            Some("left") => Side::Left,
                            Some("right") => Side::Right,
                            _ => return Err(error("POSITION must be left or right")),
                        };
                        let node = Node {
                            id: id.clone(),
                            text: attrs.remove("TEXT").unwrap_or_default(),
                            notes: String::new(),
                            link: attrs.remove("LINK").unwrap_or_default(),
                            children: Vec::new(),
                            side,
                            folded,
                        };
                        for key in attrs.keys() {
                            warn(&mut warnings, format!("attribute:{key}"), &id);
                        }
                        if let Some(Frame::Node(parent)) = &parent {
                            nodes
                                .get_mut(parent)
                                .expect("parsed parent")
                                .children
                                .push(id.clone());
                        } else if root.replace(id.clone()).is_some() {
                            return Err(error("Map must contain exactly one root node"));
                        }
                        nodes.insert(id.clone(), node);
                        Frame::Node(id)
                    }
                    ("richcontent", Some(Frame::Node(id)))
                        if attrs.get("TYPE").map(String::as_str) == Some("NOTE")
                            && attrs
                                .get("CONTENT-TYPE")
                                .is_some_and(|value| value.starts_with("plain/")) =>
                    {
                        for key in attrs
                            .keys()
                            .filter(|key| !["TYPE", "CONTENT-TYPE"].contains(&key.as_str()))
                        {
                            warn(&mut warnings, format!("attribute:{key}"), id);
                        }
                        if !nodes[id].notes.is_empty() {
                            warn(&mut warnings, "duplicate:NOTE".into(), id);
                        }
                        Frame::Note(id.clone())
                    }
                    ("richcontent", _) => {
                        warn(
                            &mut warnings,
                            format!(
                                "richcontent:{}",
                                attrs.get("TYPE").map(String::as_str).unwrap_or("unknown")
                            ),
                            location,
                        );
                        Frame::Unsupported
                    }
                    (_, None) => return Err(error("Expected a single <map> document")),
                    _ => {
                        warn(&mut warnings, format!("element:{name}"), location);
                        for key in attrs.keys() {
                            warn(&mut warnings, format!("attribute:{key}"), location);
                        }
                        Frame::Unsupported
                    }
                };
                frames.push(frame);
            }
            Event::End(_) => {
                if frames.pop().is_none() {
                    return Err(error("Unexpected closing element"));
                }
            }
            Event::Text(text) => append_text(
                text.as_ref(),
                &frames,
                &mut nodes,
                &mut text_bytes,
                &mut warnings,
            )?,
            Event::CData(text) => append_text(
                text.as_ref(),
                &frames,
                &mut nodes,
                &mut text_bytes,
                &mut warnings,
            )?,
            Event::GeneralRef(reference) => {
                let value = if let Some(character) = reference.resolve_char_ref().map_err(error)? {
                    character.to_string()
                } else {
                    match reference.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return Err(error("Unknown XML entity")),
                    }
                    .to_owned()
                };
                append_text(&value, &frames, &mut nodes, &mut text_bytes, &mut warnings)?;
            }
            Event::DocType(_) => {
                return Err(error("DTD and entity declarations are not supported"));
            }
            Event::PI(_) => warn(&mut warnings, "processing-instruction".into(), "map"),
            Event::Eof => break,
            _ => {}
        }
    }
    if !frames.is_empty() {
        return Err(error("Unclosed XML elements"));
    }
    let document = Document {
        version: 1,
        root: root.ok_or_else(|| error("Map has no root node"))?,
        nodes,
    };
    document.validate().map_err(error)?;
    Ok(ImportReport {
        document,
        warnings: warnings.into_values().collect(),
    })
}

fn append_text(
    text: &str,
    frames: &[Frame],
    nodes: &mut BTreeMap<String, Node>,
    bytes: &mut usize,
    warnings: &mut BTreeMap<String, ImportWarning>,
) -> Result<(), FormatError> {
    *bytes += text.len();
    if *bytes > MAX_TEXT_BYTES {
        return Err(error("Decoded text exceeds 64 MiB"));
    }
    match frames.last() {
        Some(Frame::Note(id)) => {
            let notes = &mut nodes.get_mut(id).expect("parsed note owner").notes;
            if notes.len() + text.len() > MAX_FIELD_BYTES {
                return Err(error("Note exceeds 1 MiB"));
            }
            notes.push_str(text);
        }
        None if !text.trim().is_empty() => return Err(error("Text outside the map")),
        Some(Frame::Node(id)) if !text.trim().is_empty() => {
            warn(warnings, "node-body-text".into(), id)
        }
        _ => {}
    }
    Ok(())
}

fn escaped(text: &str, attribute: bool) -> Result<String, FormatError> {
    if text.chars().any(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
        return Err(error("Text contains characters XML 1.0 cannot represent"));
    }
    let mut value = quick_xml::escape::escape(text).into_owned();
    if attribute {
        value = value
            .replace('\n', "&#10;")
            .replace('\r', "&#13;")
            .replace('\t', "&#9;");
    }
    Ok(value)
}

pub fn export_mm(doc: &Document) -> Result<ExportReport, FormatError> {
    doc.validate().map_err(error)?;
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<map version=\"freeplane 1.9.8\">\n",
    );
    let mut pending = vec![(doc.root.as_str(), false)];
    while let Some((id, close)) = pending.pop() {
        if close {
            output.push_str("</node>\n");
        } else {
            let node = &doc.nodes[id];
            output.push_str(&format!(
                "<node ID=\"{}\" TEXT=\"{}\" FOLDED=\"{}\" POSITION=\"{}\" LINK=\"{}\">\n",
                escaped(id, true)?,
                escaped(&node.text, true)?,
                node.folded,
                if node.side == Side::Left {
                    "left"
                } else {
                    "right"
                },
                escaped(&node.link, true)?
            ));
            if !node.notes.is_empty() {
                output.push_str(&format!(
                    "<richcontent TYPE=\"NOTE\" CONTENT-TYPE=\"plain/auto\">{}</richcontent>\n",
                    escaped(&node.notes, false)?
                ));
            }
            pending.push((id, true));
            pending.extend(node.children.iter().rev().map(|id| (id.as_str(), false)));
        }
        if output.len() > MAX_FILE_BYTES {
            return Err(error("Export exceeds 32 MiB"));
        }
    }
    output.push_str("</map>\n");
    Ok(ExportReport {
        bytes: output.into_bytes(),
        warnings: Vec::new(),
    })
}
