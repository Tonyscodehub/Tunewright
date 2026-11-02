//! Vector icons drawn with the painter, so they render identically on every machine
//! without relying on emoji or symbol fonts.

use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Previous,
    Next,
    Play,
    Pause,
    Speaker,
    Muted,
    RepeatAll,
    RepeatOne,
    Shuffle,
    Power,
}

/// A button with an icon and optional text label, with the usual hover/selected styling.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, size: f32, selected: bool) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = (!label.is_empty())
        .then(|| ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER));
    let pad = vec2(8.0, 6.0);
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 6.0);
    let desired = vec2(size + text_w, size) + pad * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let fill = if selected { ui.visuals().selection.bg_fill } else { visuals.weak_bg_fill };
        ui.painter().rect(rect, visuals.corner_radius, fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let colour = if selected { ui.visuals().selection.stroke.color } else { visuals.fg_stroke.color };
        let icon_rect = Rect::from_min_size(rect.min + pad, vec2(size, size));
        paint(ui.painter(), icon_rect, icon, colour);
        if let Some(g) = galley {
            let pos = pos2(icon_rect.right() + 6.0, rect.center().y - g.size().y / 2.0);
            ui.painter().galley(pos, g, colour);
        }
    }
    response
}

/// A small inline icon with no button behind it.
pub fn small(ui: &mut Ui, icon: Icon, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    paint(ui.painter(), rect, icon, colour);
}

pub fn dot(ui: &mut Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
}

fn tri(p: &Painter, pts: [Pos2; 3], c: Color32) {
    p.add(Shape::convex_polygon(pts.to_vec(), c, Stroke::NONE));
}

fn arrow_head(p: &Painter, tip: Pos2, dir: egui::Vec2, s: f32, c: Color32) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    tri(p, [tip, tip - d * s + n * s * 0.6, tip - d * s - n * s * 0.6], c);
}

pub fn paint(p: &Painter, r: Rect, icon: Icon, c: Color32) {
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let at = |fx: f32, fy: f32| pos2(x0 + fx * w, y0 + fy * h);
    let line = Stroke::new((w / 11.0).max(1.5), c);
    match icon {
        Icon::Play => tri(p, [at(0.25, 0.15), at(0.85, 0.5), at(0.25, 0.85)], c),
        Icon::Pause => {
            p.rect_filled(Rect::from_min_max(at(0.22, 0.15), at(0.42, 0.85)), 1.0, c);
            p.rect_filled(Rect::from_min_max(at(0.58, 0.15), at(0.78, 0.85)), 1.0, c);
        }
        Icon::Next => {
            tri(p, [at(0.15, 0.2), at(0.5, 0.5), at(0.15, 0.8)], c);
            tri(p, [at(0.45, 0.2), at(0.8, 0.5), at(0.45, 0.8)], c);
            p.rect_filled(Rect::from_min_max(at(0.8, 0.2), at(0.88, 0.8)), 0.0, c);
        }
        Icon::Previous => {
            tri(p, [at(0.85, 0.2), at(0.5, 0.5), at(0.85, 0.8)], c);
            tri(p, [at(0.55, 0.2), at(0.2, 0.5), at(0.55, 0.8)], c);
            p.rect_filled(Rect::from_min_max(at(0.12, 0.2), at(0.2, 0.8)), 0.0, c);
        }
        Icon::Speaker | Icon::Muted => {
            p.add(Shape::convex_polygon(
                vec![at(0.1, 0.38), at(0.28, 0.38), at(0.5, 0.18), at(0.5, 0.82), at(0.28, 0.62), at(0.1, 0.62)],
                c,
                Stroke::NONE,
            ));
            if icon == Icon::Muted {
                p.line_segment([at(0.62, 0.35), at(0.9, 0.65)], line);
                p.line_segment([at(0.9, 0.35), at(0.62, 0.65)], line);
            } else {
                for rad in [0.18f32, 0.32] {
                    let centre = at(0.5, 0.5);
                    let pts: Vec<Pos2> = (-4..=4)
                        .map(|i| {
                            let a = i as f32 * 0.16;
                            centre + vec2(a.cos(), a.sin()) * rad * w
                        })
                        .collect();
                    p.add(Shape::line(pts, line));
                }
            }
        }
        Icon::RepeatAll | Icon::RepeatOne => {
            // Top run left to right, bottom run right to left, joined at the sides.
            p.add(Shape::line(vec![at(0.12, 0.6), at(0.12, 0.28), at(0.78, 0.28)], line));
            arrow_head(p, at(0.92, 0.28), vec2(1.0, 0.0), w * 0.18, c);
            p.add(Shape::line(vec![at(0.88, 0.4), at(0.88, 0.72), at(0.22, 0.72)], line));
            arrow_head(p, at(0.08, 0.72), vec2(-1.0, 0.0), w * 0.18, c);
            if icon == Icon::RepeatOne {
                p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "1", egui::FontId::proportional(h * 0.5), c);
            }
        }
        Icon::Shuffle => {
            p.add(Shape::line(vec![at(0.08, 0.28), at(0.3, 0.28), at(0.62, 0.72), at(0.78, 0.72)], line));
            arrow_head(p, at(0.94, 0.72), vec2(1.0, 0.0), w * 0.18, c);
            p.add(Shape::line(vec![at(0.08, 0.72), at(0.3, 0.72), at(0.62, 0.28), at(0.78, 0.28)], line));
            arrow_head(p, at(0.94, 0.28), vec2(1.0, 0.0), w * 0.18, c);
        }
        Icon::Power => {
            let centre = at(0.5, 0.55);
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + 0.6 + i as f32 * (2.0 * std::f32::consts::PI - 1.2) / 20.0;
                    centre + vec2(a.cos(), a.sin()) * 0.36 * w
                })
                .collect();
            p.add(Shape::line(pts, line));
            p.line_segment([at(0.5, 0.08), at(0.5, 0.5)], line);
        }
    }
}
