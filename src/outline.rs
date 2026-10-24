use crate::document::{DocumentError, MAX_DEPTH, MAX_FIELD_BYTES, MAX_NODES, MAX_TEXT_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineNode {
    pub label: String,
    pub children: Vec<OutlineNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineForest {
    pub roots: Vec<OutlineNode>,
}

pub(crate) struct OutlineFacts {
    pub nodes: usize,
    pub depth: usize,
    pub text_bytes: usize,
}

impl OutlineForest {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.facts().map(|_| ())
    }

    pub(crate) fn facts(&self) -> Result<OutlineFacts, DocumentError> {
        if self.roots.is_empty() || self.roots.len() > MAX_NODES {
            return Err(DocumentError(
                "An outline must contain 1..100,000 nodes.".into(),
            ));
        }
        let mut facts = OutlineFacts {
            nodes: 0,
            depth: 0,
            text_bytes: 0,
        };
        let mut pending: Vec<_> = self.roots.iter().map(|node| (node, 1)).collect();
        let mut scheduled = self.roots.len();
        while let Some((node, depth)) = pending.pop() {
            if depth > MAX_DEPTH || node.label.len() > MAX_FIELD_BYTES {
                return Err(DocumentError(
                    "Outline depth exceeds 256 or a label exceeds 1 MiB.".into(),
                ));
            }
            scheduled = scheduled
                .checked_add(node.children.len())
                .filter(|count| *count <= MAX_NODES)
                .ok_or_else(|| DocumentError("Outline exceeds 100,000 nodes.".into()))?;
            facts.nodes += 1;
            facts.depth = facts.depth.max(depth);
            facts.text_bytes = facts
                .text_bytes
                .checked_add(node.label.len())
                .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
                .ok_or_else(|| DocumentError("Outline decoded text exceeds 64 MiB.".into()))?;
            pending.extend(node.children.iter().map(|child| (child, depth + 1)));
        }
        Ok(facts)
    }
}
