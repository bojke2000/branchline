use crate::{
    app::Editor,
    file_identity::FileIdentityState,
    layout::{Point, Rect, Size},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone, Copy)]
pub struct Viewport {
    pub pan: Point,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan: Point::default(),
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn to_screen(self, point: Point, center: Point) -> Point {
        Point {
            x: center.x + self.pan.x + point.x * self.zoom,
            y: center.y + self.pan.y + point.y * self.zoom,
        }
    }

    pub fn to_world(self, point: Point, center: Point) -> Point {
        Point {
            x: (point.x - center.x - self.pan.x) / self.zoom,
            y: (point.y - center.y - self.pan.y) / self.zoom,
        }
    }

    pub fn zoom_at(&mut self, pointer: Point, center: Point, factor: f32) {
        let world = self.to_world(pointer, center);
        self.zoom = (self.zoom * factor).clamp(0.05, 3.0);
        self.pan = Point {
            x: pointer.x - center.x - world.x * self.zoom,
            y: pointer.y - center.y - world.y * self.zoom,
        };
    }

    pub fn fit(&mut self, bounds: Rect, available: Size) {
        let width = (bounds.max.x - bounds.min.x).max(1.0);
        let height = (bounds.max.y - bounds.min.y).max(1.0);
        self.zoom = ((available.width - 80.0).max(1.0) / width)
            .min((available.height - 80.0).max(1.0) / height)
            .clamp(0.01, 1.2);
        self.focus(bounds.center());
    }

    pub fn focus(&mut self, point: Point) {
        self.pan = Point {
            x: -point.x * self.zoom,
            y: -point.y * self.zoom,
        };
    }
}

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub request_focus: bool,
}

pub struct MapSession {
    id: SessionId,
    pub editor: Editor,
    pub viewport: Viewport,
    pub search: SearchState,
    pub notes_open: bool,
    pub file_identity: FileIdentityState,
    pub(crate) pristine_blank: bool,
}

impl MapSession {
    pub fn estimated_bytes(&self) -> usize {
        self.editor.estimated_bytes() + std::mem::size_of::<Self>() + 4096
    }

    pub fn new_blank(id: SessionId) -> Self {
        Self {
            id,
            editor: Editor::new(),
            viewport: Viewport::default(),
            search: SearchState::default(),
            notes_open: false,
            file_identity: FileIdentityState::Unsaved,
            pristine_blank: true,
        }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn is_pristine_blank(&self) -> bool {
        self.pristine_blank
    }
}

pub struct RestoreFailure {
    pub path: Option<PathBuf>,
    pub reason: String,
}

pub enum SessionState {
    Map(Box<MapSession>),
    RestoreError(RestoreFailure),
}
