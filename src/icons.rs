//! Little pixel icons drawn with the painter, in the style of JAMP's skin icons.

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::model::{Kind, Status};
use crate::theme::*;

/// The header's filter icons (`None` = all games). `rect` is about 30×30.
pub fn filter_icon(p: &Painter, ic: Rect, which: Option<Status>) {
    let c = ic.center();
    let st = Stroke::new(1.5f32, INK);
    match which {
        None => {
            // a stack of cartridges
            for (i, col) in [(2.0, TEAL), (0.0, MAGENTA), (-2.0, YELLOW)].iter().enumerate() {
                let off = Vec2::new(-6.0 + i as f32 * 6.0, col.0);
                let r = Rect::from_center_size(c + off, Vec2::new(14.0, 20.0));
                p.rect_filled(r, 1.5, Color32::from_gray(200));
                p.rect_filled(Rect::from_min_size(r.min + Vec2::new(2.0, 3.0), Vec2::new(10.0, 9.0)), 1.0, col.1);
                p.rect_stroke(r, 1.5, Stroke::new(1.0f32, INK), StrokeKind::Inside);
            }
        }
        Some(Status::Playing) => {
            // a gamepad
            let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(28.0, 15.0));
            p.rect_filled(body, 6.0, WHITE);
            p.circle_filled(body.left_bottom() + Vec2::new(6.0, -1.0), 5.0, WHITE);
            p.circle_filled(body.right_bottom() + Vec2::new(-6.0, -1.0), 5.0, WHITE);
            let d = body.left_center() + Vec2::new(7.0, 0.0);
            p.rect_filled(Rect::from_center_size(d, Vec2::new(8.0, 2.5)), 0.0, INK);
            p.rect_filled(Rect::from_center_size(d, Vec2::new(2.5, 8.0)), 0.0, INK);
            let b = body.right_center() + Vec2::new(-7.0, 0.0);
            p.circle_filled(b + Vec2::new(2.5, -2.0), 2.0, MAGENTA);
            p.circle_filled(b + Vec2::new(-2.0, 2.0), 2.0, TEAL);
        }
        Some(Status::Backlog) => {
            // an hourglass
            let t = ic.top() + 3.0;
            let b = ic.bottom() - 3.0;
            p.rect_filled(Rect::from_min_max(Pos2::new(c.x - 10.0, t), Pos2::new(c.x + 10.0, t + 3.0)), 1.0, YELLOW);
            p.rect_filled(Rect::from_min_max(Pos2::new(c.x - 10.0, b - 3.0), Pos2::new(c.x + 10.0, b)), 1.0, YELLOW);
            let glass = vec![
                Pos2::new(c.x - 8.0, t + 3.0),
                Pos2::new(c.x + 8.0, t + 3.0),
                Pos2::new(c.x + 1.5, c.y),
                Pos2::new(c.x + 8.0, b - 3.0),
                Pos2::new(c.x - 8.0, b - 3.0),
                Pos2::new(c.x - 1.5, c.y),
            ];
            p.add(egui::Shape::convex_polygon(vec![glass[0], glass[1], glass[2], glass[5]], WHITE, Stroke::NONE));
            p.add(egui::Shape::convex_polygon(vec![glass[5], glass[2], glass[3], glass[4]], WHITE, Stroke::NONE));
            p.add(egui::Shape::convex_polygon(
                vec![Pos2::new(c.x - 6.0, b - 3.0), Pos2::new(c.x, b - 9.0), Pos2::new(c.x + 6.0, b - 3.0)],
                TEAL,
                Stroke::NONE,
            ));
            p.add(egui::Shape::closed_line(glass, st));
        }
        Some(Status::Paused) => {
            let r = Rect::from_center_size(c, Vec2::splat(26.0));
            p.rect_filled(r, 4.0, WHITE);
            p.rect_filled(Rect::from_center_size(c - Vec2::new(4.5, 0.0), Vec2::new(5.0, 14.0)), 0.0, INK);
            p.rect_filled(Rect::from_center_size(c + Vec2::new(4.5, 0.0), Vec2::new(5.0, 14.0)), 0.0, INK);
        }
        Some(Status::Completed) => trophy(p, ic.shrink(1.0), YELLOW),
        Some(Status::Dropped) => {
            // a cracked disc
            p.circle_filled(c, 13.0, Color32::from_gray(150));
            p.circle_filled(c, 4.0, INK);
            p.line_segment([c + Vec2::new(-4.0, -12.0), c + Vec2::new(1.0, -4.0)], Stroke::new(2.0f32, INK));
            p.line_segment([c + Vec2::new(1.0, -4.0), c + Vec2::new(-2.0, 0.0)], Stroke::new(2.0f32, INK));
            p.line_segment([c + Vec2::new(9.0, 9.0), c + Vec2::new(-9.0, -9.0)], Stroke::new(1.0f32, Color32::from_gray(90)));
        }
    }
}

