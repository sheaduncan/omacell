//! Paint a chart [`Scene`] with egui.

use egui::{Align2, FontId, Pos2, Rect, Ui, pos2};
use omacell_core::chart::{Op, Scene};

use crate::theme::hex_color;

/// Paint `scene` into `rect`.
pub fn paint(ui: &mut Ui, scene: &Scene, rect: Rect) {
    let sx = rect.width() / scene.width.max(1.0);
    let sy = rect.height() / scene.height.max(1.0);
    let painter = ui.painter_at(rect);
    for op in &scene.ops {
        match op {
            Op::FillRect { x, y, w, h, color } => {
                let r = Rect::from_min_size(
                    pos2(rect.left() + x * sx, rect.top() + y * sy),
                    egui::vec2(w * sx, h * sy),
                );
                painter.rect_filled(r, 0.0, hex_color(color));
            }
            Op::Polyline {
                points,
                color,
                width,
            } => {
                if points.len() < 2 {
                    continue;
                }
                for pair in points.windows(2) {
                    painter.line_segment(
                        [map(rect, sx, sy, pair[0]), map(rect, sx, sy, pair[1])],
                        (width.max(1.0), hex_color(color)),
                    );
                }
            }
            Op::Polygon { points, color } => {
                if points.len() < 3 {
                    continue;
                }
                let shape = points
                    .iter()
                    .map(|p| map(rect, sx, sy, *p))
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::convex_polygon(
                    shape,
                    hex_color(color),
                    egui::Stroke::NONE,
                ));
            }
            Op::Circle { x, y, r, color } => {
                painter.circle_filled(
                    map(rect, sx, sy, (*x, *y)),
                    *r * sx.min(sy),
                    hex_color(color),
                );
            }
            Op::Text {
                x,
                y,
                text,
                color,
                size,
            } => {
                painter.text(
                    map(rect, sx, sy, (*x, *y)),
                    Align2::LEFT_BOTTOM,
                    text,
                    FontId::proportional((size * sy).max(1.0)),
                    hex_color(color),
                );
            }
        }
    }
}

fn map(rect: Rect, sx: f32, sy: f32, p: (f32, f32)) -> Pos2 {
    pos2(rect.left() + p.0 * sx, rect.top() + p.1 * sy)
}
