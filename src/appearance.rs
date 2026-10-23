use crate::commands::AppCommand;
use eframe::egui::{self, Color32, FontId, Pos2, Sense, Stroke, Vec2};

const BRANCH_COLORS: [Color32; 8] = [
    Color32::from_rgb(38, 105, 220),
    Color32::from_rgb(214, 60, 75),
    Color32::from_rgb(0, 144, 125),
    Color32::from_rgb(155, 74, 198),
    Color32::from_rgb(222, 128, 29),
    Color32::from_rgb(38, 157, 184),
    Color32::from_rgb(110, 146, 35),
    Color32::from_rgb(196, 70, 144),
];

pub fn branch_color(id: &str) -> Color32 {
    let hash = id.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
    });
    BRANCH_COLORS[(hash % BRANCH_COLORS.len() as u64) as usize]
}

pub fn logo(ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(112.0, 40.0), Sense::hover());
    response.on_hover_text("BMM — a clearer space for your ideas");
    let p = ui.painter();
    let emblem = egui::Rect::from_min_size(rect.min, Vec2::splat(38.0));
    p.rect_filled(emblem, 11.0, Color32::from_rgb(22, 43, 79));
    let start = emblem.min + Vec2::new(11.0, 19.0);
    for (y, color) in [
        (10.0, Color32::from_rgb(75, 207, 225)),
        (19.0, Color32::WHITE),
        (28.0, Color32::from_rgb(72, 218, 173)),
    ] {
        let end = emblem.min + Vec2::new(28.0, y);
        p.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [
                start,
                start + Vec2::new(9.0, 0.0),
                end - Vec2::new(8.0, 0.0),
                end,
            ],
            false,
            Color32::TRANSPARENT,
            Stroke::new(2.2, color),
        ));
        p.circle_filled(end, 2.6, color);
    }
    p.circle_filled(start, 3.3, Color32::WHITE);
    p.text(
        rect.min + Vec2::new(47.0, 19.0),
        egui::Align2::LEFT_CENTER,
        "BMM",
        FontId::proportional(24.0),
        ui.visuals().strong_text_color(),
    );
}

pub fn toolbar_button(
    ui: &mut egui::Ui,
    command: AppCommand,
    enabled: bool,
    active: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let enabled = ui.is_enabled();
        let font = FontId::proportional(13.0);
        let galley = ui.painter().layout_no_wrap(
            command.toolbar_label().into(),
            font,
            ui.visuals().text_color(),
        );
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(galley.size().x + 44.0, 32.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, enabled, command.spec().label)
        });
        let visuals = ui.style().interact(&response);
        let fill = if active {
            ui.visuals().selection.bg_fill
        } else {
            visuals.bg_fill
        };
        if response.hovered()
            || response.has_focus()
            || response.is_pointer_button_down_on()
            || active
        {
            ui.painter().rect_filled(rect, 6.0, fill);
            ui.painter()
                .rect_stroke(rect, 6.0, visuals.bg_stroke, egui::StrokeKind::Inside);
        }
        let color = if enabled {
            visuals.fg_stroke.color
        } else {
            ui.visuals().weak_text_color()
        };
        icon(
            ui.painter(),
            command,
            rect.left_center() + Vec2::new(15.0, 0.0),
            color,
        );
        ui.painter().galley(
            Pos2::new(rect.left() + 29.0, rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
        let spec = command.spec();
        response.on_hover_text(if spec.shortcut.is_empty() {
            spec.label.into()
        } else {
            format!("{}  ·  {}", spec.label, spec.shortcut)
        })
    })
    .inner
}

fn icon(p: &egui::Painter, command: AppCommand, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let point = |x, y| center + Vec2::new(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    };
    let rectangle = |x, y, w, h| {
        p.rect_stroke(
            egui::Rect::from_min_size(point(x, y), Vec2::new(w, h)),
            1.5,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    use AppCommand::*;
    match command {
        New => {
            rectangle(-6.0, -8.0, 12.0, 16.0);
            line((-3.0, 0.0), (3.0, 0.0));
            line((0.0, -3.0), (0.0, 3.0));
        }
        Open => {
            p.add(egui::Shape::closed_line(
                vec![
                    point(-8.0, -5.0),
                    point(-2.0, -5.0),
                    point(0.0, -2.0),
                    point(8.0, -2.0),
                    point(6.0, 7.0),
                    point(-8.0, 7.0),
                ],
                stroke,
            ));
        }
        Save => {
            rectangle(-7.0, -7.0, 14.0, 15.0);
            rectangle(-3.0, -7.0, 6.0, 5.0);
            rectangle(-4.0, 2.0, 8.0, 6.0);
        }
        Undo | Redo => {
            let sign = if command == Undo { 1.0 } else { -1.0 };
            p.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    point(5.0 * sign, 6.0),
                    point(10.0 * sign, -7.0),
                    point(-2.0 * sign, -8.0),
                    point(-6.0 * sign, -2.0),
                ],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
            line((-6.0 * sign, -2.0), (-6.0 * sign, -8.0));
            line((-6.0 * sign, -2.0), (0.0, -2.0));
        }
        AddChild | AddSibling => {
            rectangle(-7.0, -6.0, 7.0, 5.0);
            rectangle(1.0, 2.0, 7.0, 5.0);
            line((-3.5, -1.0), (-3.5, 4.5));
            line((-3.5, 4.5), (1.0, 4.5));
            if command == AddSibling {
                rectangle(1.0, -6.0, 7.0, 5.0);
            }
        }
        Edit => {
            line((-5.0, 5.0), (5.0, -5.0));
            line((-3.0, 7.0), (7.0, -3.0));
            line((5.0, -5.0), (7.0, -3.0));
            line((-5.0, 5.0), (-6.0, 8.0));
            line((-6.0, 8.0), (-3.0, 7.0));
        }
        Delete => {
            rectangle(-5.0, -3.0, 10.0, 11.0);
            line((-7.0, -5.0), (7.0, -5.0));
            line((-2.0, -7.0), (2.0, -7.0));
            line((-2.0, 0.0), (-2.0, 5.0));
            line((2.0, 0.0), (2.0, 5.0));
        }
        Search => {
            p.circle_stroke(point(-2.0, -2.0), 5.5, stroke);
            line((2.0, 2.0), (7.0, 7.0));
        }
        Palette | Notes => {
            rectangle(-7.0, -8.0, 14.0, 16.0);
            for y in [-4.0, 0.0, 4.0] {
                line((-3.0, y), (4.0, y));
            }
        }
        Fit => {
            for (x, y) in [(-7.0, -7.0), (7.0, -7.0), (-7.0, 7.0), (7.0, 7.0)] {
                line((x, y), (x * 0.4, y));
                line((x, y), (x, y * 0.4));
            }
        }
        Focus => {
            p.circle_stroke(center, 5.0, stroke);
            line((-9.0, 0.0), (9.0, 0.0));
            line((0.0, -9.0), (0.0, 9.0));
        }
        Theme => {
            p.circle_stroke(center, 4.0, stroke);
            for (x, y) in [(0.0, 8.0), (8.0, 0.0), (0.0, -8.0), (-8.0, 0.0)] {
                line((x * 0.75, y * 0.75), (x, y));
            }
        }
        _ => {
            p.circle_stroke(center, 7.0, stroke);
            line((-3.0, 0.0), (3.0, 0.0));
            line((0.0, -3.0), (0.0, 3.0));
        }
    }
}
