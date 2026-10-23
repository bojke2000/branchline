use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;
use uuid::Uuid;

pub type NodeId = String;
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_NODES: usize = 100_000;
pub const MAX_DEPTH: usize = 256;
pub const MAX_FIELD_BYTES: usize = 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;
const HISTORY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Side {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: NodeId,
    pub text: String,
    pub notes: String,
    pub link: String,
    pub children: Vec<NodeId>,
    pub side: Side,
    pub folded: bool,
}

impl Node {
    fn new(text: &str, side: Side) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            text: text.into(),
            notes: String::new(),
            link: String::new(),
            children: Vec::new(),
            side,
            folded: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct DocumentError(pub String);

impl Document {
    pub fn new(text: &str) -> Self {
        let node = Node::new(text, Side::Right);
        Self {
            version: 1,
            root: node.id.clone(),
            nodes: BTreeMap::from([(node.id.clone(), node)]),
        }
    }

    pub fn parent(&self, id: &str) -> Option<&str> {
        self.nodes
            .values()
            .find(|node| node.children.iter().any(|child| child == id))
            .map(|node| node.id.as_str())
    }

    pub fn visible_ancestor(&self, id: &str) -> Option<NodeId> {
        self.nodes.get(id)?;
        let mut visible = id;
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            if self.nodes[parent].folded {
                visible = parent;
            }
            current = parent;
        }
        Some(visible.to_owned())
    }