pub fn trophy(p: &Painter, r: Rect, col: Color32) {
    let c = r.center();
    let s = r.width() / 28.0;
    let cup = Rect::from_min_max(Pos2::new(c.x - 8.0 * s, r.top() + 2.0 * s), Pos2::new(c.x + 8.0 * s, c.y + 3.0 * s));
    p.rect_filled(cup, 5.0 * s, col);
    p.circle_stroke(Pos2::new(cup.left(), cup.top() + 5.0 * s), 4.0 * s, Stroke::new(2.0 * s, col));
    p.circle_stroke(Pos2::new(cup.right(), cup.top() + 5.0 * s), 4.0 * s, Stroke::new(2.0 * s, col));
    p.rect_filled(Rect::from_center_size(Pos2::new(c.x, cup.bottom() + 3.0 * s), Vec2::new(3.0 * s, 6.0 * s)), 0.0, col);
    p.rect_filled(Rect::from_center_size(Pos2::new(c.x, r.bottom() - 3.0 * s), Vec2::new(14.0 * s, 4.0 * s)), 1.0, col);
}

/// Stand-in for a site's favicon until (or unless) the real one arrives.
pub fn kind_badge(p: &Painter, r: Rect, kind: Kind) {
    let c = r.center();
    let s = r.width();
    let (bg, fg) = match kind {
        Kind::Video => (Color32::from_rgb(0xE6, 0x21, 0x17), WHITE),
        Kind::Sheet => (Color32::from_rgb(0x1E, 0x8E, 0x3E), WHITE),
        Kind::Wiki => (Color32::from_rgb(0x52, 0x40, 0x92), YELLOW),
        Kind::Trophy => (Color32::from_rgb(0x1F, 0x3E, 0x6E), Color32::from_gray(220)),
        Kind::Map => (Color32::from_rgb(0x2E, 0x6B, 0x5A), YELLOW),
        Kind::Doc => (Color32::from_gray(230), INK),
        Kind::Web => (INDIGO, TEAL),
    };
    p.rect_filled(r, s * 0.18, bg);
    match kind {
        Kind::Video => {
            let k = s * 0.22;
            p.add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(-k * 0.8, -k), c + Vec2::new(k * 1.1, 0.0), c + Vec2::new(-k * 0.8, k)],
                fg,
                Stroke::NONE,
            ));
        }
        Kind::Sheet => {
            let g = r.shrink(s * 0.22);
            p.rect_stroke(g, 0.0, Stroke::new(1.0f32, fg), StrokeKind::Inside);
            for i in 1..3 {
                let y = g.top() + g.height() * i as f32 / 3.0;
                p.line_segment([Pos2::new(g.left(), y), Pos2::new(g.right(), y)], Stroke::new(1.0f32, fg));
            }
            let x = g.center().x;
            p.line_segment([Pos2::new(x, g.top()), Pos2::new(x, g.bottom())], Stroke::new(1.0f32, fg));
        }
        Kind::Wiki | Kind::Doc => {
            let g = r.shrink(s * 0.22);
            for i in 0..3 {
                let y = g.top() + 1.0 + i as f32 * g.height() / 3.0;
                let w = if i == 2 { g.width() * 0.6 } else { g.width() };
                p.line_segment([Pos2::new(g.left(), y), Pos2::new(g.left() + w, y)], Stroke::new(1.5f32, fg));
            }
        }
        Kind::Trophy => trophy(p, r.shrink(s * 0.15), fg),
        Kind::Map => {
            let k = s * 0.2;
            p.circle_filled(c - Vec2::new(0.0, k * 0.5), k, fg);
            p.add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(-k * 0.9, -k * 0.1), c + Vec2::new(k * 0.9, -k * 0.1), c + Vec2::new(0.0, k * 1.6)],
                fg,
                Stroke::NONE,
            ));
            p.circle_filled(c - Vec2::new(0.0, k * 0.5), k * 0.4, bg);
        }
        Kind::Web => {
            let k = s * 0.3;
            p.circle_stroke(c, k, Stroke::new(1.2f32, fg));
            p.line_segment([c - Vec2::new(k, 0.0), c + Vec2::new(k, 0.0)], Stroke::new(1.0f32, fg));
            p.line_segment([c - Vec2::new(0.0, k), c + Vec2::new(0.0, k)], Stroke::new(1.0f32, fg));
        }
    }
}

