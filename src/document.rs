use crate::outline::{OutlineForest, OutlineNode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
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
    fn new(text: impl Into<String>, side: Side) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovePlacement {
    Child(NodeId),
    Before(NodeId),
    After(NodeId),
    RootSide(Side),
}

pub struct MovePlan {
    source_parent: NodeId,
    destination_parent: NodeId,
    index: usize,
    side: Side,
}

pub struct DeletionPlan {
    pub roots: Vec<NodeId>,
    pub node_count: usize,
    pub root_retained: bool,
    removed: BTreeSet<NodeId>,
}

impl Document {
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.root.len()
            + self
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
                .sum::<usize>()
    }

    pub fn decoded_text_bytes(&self) -> usize {
        self.nodes
            .values()
            .map(|node| node.text.len() + node.notes.len() + node.link.len())
            .sum()
    }

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

    pub fn visible_nodes(&self) -> BTreeSet<&NodeId> {
        let mut visible = BTreeSet::new();
        let mut pending = vec![&self.root];
        while let Some(id) = pending.pop() {
            visible.insert(id);
            let node = &self.nodes[id];
            if !node.folded {
                pending.extend(&node.children);
            }
        }
        visible
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

    fn parent_index(&self) -> BTreeMap<&str, &str> {
        self.nodes
            .values()
            .flat_map(|node| {
                node.children
                    .iter()
                    .map(|child| (child.as_str(), node.id.as_str()))
            })
            .collect()
    }

    pub fn plan_move(
        &self,
        id: &str,
        placement: &MovePlacement,
    ) -> Result<MovePlan, DocumentError> {
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| DocumentError("Unknown source node".into()))?;
        if id == self.root {
            return Err(DocumentError("The central idea cannot be moved".into()));
        }
        let parents = self.parent_index();
        let source_parent = *parents
            .get(id)
            .ok_or_else(|| DocumentError("Missing source parent".into()))?;
        let (destination_parent, relative, side) = match placement {
            MovePlacement::Child(parent) => {
                let parent_node = self
                    .nodes
                    .get(parent)
                    .ok_or_else(|| DocumentError("Unknown destination".into()))?;
                (
                    parent.as_str(),
                    None,
                    if parent == &self.root {
                        node.side
                    } else {
                        parent_node.side
                    },
                )
            }
            MovePlacement::RootSide(side) => (self.root.as_str(), None, *side),
            MovePlacement::Before(sibling) | MovePlacement::After(sibling) => {
                if sibling == id {
                    return Err(DocumentError(
                        "An idea cannot be dropped onto itself".into(),
                    ));
                }
                let parent = *parents.get(sibling.as_str()).ok_or_else(|| {
                    DocumentError("Destination must be a non-root sibling".into())
                })?;
                (
                    parent,
                    Some((
                        sibling.as_str(),
                        matches!(placement, MovePlacement::After(_)),
                    )),
                    self.nodes[sibling].side,
                )
            }
        };
        let mut ancestor = destination_parent;
        let mut depth = 1;
        loop {
            if ancestor == id {
                return Err(DocumentError(
                    "A branch cannot be moved into itself or its descendants".into(),
                ));
            }
            if depth > MAX_DEPTH {
                return Err(DocumentError(
                    "Destination exceeds maximum tree depth".into(),
                ));
            }
            match parents.get(ancestor) {
                Some(parent) => {
                    ancestor = parent;
                    depth += 1;
                }
                None => break,
            }
        }
        let mut pending = vec![(id, 1)];
        while let Some((current, height)) = pending.pop() {
            if depth + height > MAX_DEPTH {
                return Err(DocumentError(
                    "Move would exceed maximum tree depth of 256".into(),
                ));
            }
            pending.extend(
                self.nodes[current]
                    .children
                    .iter()
                    .map(|child| (child.as_str(), height + 1)),
            );
        }
        let children: Vec<_> = self.nodes[destination_parent]
            .children
            .iter()
            .filter(|child| child.as_str() != id)
            .collect();
        let index = if let Some((sibling, after)) = relative {
            children
                .iter()
                .position(|child| child.as_str() == sibling)
                .expect("validated sibling")
                + usize::from(after)
        } else if matches!(placement, MovePlacement::RootSide(side) if *side == node.side)
            && source_parent == self.root
        {
            self.nodes[source_parent]
                .children
                .iter()
                .position(|child| child == id)
                .expect("source membership")
        } else {
            children.len()
        };
        Ok(MovePlan {
            source_parent: source_parent.into(),
            destination_parent: destination_parent.into(),
            index,
            side,
        })
    }

    fn move_node(
        &mut self,
        id: NodeId,
        placement: MovePlacement,
    ) -> Result<Option<NodeId>, DocumentError> {
        let plan = self.plan_move(&id, &placement)?;
        self.node_mut(&plan.source_parent)?
            .children
            .retain(|child| child != &id);
        let destination = self.node_mut(&plan.destination_parent)?;
        destination.children.insert(plan.index, id.clone());
        destination.folded = false;
        self.set_subtree_side(&id, plan.side)?;
        Ok(Some(id))
    }

    pub fn plan_deletion(&self, ids: &[NodeId]) -> Result<DeletionPlan, DocumentError> {
        let mut selected: BTreeSet<_> = ids.iter().cloned().collect();
        if selected.iter().any(|id| !self.nodes.contains_key(id)) {
            return Err(DocumentError("Selection includes an unknown node".into()));
        }
        let root_retained = selected.remove(&self.root);
        if selected.is_empty() {
            return Err(DocumentError(
                "The central idea cannot be deleted; select another idea".into(),
            ));
        }
        let mut roots = Vec::new();
        let mut removed = BTreeSet::new();
        let mut pending = vec![(self.root.clone(), false)];
        while let Some((id, selected_ancestor)) = pending.pop() {
            let selected_here = selected.contains(&id);
            if selected_here && !selected_ancestor {
                roots.push(id.clone());
            }
            let remove = selected_ancestor || selected_here;
            pending.extend(
                self.nodes[&id]
                    .children
                    .iter()
                    .rev()
                    .map(|child| (child.clone(), remove)),
            );
            if remove {
                removed.insert(id);
            }
        }
        Ok(DeletionPlan {
            roots,
            node_count: removed.len(),
            root_retained,
            removed,
        })
    }

    fn delete_many(&mut self, ids: Vec<NodeId>) -> Result<Option<NodeId>, DocumentError> {
        let plan = self.plan_deletion(&ids)?;
        let parent = self.parent_owned(&plan.roots[0])?;
        for node in self.nodes.values_mut() {
            node.children.retain(|id| !plan.removed.contains(id));
        }
        for id in plan.removed {
            self.nodes.remove(&id);
        }
        Ok(Some(parent))
    }

    fn siblings_on_side(&self, id: &str, parent: &str) -> Vec<NodeId> {
        self.nodes[parent]
            .children
            .iter()
            .filter(|sibling| {
                parent != self.root || self.nodes[*sibling].side == self.nodes[id].side
            })
            .cloned()
            .collect()
    }

    fn apply(&mut self, command: Command) -> Result<Option<NodeId>, DocumentError> {
        match command {
            Command::PasteOutline { parent, forest } => self.paste_outline(parent, forest),
            Command::ReplaceWithOutline { forest } => {
                *self = Self::from_outline(forest)?;
                Ok(Some(self.root.clone()))
            }
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
                let node = Node::new(text, side);
                let id = node.id.clone();
                self.node_mut(&parent)?.children.push(id.clone());
                self.node_mut(&parent)?.folded = false;
                self.nodes.insert(id.clone(), node);
                Ok(Some(id))
            }
            Command::AddSibling { sibling, text } => {
                let parent = self.parent_owned(&sibling)?;
                let side = self.nodes[&sibling].side;
                let node = Node::new(text, side);
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
                Ok(None)
            }
            Command::SetLink { id, link } => {
                self.node_mut(&id)?.link = link;
                Ok(None)
            }
            Command::SetDetails { id, notes, link } => {
                let node = self.node_mut(&id)?;
                node.notes = notes;
                node.link = link;
                Ok(None)
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
            Command::Delete { id } => self.delete_many(vec![id]),
            Command::DeleteMany { ids } => self.delete_many(ids),
            Command::MoveNode { id, placement } => self.move_node(id, placement),
            Command::MoveSibling { id, delta } => {
                let parent = self.parent_owned(&id)?;
                let children = self.siblings_on_side(&id, &parent);
                let index = children
                    .iter()
                    .position(|child| child == &id)
                    .expect("parent membership");
                let target = index
                    .checked_add_signed(delta)
                    .filter(|index| *index < children.len());
                if let Some(target) = target
                    && target != index
                {
                    let placement = if target < index {
                        MovePlacement::Before(children[target].clone())
                    } else {
                        MovePlacement::After(children[target].clone())
                    };
                    return self.move_node(id, placement);
                }
                Ok(Some(id))
            }
            Command::Indent { id } => {
                let parent = self.parent_owned(&id)?;
                let children = self.siblings_on_side(&id, &parent);
                let index = children
                    .iter()
                    .position(|child| child == &id)
                    .expect("parent membership");
                if index == 0 {
                    return Err(DocumentError("No preceding sibling to indent under".into()));
                }
                let destination = children[index - 1].clone();
                self.move_node(id, MovePlacement::Child(destination))
            }
            Command::Outdent { id } => {
                let parent = self.parent_owned(&id)?;
                self.move_node(id, MovePlacement::After(parent))
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

    fn insert_outline_children(&mut self, parent: &str, roots: Vec<OutlineNode>) -> NodeId {
        let owner = &self.nodes[parent];
        let count = owner.children.len();
        let inherited = owner.side;
        let mut pending: Vec<_> = roots
            .into_iter()
            .enumerate()
            .rev()
            .map(|(index, node)| {
                let side = if parent == self.root {
                    if (count + index).is_multiple_of(2) {
                        Side::Right
                    } else {
                        Side::Left
                    }
                } else {
                    inherited
                };
                (parent.to_owned(), node, side)
            })
            .collect();
        let mut first = None;
        while let Some((parent, source, side)) = pending.pop() {
            let node = Node::new(source.label, side);
            let id = node.id.clone();
            first.get_or_insert_with(|| id.clone());
            self.nodes
                .get_mut(&parent)
                .expect("validated outline parent")
                .children
                .push(id.clone());
            self.nodes.insert(id.clone(), node);
            pending.extend(
                source
                    .children
                    .into_iter()
                    .rev()
                    .map(|child| (id.clone(), child, side)),
            );
        }
        first.expect("validated outline is nonempty")
    }

    fn from_outline(mut forest: OutlineForest) -> Result<Self, DocumentError> {
        let facts = forest.facts()?;
        let synthetic = usize::from(forest.roots.len() != 1);
        if facts.nodes + synthetic > MAX_NODES
            || facts.depth + synthetic > MAX_DEPTH
            || facts.text_bytes + synthetic * "Pasted ideas".len() > MAX_TEXT_BYTES
        {
            return Err(DocumentError(
                "Outline plus its synthetic root exceeds document capacity.".into(),
            ));
        }
        let source = if synthetic == 0 {
            forest.roots.pop().expect("validated single root")
        } else {
            OutlineNode {
                label: "Pasted ideas".into(),
                children: forest.roots,
            }
        };
        let root = Node::new(source.label, Side::Right);
        let mut document = Self {
            version: 1,
            root: root.id.clone(),
            nodes: BTreeMap::from([(root.id.clone(), root)]),
        };
        if !source.children.is_empty() {
            document.insert_outline_children(&document.root.clone(), source.children);
        }
        Ok(document)
    }

    fn paste_outline(
        &mut self,
        parent: NodeId,
        forest: OutlineForest,
    ) -> Result<Option<NodeId>, DocumentError> {
        let facts = forest.facts()?;
        if !self.nodes.contains_key(&parent) {
            return Err(DocumentError(
                "The paste destination no longer exists.".into(),
            ));
        }
        if self.nodes.len() + facts.nodes > MAX_NODES
            || self.decoded_text_bytes().saturating_add(facts.text_bytes) > MAX_TEXT_BYTES
        {
            return Err(DocumentError(
                "Pasted outline exceeds the destination's document capacity.".into(),
            ));
        }
        let mut pending = vec![(self.root.as_str(), 1)];
        let depth = loop {
            let (id, depth) = pending.pop().expect("validated destination is reachable");
            if id == parent {
                break depth;
            }
            pending.extend(
                self.nodes[id]
                    .children
                    .iter()
                    .map(|child| (child.as_str(), depth + 1)),
            );
        };
        if depth + facts.depth > MAX_DEPTH {
            return Err(DocumentError(
                "Pasted outline would exceed document depth 256.".into(),
            ));
        }
        let first = self.insert_outline_children(&parent, forest.roots);
        self.nodes
            .get_mut(&parent)
            .expect("validated paste destination")
            .folded = false;
        Ok(Some(first))
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    PasteOutline {
        parent: NodeId,
        forest: OutlineForest,
    },
    ReplaceWithOutline {
        forest: OutlineForest,
    },
    AddChild {
        parent: NodeId,
        text: String,
    },
    AddSibling {
        sibling: NodeId,
        text: String,
    },
    SetText {
        id: NodeId,
        text: String,
    },
    SetNotes {
        id: NodeId,
        notes: String,
    },
    SetLink {
        id: NodeId,
        link: String,
    },
    SetDetails {
        id: NodeId,
        notes: String,
        link: String,
    },
    Delete {
        id: NodeId,
    },
    DeleteMany {
        ids: Vec<NodeId>,
    },
    ToggleFold {
        id: NodeId,
    },
    Reveal {
        id: NodeId,
    },
    MoveSibling {
        id: NodeId,
        delta: isize,
    },
    Indent {
        id: NodeId,
    },
    Outdent {
        id: NodeId,
    },
    MoveNode {
        id: NodeId,
        placement: MovePlacement,
    },
}

#[derive(Clone)]
struct HistoryEntry {
    document: Arc<Document>,
    bytes: usize,
    serial: u64,
    stamp: u64,
}

#[derive(Clone, Default)]
pub struct History {
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    pub evicted: bool,
    pub generation: u64,
    next_serial: u64,
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
            self.push(doc.clone(), false);
            *doc = candidate;
            self.generation = self.generation.wrapping_add(1);
        }
        Ok(selected)
    }

    fn push(&mut self, doc: Document, redo: bool) {
        let bytes = doc.estimated_bytes();
        let stack = if redo { &mut self.redo } else { &mut self.undo };
        while stack.iter().map(|entry| entry.bytes).sum::<usize>() + bytes > HISTORY_BYTES
            && !stack.is_empty()
        {
            stack.pop_front();
            self.evicted = true;
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        if bytes <= HISTORY_BYTES {
            stack.push_back(HistoryEntry {
                document: Arc::new(doc),
                bytes,
                serial,
                stamp: serial,
            });
        } else {
            self.evicted = true;
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(previous) = self.undo.pop_back() {
            let previous = Arc::unwrap_or_clone(previous.document);
            self.push(std::mem::replace(doc, previous), true);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(next) = self.redo.pop_back() {
            let next = Arc::unwrap_or_clone(next.document);
            self.push(std::mem::replace(doc, next), false);
            true
        } else {
            false
        }
    }

    pub fn retained_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(|entry| entry.bytes)
            .sum()
    }

    pub(crate) fn next_serial(&self) -> u64 {
        self.next_serial
    }

    pub(crate) fn stamp_since(&mut self, serial: u64, stamp: u64) {
        for entry in self.undo.iter_mut().chain(&mut self.redo) {
            if entry.serial >= serial {
                entry.stamp = stamp + entry.serial - serial;
            }
        }
    }

    pub fn oldest_retained(&self) -> Option<(u64, usize)> {
        self.undo
            .front()
            .into_iter()
            .chain(self.redo.front())
            .map(|entry| (entry.stamp, entry.bytes))
            .min_by_key(|entry| entry.0)
    }

    pub fn evict_oldest(&mut self, stamp: u64) -> bool {
        let stack = if self.undo.front().is_some_and(|entry| entry.stamp == stamp) {
            &mut self.undo
        } else if self.redo.front().is_some_and(|entry| entry.stamp == stamp) {
            &mut self.redo
        } else {
            return false;
        };
        stack.pop_front();
        self.evicted = true;
        true
    }
}
