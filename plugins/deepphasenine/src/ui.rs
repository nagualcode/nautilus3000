//! The editor, laid out like the 2000 original (interface by Captain Video):
//!
//! ┌ DEEP PHASE NINE ───────────────────────────┬──────────────┐
//! │ notches display: both channels' response,   │ notches      │
//! │ low · offset · high                         │ spread       │
//! ├ LFO: shape · oval with the yellow ball      │ feedback     │
//! │ speed · sync to beats · one-shot socket     │ mix · output │
//! └─────────────────────────────────────────────┴──────────────┘

use crate::engine::section_coefs;
use crate::params::{DpnParams, DpnParamsParamId as P, LfoShape, OneShotDirection};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use truce::prelude::*;
use truce_core::editor::PluginContext;
use truce_egui::widgets::{level_meter, param_knob};

pub const WINDOW_W: u32 = 900;
pub const WINDOW_H: u32 = 580;
pub const MIN_W: u32 = 800;
pub const MIN_H: u32 = 540;
pub const MAX_W: u32 = 1800;
pub const MAX_H: u32 = 1200;

type Ctx = PluginContext<DpnParams>;

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    screen: Color32,
    grid: Color32,
    text: Color32,
    dim: Color32,
    accent: Color32,
    accent_text: Color32,
    left: Color32,
    right: Color32,
    ball: Color32,
    header: Color32,
}

const MODERN: Palette = Palette {
    bg: Color32::from_rgb(28, 30, 33),
    panel: Color32::from_rgb(40, 44, 48),
    screen: Color32::from_rgb(16, 20, 24),
    grid: Color32::from_rgb(52, 58, 64),
    text: Color32::from_rgb(214, 218, 222),
    dim: Color32::from_rgb(128, 136, 144),
    accent: Color32::from_rgb(255, 170, 60),
    accent_text: Color32::from_rgb(24, 18, 10),
    left: Color32::from_rgb(250, 210, 60),
    right: Color32::from_rgb(235, 70, 60),
    ball: Color32::from_rgb(255, 225, 40),
    header: Color32::from_rgb(54, 58, 62),
};

/// The original: orange case, teal panels, a sea in the display.
const CLASSIC: Palette = Palette {
    bg: Color32::from_rgb(205, 112, 24),
    panel: Color32::from_rgb(64, 128, 146),
    screen: Color32::from_rgb(150, 182, 196),
    grid: Color32::from_rgb(110, 150, 166),
    text: Color32::from_rgb(244, 246, 240),
    dim: Color32::from_rgb(214, 232, 236),
    accent: Color32::from_rgb(250, 220, 50),
    accent_text: Color32::from_rgb(20, 20, 20),
    left: Color32::from_rgb(250, 220, 40),
    right: Color32::from_rgb(210, 30, 30),
    ball: Color32::from_rgb(255, 230, 20),
    header: Color32::from_rgb(70, 72, 72),
};

fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.screen;
    v.faint_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent_text);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, p.header),
        (&mut v.widgets.inactive, p.header),
        (&mut v.widgets.hovered, p.grid),
        (&mut v.widgets.active, p.accent),
        (&mut v.widgets.open, p.header),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = egui::CornerRadius::same(3);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.dim);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, p.text);
    v
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn normalized(ctx: &Ctx, id: P, plain: f64) -> f64 {
    let id: u32 = id.into();
    ctx.params().param_infos().iter().find(|i| i.id == id).map(|i| i.range.normalize(plain)).unwrap_or(0.0)
}

fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(p.dim).strong());
}

fn pill(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool) -> egui::Response {
    let (fill, fg) = if on { (p.accent, p.accent_text) } else { (p.header, p.text) };
    ui.add(egui::Button::new(egui::RichText::new(text).size(11.5).color(fg)).fill(fill).min_size(vec2(0.0, 22.0)))
}

fn enum_combo<E: ParamEnum>(ui: &mut egui::Ui, ctx: &Ctx, id: P, current: usize, width: f32) {
    let names = E::variant_names();
    egui::ComboBox::from_id_salt(("dpn-enum", u32::from(id)))
        .width(width)
        .selected_text(egui::RichText::new(names[current.min(names.len() - 1)]).size(11.5))
        .show_ui(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                if ui.selectable_label(i == current, *name).clicked() && i != current {
                    ctx.automate(id, normalized(ctx, id, i as f64));
                }
            }
        });
}