    pub fn validate(&self) -> Result<(), DocumentError> {
        let fail = |message: &str| Err(DocumentError(message.into()));
        if self.version != 1 {
            return fail("Unsupported document version");
        }
        if self.nodes.is_empty() || self.nodes.len() > MAX_NODES {
            return fail("Document must contain 1..100,000 nodes");
        }
        let mut seen = BTreeSet::new();
        let mut pending = vec![(self.root.as_str(), 1)];
        let mut text_bytes = 0usize;
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                return fail("Tree exceeds maximum depth of 256");
            }
            if !seen.insert(id) {
                return fail("Tree contains a cycle or a node with multiple parents");
            }
            let node = self
                .nodes
                .get(id)
                .ok_or_else(|| DocumentError(format!("Missing node: {id}")))?;
            if node.id != id || id.is_empty() || id.len() > MAX_FIELD_BYTES {
                return fail("Invalid node identity");
            }
            for field in [&node.text, &node.notes, &node.link] {
                if field.len() > MAX_FIELD_BYTES {
                    return fail("Text field exceeds 1 MiB");
                }
                text_bytes += field.len();
            }
            if text_bytes > MAX_TEXT_BYTES {
                return fail("Decoded text exceeds 64 MiB");
            }
            if node.children.len() > MAX_NODES {
                return fail("Too many children");
            }
            pending.extend(
                node.children
                    .iter()
                    .map(|child| (child.as_str(), depth + 1)),
            );
        }
        if seen.len() != self.nodes.len() {
            return fail("Document contains disconnected nodes");
        }
        Ok(())
    }

    pub fn reveal(&mut self, id: &str) {
        let mut current = id.to_owned();
        while let Some(parent) = self.parent(&current).map(str::to_owned) {
            self.nodes
                .get_mut(&parent)
                .expect("validated parent")
                .folded = false;
            current = parent;
        }
    }

    fn node_mut(&mut self, id: &str) -> Result<&mut Node, DocumentError> {
        self.nodes
            .get_mut(id)
            .ok_or_else(|| DocumentError(format!("Unknown node: {id}")))
    }

    fn parent_owned(&self, id: &str) -> Result<String, DocumentError> {
        self.parent(id)
            .map(str::to_owned)
            .ok_or_else(|| DocumentError("This action requires a non-root node".into()))
    }

    fn apply(&mut self, command: Command) -> Result<Option<NodeId>, DocumentError> {
        match command {
            Command::AddChild { parent, text } => {
                let parent_node = self
                    .nodes
                    .get(&parent)
                    .ok_or_else(|| DocumentError("Unknown parent".into()))?;
                let side = if parent == self.root {
                    if parent_node.children.len() % 2 == 1 {
                        Side::Left
                    } else {
                        Side::Right
                    }
                } else {
                    parent_node.side
                };
                let node = Node::new(&text, side);
                let id = node.id.clone();
                self.node_mut(&parent)?.children.push(id.clone());
                self.node_mut(&parent)?.folded = false;
                self.nodes.insert(id.clone(), node);
                Ok(Some(id))
            }
            Command::AddSibling { sibling, text } => {
                let parent = self.parent_owned(&sibling)?;
                let side = self.nodes[&sibling].side;
                let node = Node::new(&text, side);
                let id = node.id.clone();
                let children = &mut self.node_mut(&parent)?.children;
                let index = children
                    .iter()
                    .position(|child| child == &sibling)
                    .expect("parent membership");
                children.insert(index + 1, id.clone());
                self.nodes.insert(id.clone(), node);
                Ok(Some(id))
            }
            Command::SetText { id, text } => {
                self.node_mut(&id)?.text = text;
                Ok(Some(id))
            }
            Command::SetNotes { id, notes } => {
                self.node_mut(&id)?.notes = notes;
                Ok(Some(id))
            }
            Command::SetLink { id, link } => {
                self.node_mut(&id)?.link = link;
                Ok(Some(id))
            }
            Command::ToggleFold { id } => {
                let node = self.node_mut(&id)?;
                node.folded = !node.folded;
                Ok(Some(id))
            }
            Command::Reveal { id } => {
                if !self.nodes.contains_key(&id) {
                    return Err(DocumentError("Unknown node".into()));
                }
                self.reveal(&id);
                Ok(Some(id))
            }
            Command::Delete { id } => {
                let parent = self.parent_owned(&id)?;
                self.node_mut(&parent)?
                    .children
                    .retain(|child| child != &id);
                let mut pending = vec![id];
                while let Some(id) = pending.pop() {
                    let node = self
                        .nodes
                        .remove(&id)
                        .ok_or_else(|| DocumentError("Unknown node".into()))?;
                    pending.extend(node.children);
                }
                Ok(Some(parent))
            }
            Command::MoveSibling { id, delta } => {
                let parent = self.parent_owned(&id)?;
                let children = &mut self.node_mut(&parent)?.children;
                let index = children
                    .iter()
                    .position(|child| child == &id)
                    .expect("parent membership");
                let target = index
                    .checked_add_signed(delta)
                    .filter(|index| *index < children.len());
                if let Some(target) = target {
                    children.swap(index, target);
                }
                Ok(Some(id))
            }
            Command::Indent { id } => {
                let parent = self.parent_owned(&id)?;
                let children = &self.nodes[&parent].children;
                let index = children
                    .iter()
                    .position(|child| child == &id)
                    .expect("parent membership");
                if index == 0 {
                    return Err(DocumentError("No preceding sibling to indent under".into()));
                }
                let destination = children[index - 1].clone();
                self.node_mut(&parent)?.children.remove(index);
                self.node_mut(&destination)?.children.push(id.clone());
                self.node_mut(&destination)?.folded = false;
                self.set_subtree_side(&id, self.nodes[&destination].side)?;
                Ok(Some(id))
            }
            Command::Outdent { id } => {
                let parent = self.parent_owned(&id)?;
                let grandparent = self.parent_owned(&parent)?;
                self.node_mut(&parent)?
                    .children
                    .retain(|child| child != &id);
                let children = &mut self.node_mut(&grandparent)?.children;
                let index = children
                    .iter()
                    .position(|child| child == &parent)
                    .expect("parent membership");
                children.insert(index + 1, id.clone());
                self.set_subtree_side(&id, self.nodes[&parent].side)?;
                Ok(Some(id))
            }
        }
    }

    fn set_subtree_side(&mut self, id: &str, side: Side) -> Result<(), DocumentError> {
        let mut pending = vec![id.to_owned()];
        while let Some(id) = pending.pop() {
            let node = self.node_mut(&id)?;
            node.side = side;
            pending.extend(node.children.iter().cloned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    AddChild { parent: NodeId, text: String },
    AddSibling { sibling: NodeId, text: String },
    SetText { id: NodeId, text: String },
    SetNotes { id: NodeId, notes: String },
    SetLink { id: NodeId, link: String },
    Delete { id: NodeId },
    ToggleFold { id: NodeId },
    Reveal { id: NodeId },
    MoveSibling { id: NodeId, delta: isize },
    Indent { id: NodeId },
    Outdent { id: NodeId },
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<(Document, usize)>,
    redo: VecDeque<(Document, usize)>,
    pub evicted: bool,
    pub generation: u64,
}

impl History {
    pub fn execute(
        &mut self,
        doc: &mut Document,
        command: Command,
    ) -> Result<Option<NodeId>, DocumentError> {
        doc.validate()?;
        let mut candidate = doc.clone();
        let selected = candidate.apply(command)?;
        candidate.validate()?;
        if candidate != *doc {
            self.redo.clear();
            Self::push(&mut self.undo, doc.clone(), &mut self.evicted);
            *doc = candidate;
            self.generation = self.generation.wrapping_add(1);
        }
        Ok(selected)
    }

    fn push(stack: &mut VecDeque<(Document, usize)>, doc: Document, evicted: &mut bool) {
        let bytes = doc
            .nodes
            .values()
            .map(|node| {
                std::mem::size_of::<Node>()
                    + node.id.len()
                    + node.text.len()
                    + node.notes.len()
                    + node.link.len()
                    + node
                        .children
                        .iter()
                        .map(|id| id.len() + std::mem::size_of::<String>())
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>();
        while stack.iter().map(|(_, bytes)| bytes).sum::<usize>() + bytes > HISTORY_BYTES
            && !stack.is_empty()
        {
            stack.pop_front();
            *evicted = true;
        }
        if bytes <= HISTORY_BYTES {
            stack.push_back((doc, bytes));
        } else {
            *evicted = true;
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some((previous, _)) = self.undo.pop_back() {
            Self::push(
                &mut self.redo,
                std::mem::replace(doc, previous),
                &mut self.evicted,
            );
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some((next, _)) = self.redo.pop_back() {
            Self::push(
                &mut self.undo,
                std::mem::replace(doc, next),
                &mut self.evicted,
            );
            true
        } else {
            false
        }
    }
}
