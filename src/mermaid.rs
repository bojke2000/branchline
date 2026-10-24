use crate::{
    document::{Document, MAX_FILE_BYTES},
    interchange::FormatError,
    outline_format::TextSink,
};

pub fn export_mermaid(document: &Document) -> Result<Vec<u8>, FormatError> {
    document
        .validate()
        .map_err(|error| FormatError(error.to_string()))?;
    let mut counter = TextSink::count(MAX_FILE_BYTES);
    encode(document, &mut counter)?;
    let mut output = TextSink::output(counter.bytes);
    encode(document, &mut output)?;
    Ok(output.finish().into_bytes())
}

fn encode(document: &Document, output: &mut TextSink) -> Result<(), FormatError> {
    output.push("mindmap\n")?;
    let mut pending = vec![(document.root.as_str(), 1)];
    let mut index = 0;
    while let Some((id, depth)) = pending.pop() {
        let node = &document.nodes[id];
        if node.text.is_empty() || node.text.chars().any(|c| c.is_control() && c != '\n') {
            return Err(FormatError("Mermaid cannot preserve an empty or control-character label; no export was written.".into()));
        }
        for _ in 0..depth {
            output.push("  ")?;
        }
        output.push(&format!("bmm_{index}[\""))?;
        let mut start = 0;
        for (offset, character) in node.text.char_indices() {
            if !character.is_ascii_alphanumeric() && character != ' ' {
                output.push(&node.text[start..offset])?;
                output.push(&format!("#{};", character as u32))?;
                start = offset + character.len_utf8();
            }
        }
        output.push(&node.text[start..])?;
        output.push("\"]\n")?;
        pending.extend(
            node.children
                .iter()
                .rev()
                .map(|child| (child.as_str(), depth + 1)),
        );
        index += 1;
    }
    Ok(())
}
