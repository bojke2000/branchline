use crate::document::{Document, NodeId, Side};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn center(self) -> Point {
        Point {
            x: (self.min.x + self.max.x) * 0.5,
            y: (self.min.y + self.max.y) * 0.5,
        }
    }
    pub fn intersects(self, other: Self) -> bool {
        self.min.x < other.max.x
            && self.max.x > other.min.x
            && self.min.y < other.max.y
            && self.max.y > other.min.y
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub nodes: BTreeMap<NodeId, Rect>,
    pub edges: Vec<(NodeId, NodeId)>,
    pub bounds: Rect,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct LayoutError(String);

pub fn layout(doc: &Document, sizes: &BTreeMap<NodeId, Size>) -> Result<Layout, LayoutError> {
    doc.validate().map_err(|e| LayoutError(e.to_string()))?;
    let size = |id: &str| {
        sizes.get(id).copied().unwrap_or(Size {
            width: 140.0,
            height: 48.0,
        })
    };
    for id in doc.nodes.keys() {
        let size = size(id);
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return Err(LayoutError(
                "Node dimensions must be finite and positive".into(),
            ));
        }
    }
    let mut order = Vec::new();
    let mut pending = vec![doc.root.as_str()];
    while let Some(id) = pending.pop() {
        order.push(id);
        let node = &doc.nodes[id];
        if !node.folded {
            pending.extend(node.children.iter().rev().map(String::as_str));
        }
    }
    let mut heights = BTreeMap::new();
    for id in order.into_iter().rev() {
        let node = &doc.nodes[id];
        let children_height = if node.folded {
            0.0
        } else {
            node.children
                .iter()
                .map(|id| heights[id.as_str()] + 20.0)
                .sum::<f32>()
                - 20.0
        };
        heights.insert(id, size(id).height.max(children_height));
    }
    let root_size = size(&doc.root);
    let root_rect = Rect {
        min: Point {
            x: -root_size.width / 2.0,
            y: -root_size.height / 2.0,
        },
        max: Point {
            x: root_size.width / 2.0,
            y: root_size.height / 2.0,
        },
    };
    let mut result = Layout {
        nodes: BTreeMap::from([(doc.root.clone(), root_rect)]),
        edges: Vec::new(),
        bounds: root_rect,
    };
    let mut pending = vec![(doc.root.as_str(), Side::Right)];
    while let Some((id, inherited_side)) = pending.pop() {
        let node = &doc.nodes[id];
        if node.folded {
            continue;
        }
        let parent = result.nodes[id];
        let groups: Vec<(Side, Vec<&str>)> = if id == doc.root {
            [Side::Left, Side::Right]
                .into_iter()
                .map(|side| {
                    (
                        side,
                        node.children
                            .iter()
                            .filter(|id| doc.nodes[*id].side == side)
                            .map(String::as_str)
                            .collect(),
                    )
                })
                .collect()
        } else {
            vec![(
                inherited_side,
                node.children.iter().map(String::as_str).collect(),
            )]
        };
        for (side, children) in groups {
            let height = children.iter().map(|id| heights[id] + 20.0).sum::<f32>() - 20.0;
            let mut top = parent.center().y - height / 2.0;
            for child in children {
                let child_size = size(child);
                let x = if side == Side::Right {
                    parent.max.x + 70.0
                } else {
                    parent.min.x - 70.0 - child_size.width
                };
                let y = top + heights[child] / 2.0 - child_size.height / 2.0;
                let rect = Rect {
                    min: Point { x, y },
                    max: Point {
                        x: x + child_size.width,
                        y: y + child_size.height,
                    },
                };
                result.nodes.insert(child.into(), rect);
                result.edges.push((id.into(), child.into()));
                result.bounds.min.x = result.bounds.min.x.min(rect.min.x);
                result.bounds.min.y = result.bounds.min.y.min(rect.min.y);
                result.bounds.max.x = result.bounds.max.x.max(rect.max.x);
                result.bounds.max.y = result.bounds.max.y.max(rect.max.y);
                top += heights[child] + 20.0;
                pending.push((child, side));
            }
        }
    }
    Ok(result)
}