fn hslider(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, id: P, width: f32, overlay: Option<&dyn Fn(&egui::Painter, Rect)>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    if let Some(f) = overlay {
        f(&painter, rect);
    }
    let v = ctx.get_param(id) as f32;
    let x = rect.left() + 4.0 + v * (rect.width() - 8.0);
    painter.rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(8.0, 14.0)), 2.0, p.right);
    painter.text(rect.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, ctx.format_param(id), FontId::proportional(10.5), alpha(p.text, 0.9));
    if resp.drag_started() {
        ctx.begin_edit(id);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            let n = ((pt.x - rect.left() - 4.0) / (rect.width() - 8.0)).clamp(0.0, 1.0) as f64;
            if resp.clicked() { ctx.automate(id, n) } else { ctx.set_param(id, n) }
        }
    }
    if resp.drag_stopped() {
        ctx.end_edit(id);
    }
    resp
}

fn panel(ui: &mut egui::Ui, fill: Color32, size: egui::Vec2, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.fill(fill).corner_radius(5.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        let inner = size - vec2(20.0, 20.0);
        ui.set_min_size(inner);
        ui.set_max_size(inner);
        ui.vertical(|ui| add(ui));
    });
}

// --- Editor ------------------------------------------------------------------

pub fn dpn_ui(ui: &mut egui::Ui, ctx: &Ctx) {
    let params: &Arc<DpnParams> = ctx.params();
    let classic = params.theme.read().map(|t| *t == 1).unwrap_or(false);
    let p = if classic { CLASSIC } else { MODERN };
    let v = visuals(&p);
    ui.ctx().set_visuals(v.clone());
    ui.style_mut().visuals = v;

    egui::Panel::top("dpn-top")
        .exact_size(38.0)
        .frame(egui::Frame::NONE.fill(p.header).inner_margin(egui::Margin::symmetric(12, 8)))
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new("DEEP PHASE NINE 3K").size(16.0).strong().italics().color(p.text));
                ui.label(egui::RichText::new("syncable 24-notch true deep phaser · nautilus3000").size(10.5).color(p.dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill(ui, &p, if classic { "Classic" } else { "Modern" }, false).on_hover_text("Theme").clicked() {
                        if let Ok(mut t) = params.theme.write() {
                            *t = if classic { 0 } else { 1 };
                        }
                    }
                });
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.bg).inner_margin(egui::Margin::same(12)))
        .show_inside(ui, |ui| {
            let h = ui.available_height();
            ui.horizontal_top(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let side = 200.0;
                let w = ui.available_width() - side - gap;
                ui.vertical(|ui| {
                    let top = (h * 0.64).floor();
                    panel(ui, p.panel, vec2(w, top), |ui| display_panel(ui, ctx, &p));
                    panel(ui, p.panel, vec2(w, h - top - ui.spacing().item_spacing.y), |ui| lfo_panel(ui, ctx, &p));
                });
                panel(ui, p.panel, vec2(side, h), |ui| side_panel(ui, ctx, &p));
            });
        });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

// --- Notches display ---------------------------------------------------------

/// |H(e^jω)| of the phaser: x + depth·A^N/(1 − fb·A^N), A the section.
fn response(c: &[f32; 5], n: i32, fb: f32, depth: f32, omega: f64) -> f64 {
    let (a1, a2, b0, b1, b2) = (c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64, c[4] as f64);
    let (c1, s1, c2, s2) = (omega.cos(), -omega.sin(), (2.0 * omega).cos(), -(2.0 * omega).sin());
    let num = (b0 + b1 * c1 + b2 * c2, b1 * s1 + b2 * s2);
    let den = (1.0 + a1 * c1 + a2 * c2, a1 * s1 + a2 * s2);
    let d2 = den.0 * den.0 + den.1 * den.1;
    let a = ((num.0 * den.0 + num.1 * den.1) / d2, (num.1 * den.0 - num.0 * den.1) / d2);
    // A^N
    let (mut pr, mut pi) = (1.0, 0.0);
    for _ in 0..n {
        let t = pr * a.0 - pi * a.1;
        pi = pr * a.1 + pi * a.0;
        pr = t;
    }
    // y = A^N / (1 − fb·z⁻¹·A^N)  (one sample of loop delay)
    let fbr = fb as f64 * (pr * c1 - pi * s1);
    let fbi = fb as f64 * (pr * s1 + pi * c1);
    let (dr, di) = (1.0 - fbr, -fbi);
    let dd = dr * dr + di * di;
    let (yr, yi) = ((pr * dr + pi * di) / dd, (pi * dr - pr * di) / dd);
    let (hr, hi) = (1.0 + depth as f64 * yr, depth as f64 * yi);
    (hr * hr + hi * hi).sqrt()
}