/// Small icon for a checklist item's type. `r` is about 20×20.
pub fn step_icon(p: &Painter, r: Rect, kind: crate::model::StepKind, dim: bool) {
    use crate::model::StepKind;
    let k = |c: Color32| if dim { c.gamma_multiply(0.45) } else { c };
    let c = r.center();
    match kind {
        StepKind::Chest => {
            let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(r.width(), r.height() * 0.6));
            let lid = Rect::from_min_max(Pos2::new(body.left(), r.top() + 2.0), Pos2::new(body.right(), body.top() + 2.0));
            p.rect_filled(lid, 3.0, k(Color32::from_rgb(0xB5, 0x6A, 0x2E)));
            p.rect_filled(body, 1.0, k(Color32::from_rgb(0x8E, 0x4E, 0x1E)));
            p.rect_filled(Rect::from_center_size(Pos2::new(c.x, body.top()), Vec2::new(r.width(), 2.5)), 0.0, k(YELLOW));
            p.rect_filled(Rect::from_center_size(Pos2::new(c.x, body.top() + 2.0), Vec2::new(4.0, 5.0)), 1.0, k(YELLOW));
        }
        StepKind::Objective => {
            p.circle_filled(c, r.width() * 0.5, k(TEAL));
            p.circle_filled(c, r.width() * 0.32, k(INK));
            p.circle_filled(c, r.width() * 0.16, k(TEAL));
        }
        StepKind::Achievement => trophy(p, r, k(YELLOW)),
        StepKind::Task => {
            let s = r.width() * 0.32;
            p.add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(0.0, -s), c + Vec2::new(s, 0.0), c + Vec2::new(0.0, s), c + Vec2::new(-s, 0.0)],
                k(MAGENTA),
                Stroke::NONE,
            ));
        }
    }
}

/// A pixel folder for the games explorer. `r` is about 20×16. Top-level folders are brighter.
pub fn folder_icon(p: &Painter, r: Rect, open: bool, top: bool) {
    let body_col = if top { YELLOW } else { Color32::from_rgb(0xC9, 0xA4, 0x3A) };
    let back_col = Color32::from_rgb(0xB0, 0x80, 0x22);
    let tab = Rect::from_min_size(r.min, Vec2::new(r.width() * 0.45, 4.0));
    p.rect_filled(tab, 1.0, back_col);
    let body = Rect::from_min_max(Pos2::new(r.left(), r.top() + 3.0), r.max);
    p.rect_filled(body, 1.5, back_col);
    if open {
        // the front flap tilts forward
        let flap = vec![
            Pos2::new(r.left() + 3.0, r.top() + 7.0),
            Pos2::new(r.right() + 1.0, r.top() + 7.0),
            Pos2::new(r.right() - 2.0, r.bottom()),
            Pos2::new(r.left(), r.bottom()),
        ];
        p.add(egui::Shape::convex_polygon(flap, body_col, Stroke::new(1.0f32, INK)));
    } else {
        let front = Rect::from_min_max(Pos2::new(r.left(), r.top() + 5.0), r.max);
        p.rect_filled(front, 1.5, body_col);
        p.rect_stroke(front, 1.5, Stroke::new(1.0f32, INK), StrokeKind::Inside);
    }
}

