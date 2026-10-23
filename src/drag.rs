use crate::{
    document::{Document, MovePlacement, NodeId, Side},
    layout::{Layout, Point, Rect},
};

pub(crate) struct DropPreview {
    pub target: NodeId,
    pub placement: MovePlacement,
    pub rejection: Option<String>,
}

impl DropPreview {
    pub fn label(&self, doc: &Document) -> &str {
        if let Some(reason) = &self.rejection {
            return reason;
        }
        match &self.placement {
            MovePlacement::Child(parent) if doc.nodes[parent].folded => {
                "Move inside topic (unfolds branch)"
            }
            MovePlacement::Child(_) => "Move inside topic",
            MovePlacement::Before(_) => "Insert before topic",
            MovePlacement::After(_) => "Insert after topic",
            MovePlacement::RootSide(Side::Left) => "Move branch to left",
            MovePlacement::RootSide(Side::Right) => "Move branch to right",
        }
    }
}

pub(crate) struct DragSession {
    pub source: NodeId,
    pub revision: u64,
    pub title: String,
    pub preview: Option<DropPreview>,
    top_level: bool,
}

impl DragSession {
    pub fn new(doc: &Document, source: NodeId, revision: u64) -> Self {
        let mut characters = doc.nodes[&source].text.chars();
        let mut title: String = characters
            .by_ref()
            .take(48)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        if characters.next().is_some() {
            title.push_str("...");
        }
        let top_level = doc.parent(&source) == Some(doc.root.as_str());
        Self {
            source,
            revision,
            title,
            preview: None,
            top_level,
        }
    }

    pub fn update(&mut self, doc: &Document, layout: &Layout, pointer: Option<Point>, snap: f32) {
        let candidate = pointer.and_then(|pointer| self.destination(doc, layout, pointer, snap));
        if self
            .preview
            .as_ref()
            .map(|preview| (&preview.target, &preview.placement))
            == candidate
                .as_ref()
                .map(|(target, placement)| (target, placement))
        {
            return;
        }
        self.preview = candidate.map(|(target, placement)| {
            let rejection = doc
                .plan_move(&self.source, &placement)
                .err()
                .map(|error| error.to_string());
            DropPreview {
                target,
                placement,
                rejection,
            }
        });
    }

    fn destination(
        &self,
        doc: &Document,
        layout: &Layout,
        pointer: Point,
        snap: f32,
    ) -> Option<(NodeId, MovePlacement)> {
        let distance = |rect: &Rect| {
            let x = (pointer.x - pointer.x.clamp(rect.min.x, rect.max.x)).abs();
            let y = (pointer.y - pointer.y.clamp(rect.min.y, rect.max.y)).abs();
            x * x + y * y
        };
        if let Some((id, rect)) = layout
            .nodes
            .iter()
            .filter(|(_, rect)| distance(rect) <= snap * snap)
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
        {
            let placement = if id == &doc.root {
                MovePlacement::RootSide(if pointer.x < rect.center().x {
                    Side::Left
                } else {
                    Side::Right
                })
            } else {
                let fraction = (pointer.y - rect.min.y) / (rect.max.y - rect.min.y);
                if fraction < 0.25 {
                    MovePlacement::Before(id.clone())
                } else if fraction > 0.75 {
                    MovePlacement::After(id.clone())
                } else {
                    MovePlacement::Child(id.clone())
                }
            };
            return Some((id.clone(), placement));
        }
        if self.top_level {
            let root = layout.nodes[&doc.root];
            let side = if pointer.x < root.center().x {
                Side::Left
            } else {
                Side::Right
            };
            if side != doc.nodes[&self.source].side && (pointer.x - root.center().x).abs() > 24.0 {
                return Some((doc.root.clone(), MovePlacement::RootSide(side)));
            }
        }
        None
    }
}