fn display_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    let st = &params.status;
    let sr = st.sample_rate.load(Ordering::Relaxed) as f64;
    label(ui, p, "NOTCHES");
    let w = ui.available_width();
    let h = (ui.available_height() - 104.0).max(80.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, p.screen);
    let (f0, f1) = (20.0f64, 20000.0f64);
    let fx = |f: f64| rect.left() + ((f.max(f0) / f0).ln() / (f1 / f0).ln()) as f32 * rect.width();
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = fx(f);
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0_f32, alpha(p.grid, 0.6)));
        let t = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        painter.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, t, FontId::proportional(9.0), alpha(p.dim, 0.7));
    }
    // Sweep range.
    let (lo, hi) = (params.low.value() as f64, params.high.value() as f64);
    let band = Rect::from_x_y_ranges(fx(lo.min(hi))..=fx(hi.max(lo)), rect.y_range());
    painter.rect_filled(band, 0.0, alpha(p.grid, 0.25));

    let n = params.notches.value() as i32;
    let fb = params.feedback.value();
    let depth = params.mix.value();
    let spread = params.spread.value().max(2.0) / 200.0;
    let curves = [(st.freq_r.load(Ordering::Relaxed), p.right), (st.freq_l.load(Ordering::Relaxed), p.left)];
    for (bits, col) in curves.iter() {
        let wc = f32::from_bits(*bits);
        let c = section_coefs(wc, spread);
        let steps = rect.width() as usize;
        let mut pts = Vec::with_capacity(steps + 2);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let f = f0 * (f1 / f0).powf(t);
            let om = std::f64::consts::PI * f / (sr * 0.5);
            if om >= std::f64::consts::PI {
                break;
            }
            let db = 20.0 * response(&c, n, fb, depth, om).max(1e-6).log10();
            // 0 dB in the upper third, −36 dB at the bottom
            let y = rect.top() + rect.height() * (0.3 + (-(db as f32)).clamp(-12.0, 36.0) / 36.0 * 0.7);
            pts.push(pos2(rect.left() + t as f32 * rect.width(), y.clamp(rect.top(), rect.bottom())));
        }
        painter.add(egui::Shape::line(pts, Stroke::new(1.8_f32, alpha(*col, 0.25 + 0.75 * depth))));
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Low, "low");
        ui.add_space(20.0);
        param_knob(ui, ctx, P::Offset, "stereo offset");
        ui.add_space(20.0);
        param_knob(ui, ctx, P::High, "high");
    });
}

// --- LFO -------------------------------------------------------------------------