/// A large explorer folder (Windows "Large icons" style). `peek` colours show as the edges of the
/// items inside, sticking out of the top, like the files peeking out of a real folder icon.
pub fn folder_big(p: &Painter, r: Rect, peek: &[Color32]) {
    let w = r.width();
    let h = r.height();
    let back = Color32::from_rgb(0xD9, 0xA4, 0x2B);
    let front = YELLOW;
    // back panel with its tab
    let tab = Rect::from_min_size(r.min, Vec2::new(w * 0.42, h * 0.16));
    p.rect_filled(tab, 3.0, back);
    let body = Rect::from_min_max(Pos2::new(r.left(), r.top() + h * 0.1), r.max);
    p.rect_filled(body, 4.0, back);
    // the things inside, peeking out
    for (i, c) in peek.iter().take(3).enumerate() {
        let x = r.left() + w * 0.14 + i as f32 * w * 0.2;
        let sheet = Rect::from_min_size(Pos2::new(x, r.top() + h * 0.18 - i as f32 * 2.0), Vec2::new(w * 0.42, h * 0.5));
        p.rect_filled(sheet, 2.0, Color32::from_gray(235));
        p.rect_filled(Rect::from_min_size(sheet.min, Vec2::new(sheet.width(), h * 0.09)), 2.0, *c);
        p.rect_stroke(sheet, 2.0, Stroke::new(1.0f32, Color32::from_gray(150)), StrokeKind::Inside);
    }
    // front panel
    let front_r = Rect::from_min_max(Pos2::new(r.left(), r.top() + h * 0.36), r.max);
    p.rect_filled(front_r, 4.0, front);
    p.rect_filled(Rect::from_min_size(front_r.min, Vec2::new(front_r.width(), 3.0)), 2.0, Color32::from_rgb(0xFF, 0xE0, 0x80));
    p.rect_stroke(front_r, 4.0, Stroke::new(1.5f32, INK), StrokeKind::Inside);
}

/// A game case tile: coloured spine band for the platform, the guide site's icon (or a pad) on the
/// cover, and a strip at the bottom for the status colour.
pub fn game_case(p: &Painter, r: Rect, band: Color32, cover: Option<egui::TextureId>, status: Color32) {
    p.rect_filled(r, 4.0, Color32::from_rgb(0x2B, 0x27, 0x45));
    p.rect_stroke(r, 4.0, Stroke::new(1.5f32, Color32::from_gray(200)), StrokeKind::Inside);
    let top = Rect::from_min_size(r.min, Vec2::new(r.width(), r.height() * 0.17));
    p.rect_filled(top, 4.0, band);
    p.rect_filled(Rect::from_min_size(Pos2::new(r.left(), top.bottom() - 4.0), Vec2::new(r.width(), 4.0)), 0.0, band);
    let art = Rect::from_center_size(Pos2::new(r.center().x, r.top() + r.height() * 0.56), Vec2::splat(r.width() * 0.55));
    match cover {
        Some(t) => {
            p.rect_filled(art.expand(3.0), 3.0, Color32::from_gray(240));
            p.image(t, art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => filter_icon(p, art, Some(crate::model::Status::Playing)),
    }
    p.rect_filled(Rect::from_min_max(Pos2::new(r.left() + 4.0, r.bottom() - 6.0), Pos2::new(r.right() - 4.0, r.bottom() - 3.0)), 1.0, status);
}

/// Toolbar arrows: back (←), forward (→), up (↑).
pub fn arrow(p: &Painter, r: Rect, dir: Vec2, col: Color32) {
    let c = r.center();
    let s = r.width().min(r.height()) * 0.32;
    let tip = c + dir * s;
    let tail = c - dir * s;
    let side = Vec2::new(-dir.y, dir.x) * s * 0.7;
    let st = Stroke::new(2.0f32, col);
    p.line_segment([tail, tip], st);
    p.line_segment([tip, tip - dir * s * 0.7 + side], st);
    p.line_segment([tip, tip - dir * s * 0.7 - side], st);
}