fn lfo_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params().clone();
    let st = &params.status;
    let shape = params.lfo.index() as usize;
    let one_shot = shape >= 3;
    ui.horizontal(|ui| {
        label(ui, p, "LFO");
        enum_combo::<LfoShape>(ui, ctx, P::Lfo, shape, 150.0);
        if one_shot {
            ui.add_space(8.0);
            label(ui, p, "one shot");
            enum_combo::<OneShotDirection>(ui, ctx, P::OneshotDir, params.oneshot_dir.index() as usize, 70.0);
        }
    });
    ui.add_space(6.0);

    // The oval: x = the sweep position, top half moving up, bottom down.
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 74.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let oval = rect.shrink2(vec2(14.0, 8.0));
    let c = oval.center();
    let (rx, ry) = (oval.width() * 0.5, oval.height() * 0.5);
    let pts: Vec<Pos2> = (0..=64)
        .map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(pts, alpha(Color32::BLACK, 0.25), Stroke::new(1.5_f32, alpha(p.dim, 0.6))));
    let at = |v: f32, up: bool| {
        let x = (v.clamp(0.0, 1.0) * 2.0 - 1.0) * rx;
        let y = ry * (1.0 - (x / rx).powi(2)).max(0.0).sqrt();
        pos2(c.x + x, if up { c.y - y } else { c.y + y })
    };
    if one_shot {
        let up = params.oneshot_dir.index() == 2;
        painter.circle_stroke(at(params.oneshot_end.value(), up), 8.0, Stroke::new(2.0_f32, p.ball));
    }
    let v = f32::from_bits(st.phase.load(Ordering::Relaxed));
    let dir = st.dir.load(Ordering::Relaxed);
    let ball = at(v, dir > 0);
    painter.circle_filled(ball, 6.5, p.ball);
    painter.circle_stroke(ball, 6.5, Stroke::new(1.0_f32, Color32::BLACK));

    // Dragging holds the ball ("Hold" / "UIF Position" in the original);
    // with a one-shot, alt-drag moves the socket instead.
    if let Some(pt) = resp.interact_pointer_pos() {
        let vv = ((pt.x - c.x) / rx * 0.5 + 0.5).clamp(0.0, 1.0);
        let up = pt.y < c.y;
        if one_shot && ui.input(|i| i.modifiers.alt) {
            ctx.automate(P::OneshotEnd, vv as f64);
            ctx.automate(P::OneshotDir, normalized(ctx, P::OneshotDir, if up { 2.0 } else { 0.0 }));
        } else if resp.dragged() || resp.is_pointer_button_down_on() {
            st.drag_pos.store(vv.to_bits(), Ordering::Relaxed);
            st.drag_dir.store(if up { 1 } else { -1 }, Ordering::Relaxed);
        }
    }
    if resp.drag_stopped() || resp.clicked() || !resp.is_pointer_button_down_on() {
        st.drag_pos.store((-1.0f32).to_bits(), Ordering::Relaxed);
    }
    resp.on_hover_text(if one_shot {
        "Drag the ball; alt-drag places the socket where a one-shot stops"
    } else {
        "Drag the yellow ball to move the notches by hand"
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, p, "speed");
        let sync = params.sync.value();
        let w = ui.available_width() - 130.0;
        let ticks = |painter: &egui::Painter, r: Rect| {
            if sync {
                for i in 0..=70 {
                    if i % 5 == 0 {
                        let x = r.left() + 4.0 + i as f32 / 70.0 * (r.width() - 8.0);
                        painter.line_segment([pos2(x, r.top() + 4.0), pos2(x, r.bottom() - 4.0)], Stroke::new(1.0_f32, Color32::from_gray(80)));
                    }
                }
            }
        };
        hslider(ui, ctx, p, P::Speed, w, Some(&ticks));
        if pill(ui, p, "sync to beats", sync).clicked() {
            ctx.automate(P::Sync, if sync { 0.0 } else { 1.0 });
        }
    });
}

// --- Right column ----------------------------------------------------------------

fn side_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    label(ui, p, "notches");
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    let n = params.notches.value() as usize;
    for i in 0..24 {
        let x = rect.left() + 3.0 + i as f32 * (rect.width() - 6.0) / 24.0;
        let bar = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2((rect.width() - 6.0) / 24.0 - 1.5, rect.height() - 6.0));
        painter.rect_filled(bar, 1.0, if i < n { p.left } else { Color32::from_gray(45) });
    }
    if let Some(pt) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let k = (((pt.x - rect.left()) / rect.width() * 24.0).ceil() as i64).clamp(1, 24);
            ctx.automate(P::Notches, normalized(ctx, P::Notches, k as f64));
        }
    }
    ui.label(egui::RichText::new(format!("{n} notches per channel")).size(10.5).color(p.dim));
    ui.add_space(8.0);
    param_knob(ui, ctx, P::Spread, "spread");
    param_knob(ui, ctx, P::Feedback, "feedback");
    param_knob(ui, ctx, P::Mix, "mix");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Output, "output");
        level_meter(ui, ctx, &[P::MeterL, P::MeterR], 70.0);
    });
    let g = f32::from_bits(params.status.limiter.load(Ordering::Relaxed));
    ui.label(egui::RichText::new(format!("limiter {:+.1} dB", 20.0 * g.max(1e-6).log10())).size(10.5).color(p.dim))
        .on_hover_text("The original look-ahead peak limiter (−6 dBFS, 300 samples)");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test --release -p deepphasenine --lib readme_screenshots -- --ignored`
    #[test]
    #[ignore]
    fn readme_screenshots() {
        let params = Arc::new(DpnParams::new());
        params.notches.set_value(6);
        params.status.freq_l.store(0.05f32.to_bits(), Ordering::Relaxed);
        params.status.freq_r.store(0.12f32.to_bits(), Ordering::Relaxed);
        params.status.phase.store(0.3f32.to_bits(), Ordering::Relaxed);
        params.status.sample_rate.store(44100, Ordering::Relaxed);
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        std::fs::create_dir_all(&docs).unwrap();
        for (theme, file) in [(0u8, docs.join("deepphasenine.png")), (1, docs.join("deepphasenine-classic.png"))] {
            *params.theme.write().unwrap() = theme;
            let mut editor = crate::DpnPlugin::editor(params.clone());
            if let Some((px, w, h)) = editor.screenshot(params.clone()) {
                truce_test::save_png(&file, &px, w, h);
            }
        }
    }
}
