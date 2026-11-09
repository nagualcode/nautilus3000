//! The editor. Same anatomy as the 2000 original (interface by Captain Video):
//!
//! ┌ RIVERRUN · file · theme ─────────────────────────────────────────────┐
//! │ input screen: waveform, red/yellow needles, Grain Glass, live grains │ ● record
//! │ walk  [−] ────────────●──────────── [+]                               │
//! ├ shape ▲▱◣◠ · mono/stereo ─────────── · load / save / demo / clear ──┤
//! │ TEXTURE: flowing/rhythmic, speed, speed random, length │ TRANSPOSITION │ OUT │
//! └──────────────────────────────────────────────────────────────────────┘

use crate::audio;
use crate::demo;
use crate::engine::tables::{self, breakpoint, to_short, Scales};
use crate::params::{
    unpack_grain, GrainMode, RiverrunParams, RiverrunParamsParamId as P, GRAIN_TRAIL, OVERVIEW,
};
use crate::snippet::SnippetData;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use truce::prelude::*;
use truce_core::editor::PluginContext;
use truce_egui::widgets::{level_meter, param_knob};

pub const WINDOW_W: u32 = 980;
pub const WINDOW_H: u32 = 620;
pub const MIN_W: u32 = 860;
pub const MIN_H: u32 = 560;
pub const MAX_W: u32 = 1900;
pub const MAX_H: u32 = 1200;

type Ctx = PluginContext<RiverrunParams>;

// --- Themes ------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    texture: Color32,
    transpose: Color32,
    header: Color32,
    screen: Color32,
    wave: Color32,
    grid: Color32,
    text: Color32,
    dim: Color32,
    accent: Color32,
    accent_text: Color32,
    red_needle: Color32,
    yellow_needle: Color32,
    glass: Color32,
    record: Color32,
    key_hit: Color32,
    band: Color32,
}

const MODERN: Palette = Palette {
    bg: Color32::from_rgb(28, 30, 31),
    panel: Color32::from_rgb(40, 43, 44),
    texture: Color32::from_rgb(40, 43, 44),
    transpose: Color32::from_rgb(40, 43, 44),
    header: Color32::from_rgb(54, 58, 60),
    screen: Color32::from_rgb(18, 22, 23),
    wave: Color32::from_rgb(120, 200, 190),
    grid: Color32::from_rgb(56, 62, 64),
    text: Color32::from_rgb(214, 218, 218),
    dim: Color32::from_rgb(128, 136, 138),
    accent: Color32::from_rgb(64, 214, 190),
    accent_text: Color32::from_rgb(12, 20, 20),
    red_needle: Color32::from_rgb(235, 70, 60),
    yellow_needle: Color32::from_rgb(245, 205, 60),
    glass: Color32::from_rgb(200, 245, 240),
    record: Color32::from_rgb(230, 40, 40),
    key_hit: Color32::from_rgb(235, 70, 60),
    band: Color32::from_rgb(70, 130, 255),
};

/// The original: teal case, brushed screen, red Texture panel.
const CLASSIC: Palette = Palette {
    bg: Color32::from_rgb(24, 128, 110),
    panel: Color32::from_rgb(52, 58, 58),
    texture: Color32::from_rgb(160, 46, 34),
    transpose: Color32::from_rgb(150, 40, 30),
    header: Color32::from_rgb(70, 72, 72),
    screen: Color32::from_rgb(150, 172, 158),
    wave: Color32::from_rgb(40, 66, 56),
    grid: Color32::from_rgb(20, 20, 20),
    text: Color32::from_rgb(250, 248, 236),
    dim: Color32::from_rgb(210, 220, 210),
    accent: Color32::from_rgb(240, 236, 120),
    accent_text: Color32::from_rgb(20, 20, 20),
    red_needle: Color32::from_rgb(220, 20, 20),
    yellow_needle: Color32::from_rgb(250, 220, 30),
    glass: Color32::from_rgb(255, 255, 255),
    record: Color32::from_rgb(235, 20, 20),
    key_hit: Color32::from_rgb(220, 20, 20),
    band: Color32::from_rgb(60, 110, 240),
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

// --- Cached data -------------------------------------------------------------

#[derive(Default)]
struct Cache {
    mirror_gen: u64,
    peaks: Vec<f32>,
    name: String,
    message: String,
    scales: Option<Scales>,
    glass_grab: Option<f32>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Cache>>);

fn cache(ui: &egui::Ui) -> Shared {
    ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Shared>(egui::Id::new("riverrun-cache")).clone())
}

fn set_message(ui: &egui::Ui, msg: impl Into<String>) {
    if let Ok(mut c) = cache(ui).0.lock() {
        c.message = msg.into();
    }
}

// --- Parameter helpers -------------------------------------------------------

fn info_range(ctx: &Ctx, id: P) -> Option<truce::params::ParamRange> {
    let id: u32 = id.into();
    ctx.params().param_infos().iter().find(|i| i.id == id).map(|i| i.range)
}

fn normalized(ctx: &Ctx, id: P, plain: f64) -> f64 {
    info_range(ctx, id).map(|r| r.normalize(plain)).unwrap_or(0.0)
}

fn plain(ctx: &Ctx, id: P) -> f64 {
    ctx.get_param_plain(id) as f64
}

fn set_plain(ctx: &Ctx, id: P, v: f64) {
    ctx.set_param(id, normalized(ctx, id, v));
}

fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(p.dim).strong());
}

fn pill(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool) -> egui::Response {
    let (fill, fg) = if on { (p.accent, p.accent_text) } else { (p.header, p.text) };
    ui.add(egui::Button::new(egui::RichText::new(text).size(11.5).color(fg)).fill(fill).min_size(vec2(0.0, 22.0)))
}

/// A horizontal slider bound to a parameter, in the style of the original's
/// long black sliders with a red thumb.
fn hslider(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, id: P, width: f32, overlay: Option<&dyn Fn(&egui::Painter, Rect)>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    if let Some(f) = overlay {
        f(&painter, rect);
    }
    let v = ctx.get_param(id) as f32;
    let x = rect.left() + 4.0 + v * (rect.width() - 8.0);
    painter.rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(8.0, 14.0)), 2.0, p.red_needle);
    let text = ctx.format_param(id);
    painter.text(rect.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, text, FontId::proportional(10.5), alpha(p.text, 0.85));
    if resp.drag_started() {
        ctx.begin_edit(id);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pt) = resp.interact_pointer_pos() {
            let n = ((pt.x - rect.left() - 4.0) / (rect.width() - 8.0)).clamp(0.0, 1.0);
            if resp.clicked() {
                ctx.automate(id, n as f64);
            } else {
                ctx.set_param(id, n as f64);
            }
        }
    }
    if resp.drag_stopped() {
        ctx.end_edit(id);
    }
    resp
}

fn enum_combo<E: ParamEnum>(ui: &mut egui::Ui, ctx: &Ctx, id: P, current: usize, width: f32) {
    let names = E::variant_names();
    egui::ComboBox::from_id_salt(("rr-enum", u32::from(id)))
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

// --- Editor ------------------------------------------------------------------

pub fn riverrun_ui(ui: &mut egui::Ui, ctx: &Ctx) {
    let params: &Arc<RiverrunParams> = ctx.params();
    let classic = params.theme.read().map(|t| *t == 1).unwrap_or(false);
    let p = if classic { CLASSIC } else { MODERN };
    let v = visuals(&p);
    ui.ctx().set_visuals(v.clone());
    ui.style_mut().visuals = v;

    handle_dropped_files(ui, ctx);

    egui::Panel::top("rr-top")
        .exact_size(38.0)
        .frame(egui::Frame::NONE.fill(p.header).inner_margin(egui::Margin::symmetric(12, 8)))
        .show_inside(ui, |ui| top_bar(ui, ctx, &p, classic));

    egui::Panel::bottom("rr-status")
        .exact_size(24.0)
        .frame(egui::Frame::NONE.fill(p.header).inner_margin(egui::Margin::symmetric(12, 4)))
        .show_inside(ui, |ui| status_bar(ui, ctx, &p));

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.bg).inner_margin(egui::Margin::same(12)))
        .show_inside(ui, |ui| {
            input_screen(ui, ctx, &p);
            ui.add_space(8.0);
            shape_row(ui, ctx, &p);
            ui.add_space(10.0);
            let h = ui.available_height();
            ui.horizontal_top(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let w = ui.available_width() - 2.0 * gap;
                let tw = (w * 0.40).floor();
                let pw = (w * 0.37).floor();
                let ow = w - tw - pw;
                panel(ui, p.texture, vec2(tw, h), |ui| texture_panel(ui, ctx, &p));
                panel(ui, p.transpose, vec2(pw, h), |ui| transposition_panel(ui, ctx, &p));
                panel(ui, p.panel, vec2(ow, h), |ui| output_panel(ui, ctx, &p, classic));
            });
        });

    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

fn panel(ui: &mut egui::Ui, fill: Color32, size: egui::Vec2, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            let inner = size - vec2(20.0, 20.0);
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            ui.vertical(|ui| add(ui));
        });
}

fn top_bar(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, classic: bool) {
    let params = ctx.params().clone();
    ui.horizontal_centered(|ui| {
        ui.label(egui::RichText::new("★★★ R I V E R R U N").size(16.0).strong().color(p.text));
        ui.label(egui::RichText::new("real-time granular synthesizer · Nautilus Bundle").size(10.5).color(p.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pill(ui, p, if classic { "Classic" } else { "Modern" }, false).on_hover_text("Theme").clicked() {
                if let Ok(mut t) = params.theme.write() {
                    *t = if classic { 0 } else { 1 };
                }
            }
        });
    });
}

fn status_bar(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    let st = &params.status;
    let sr = st.sr();
    let snap = (sr * 4.65) as usize;
    let c = cache(ui);
    let (name, msg) = c.0.lock().map(|c| (c.name.clone(), c.message.clone())).unwrap_or_default();
    ui.horizontal_centered(|ui| {
        let rec = st.recording.load(Ordering::Relaxed);
        let txt = if rec { "● recording".to_string() } else if name.is_empty() { "screen empty — record, load or try the demo".into() } else { name };
        ui.label(egui::RichText::new(txt).size(11.0).color(if rec { p.record } else { p.dim }));
        ui.label(egui::RichText::new(format!("· {:.1} kHz · screen {:.2} s", sr / 1000.0, snap as f64 / sr)).size(11.0).color(p.dim));
        ui.label(egui::RichText::new(format!("· {} grains", st.grains_total.load(Ordering::Relaxed))).size(11.0).color(p.dim));
        if !msg.is_empty() {
            ui.label(egui::RichText::new(format!("· {msg}")).size(11.0).color(p.accent));
        }
    });
}

// --- Input screen ------------------------------------------------------------

fn refresh_peaks(ui: &egui::Ui, params: &RiverrunParams) -> Vec<f32> {
    let c = cache(ui);
    let mut cg = c.0.lock().unwrap();
    let mg = params.snippet.mirror_generation();
    if mg != cg.mirror_gen || cg.peaks.is_empty() {
        if let Ok(m) = params.snippet.mirror.try_lock() {
            cg.mirror_gen = mg;
            let mut peaks = vec![0.0f32; OVERVIEW];
            if !m.samples.is_empty() {
                let per = m.samples.len() as f32 / OVERVIEW as f32;
                for (i, pk) in peaks.iter_mut().enumerate() {
                    let a = (i as f32 * per) as usize;
                    let b = (((i + 1) as f32 * per) as usize).min(m.samples.len());
                    *pk = m.samples[a..b].iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
                }
            }
            cg.peaks = peaks;
            cg.name = m.name.clone();
        }
    }
    cg.peaks.clone()
}

fn input_screen(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params().clone();
    let st = &params.status;
    let full_w = ui.available_width();
    ui.horizontal_top(|ui| {
        let w = full_w - 96.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 150.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, p.screen);
        let px_to_x = |px: f32| rect.left() + px / 472.0 * rect.width();
        let x_to_px = |x: f32| ((x - rect.left()) / rect.width() * 472.0).clamp(0.0, 472.0);

        // Waveform: the upper half, like the original's input screen.
        let peaks = refresh_peaks(ui, &params);
        let base = rect.bottom() - 6.0;
        let top = rect.top() + 18.0;
        for (i, &pk) in peaks.iter().enumerate() {
            let x = rect.left() + (i as f32 + 0.5) / OVERVIEW as f32 * rect.width();
            let h = pk.min(1.0).sqrt() * (base - top);
            painter.line_segment([pos2(x, base), pos2(x, base - h)], Stroke::new((rect.width() / OVERVIEW as f32).max(1.0), alpha(p.wave, 0.9)));
        }
        if peaks.iter().all(|&v| v == 0.0) {
            painter.text(rect.center(), Align2::CENTER_CENTER, "press ● to record the track, load a sound, or drop a file here", FontId::proportional(12.0), alpha(p.wave, 0.8));
        }

        // Needles (red = left, yellow = right).
        let nl = plain(ctx, P::NeedleL) as f32;
        let nr = plain(ctx, P::NeedleR) as f32;
        let (xl, xr) = (px_to_x(nl), px_to_x(nr));
        painter.rect_filled(Rect::from_x_y_ranges(rect.left()..=xl, rect.y_range()), 0.0, alpha(Color32::BLACK, 0.25));
        painter.rect_filled(Rect::from_x_y_ranges(xr..=rect.right(), rect.y_range()), 0.0, alpha(Color32::BLACK, 0.25));
        for (x, col) in [(xl, p.red_needle), (xr, p.yellow_needle)] {
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0_f32, col));
            painter.add(egui::Shape::convex_polygon(vec![pos2(x - 7.0, rect.top()), pos2(x + 7.0, rect.top()), pos2(x, rect.top() + 11.0)], col, Stroke::NONE));
        }

        // Grain Glass.
        let gpx = st.glass_px.load(Ordering::Relaxed) as f32 / 100.0;
        let gw = (st.glass_w_px.load(Ordering::Relaxed) as f32 / 100.0).max(1.0);
        let glass = Rect::from_x_y_ranges(px_to_x(gpx)..=px_to_x(gpx + gw), (rect.top() + 14.0)..=(rect.bottom() - 2.0));
        painter.rect_filled(glass, 3.0, alpha(p.glass, 0.22));
        painter.rect_stroke(glass, 3.0, Stroke::new(1.5_f32, alpha(p.glass, 0.8)), egui::StrokeKind::Inside);

        // Live grains: x = where they were read, y = pitch (high = up), colour = side.
        let seq = st.grain_seq.load(Ordering::Acquire);
        for k in 0..GRAIN_TRAIL as u32 {
            let i = seq.wrapping_sub(1 + k) as usize % GRAIN_TRAIL;
            let v = st.grains[i].load(Ordering::Relaxed);
            if v == 0 {
                continue;
            }
            let (pos, ratio, bus) = unpack_grain(v);
            let age = 1.0 - k as f32 / GRAIN_TRAIL as f32;
            let y = top + (1.0 - ((ratio.log2() + 4.0) / 9.0).clamp(0.0, 1.0)) * (base - top);
            let x = rect.left() + pos * rect.width();
            let col = if bus == 0 { p.yellow_needle } else { p.accent };
            painter.circle_filled(pos2(x, y), 1.5 + 2.5 * age, alpha(col, 0.15 + 0.7 * age));
        }

        // Interaction: needles near the top edge, the glass everywhere else.
        let c = cache(ui);
        if let Some(pt) = resp.interact_pointer_pos() {
            let px = x_to_px(pt.x);
            let near_top = pt.y < rect.top() + 16.0;
            let target = if near_top { if (pt.x - xl).abs() <= (pt.x - xr).abs() { Some(P::NeedleL) } else { Some(P::NeedleR) } } else { None };
            if resp.drag_started() || resp.clicked() {
                let grab = if near_top { None } else { Some(if glass.contains(pt) { px - gpx } else { gw * 0.5 }) };
                if let Ok(mut cg) = c.0.lock() {
                    cg.glass_grab = grab;
                }
                if let Some(t) = target {
                    ctx.begin_edit(t);
                } else {
                    ctx.begin_edit(P::Glass);
                }
            }
            let grab = c.0.lock().ok().and_then(|cg| cg.glass_grab);
            if resp.dragged() || resp.clicked() {
                match (grab, target) {
                    (Some(g), _) => {
                        let v = (px - g).clamp(0.0, 472.0);
                        set_plain(ctx, P::Glass, v as f64);
                        st.glass_req.store((v * 100.0) as u32, Ordering::Relaxed);
                        st.glass_req_seq.fetch_add(1, Ordering::Release);
                    }
                    (None, Some(P::NeedleL)) => {
                        set_plain(ctx, P::NeedleL, px as f64);
                        if px > nr {
                            ctx.automate(P::NeedleR, normalized(ctx, P::NeedleR, px as f64));
                        }
                    }
                    (None, Some(_)) => {
                        set_plain(ctx, P::NeedleR, px as f64);
                        if px < nl {
                            ctx.automate(P::NeedleL, normalized(ctx, P::NeedleL, px as f64));
                        }
                    }
                    _ => {}
                }
            }
            if resp.drag_stopped() || resp.clicked() {
                match (grab, target) {
                    (Some(_), _) | (None, None) => ctx.end_edit(P::Glass),
                    (None, Some(t)) => ctx.end_edit(t),
                }
            }
        }
        resp.on_hover_text("Drag the Grain Glass; drag the red / yellow needles at the top edge");

        // Record button.
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            let rec = params.record.value();
            let (r, resp) = ui.allocate_exact_size(vec2(76.0, 76.0), Sense::click());
            let painter = ui.painter_at(r);
            let c = r.center();
            painter.circle_filled(c, 34.0, if rec { p.record } else { alpha(p.record, 0.75) });
            painter.circle_filled(c, 15.0, if rec { Color32::WHITE } else { alpha(Color32::WHITE, 0.85) });
            if rec {
                painter.circle_stroke(c, 37.0, Stroke::new(2.0_f32, p.record));
            }
            if resp.clicked() {
                ctx.automate(P::Record, if rec { 0.0 } else { 1.0 });
            }
            resp.on_hover_text("Record: stream the track into the screen; press again to freeze it");
            ui.label(egui::RichText::new("record").size(11.0).color(p.dim));
        });
    });

    // Walk.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, p, "walk");
        let walk = plain(ctx, P::Walk);
        if pill(ui, p, "−", false).clicked() {
            ctx.automate(P::Walk, normalized(ctx, P::Walk, (walk - 0.01).max(-5.0)));
        }
        let w = ui.available_width() - 120.0;
        hslider(ui, ctx, p, P::Walk, w, Some(&|painter: &egui::Painter, r: Rect| {
            let x = r.center().x;
            painter.line_segment([pos2(x, r.top() + 3.0), pos2(x, r.bottom() - 3.0)], Stroke::new(1.0_f32, Color32::from_gray(90)));
        }));
        if pill(ui, p, "+", false).clicked() {
            ctx.automate(P::Walk, normalized(ctx, P::Walk, (walk + 0.01).min(5.0)));
        }
        if pill(ui, p, "0", walk == 0.0).on_hover_text("Halt the Grain Glass").clicked() {
            ctx.automate(P::Walk, normalized(ctx, P::Walk, 0.0));
        }
    });
}

fn handle_dropped_files(ui: &egui::Ui, ctx: &Ctx) {
    let files: Vec<_> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if let Some(path) = files.into_iter().next() {
        load_path(ui, ctx.params().clone(), path);
    }
}

fn load_path(ui: &egui::Ui, params: Arc<RiverrunParams>, path: std::path::PathBuf) {
    let c = cache(ui);
    std::thread::spawn(move || match audio::load(&path) {
        Ok(d) => {
            let keep = ((d.sample_rate * 4.65).ceil() as usize + 2).min(d.samples.len());
            params.snippet.load(SnippetData { name: d.name, sample_rate: d.sample_rate, samples: d.samples[..keep].to_vec() });
            if let Ok(mut cg) = c.0.lock() {
                cg.message.clear();
            }
        }
        Err(e) => {
            if let Ok(mut cg) = c.0.lock() {
                cg.message = format!("could not load: {e}");
            }
        }
    });
}

// --- Shape, mono/stereo, files ------------------------------------------------

fn shape_icon(painter: &egui::Painter, r: Rect, which: usize, col: Color32) {
    let (l, rr, t, b) = (r.left() + 5.0, r.right() - 5.0, r.top() + 5.0, r.bottom() - 5.0);
    let w = rr - l;
    let pts: Vec<Pos2> = match which {
        0 => vec![pos2(l, b), pos2(l + w * 0.5, t), pos2(rr, b)],
        1 => vec![pos2(l, b), pos2(l + w * 0.08, t), pos2(rr - w * 0.08, t), pos2(rr, b)],
        2 => vec![pos2(l, b), pos2(l + w * 0.03, t), pos2(l + w * 0.07, b - (b - t) * 0.2), pos2(rr - w * 0.3, b - (b - t) * 0.2), pos2(rr, b)],
        _ => (0..=24)
            .map(|i| {
                let x = i as f32 / 24.0;
                pos2(l + x * w, b - (b - t) * (0.5 - 0.5 * (x * std::f32::consts::TAU).cos()))
            })
            .collect(),
    };
    painter.add(egui::Shape::line(pts, Stroke::new(1.8_f32, col)));
}

fn shape_row(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params().clone();
    ui.horizontal(|ui| {
        label(ui, p, "shape");
        let cur = params.envelope.index() as usize;
        for i in 0..4 {
            let (r, resp) = ui.allocate_exact_size(vec2(46.0, 26.0), Sense::click());
            let painter = ui.painter_at(r);
            let on = i == cur;
            painter.rect_filled(r, 3.0, if on { p.accent } else { Color32::from_rgb(12, 16, 16) });
            shape_icon(&painter, r, i, if on { p.accent_text } else { Color32::from_rgb(200, 230, 170) });
            if resp.clicked() {
                ctx.automate(P::Envelope, normalized(ctx, P::Envelope, i as f64));
            }
            resp.on_hover_text(tables::ENVELOPE_NAMES[i]);
        }
        ui.add_space(14.0);
        label(ui, p, "mono / stereo");
        hslider(ui, ctx, p, P::MonoStereo, 180.0, None);
        ui.add_space(14.0);
        if pill(ui, p, "load", false).on_hover_text("Load a WAV or AIFF file into the screen (first 4.65 s)").clicked() {
            let params = params.clone();
            let c = cache(ui);
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new().add_filter("Audio", &["wav", "aif", "aiff", "aifc"]).set_title("Load into Riverrun").pick_file() {
                    match audio::load(&path) {
                        Ok(d) => {
                            let keep = ((d.sample_rate * 4.65).ceil() as usize + 2).min(d.samples.len());
                            params.snippet.load(SnippetData { name: d.name, sample_rate: d.sample_rate, samples: d.samples[..keep].to_vec() });
                        }
                        Err(e) => {
                            if let Ok(mut cg) = c.0.lock() {
                                cg.message = format!("could not load: {e}");
                            }
                        }
                    }
                }
            });
        }
        if pill(ui, p, "save", false).on_hover_text("Save the screen as AIFF or WAV (24-bit)").clicked() {
            match params.snippet.snapshot() {
                Some(snap) => {
                    let c = cache(ui);
                    std::thread::spawn(move || {
                        if let Some(path) = rfd::FileDialog::new().add_filter("AIFF", &["aif", "aiff"]).add_filter("WAV", &["wav"]).set_file_name("Riverrun snippet.aif").save_file() {
                            let r = audio::save(&path, snap.sample_rate, 1, &snap.samples, audio::SampleFormat::Int24);
                            if let Ok(mut cg) = c.0.lock() {
                                cg.message = match r {
                                    Ok(()) => format!("saved {}", path.display()),
                                    Err(e) => format!("could not save: {e}"),
                                };
                            }
                        }
                    });
                }
                None => set_message(ui, "nothing to save yet"),
            }
        }
        egui::ComboBox::from_id_salt("rr-presets")
            .width(150.0)
            .selected_text(egui::RichText::new("demo & presets").size(11.5))
            .show_ui(ui, |ui| {
                if ui.selectable_label(false, "Voice — Audio Ease's demo, 2000").clicked() {
                    params.snippet.load(demo::voice());
                }
                if ui.selectable_label(false, "Sung phrase (synthetic)").clicked() {
                    params.snippet.load(demo::snippet(params.status.sr()));
                }
                ui.separator();
                ui.label(egui::RichText::new("from the Riverrun Demo song").size(10.0).color(p.dim));
                for pr in crate::presets::PRESETS.iter() {
                    if ui.selectable_label(false, pr.name).on_hover_text(pr.about).clicked() {
                        apply_preset(ctx, pr);
                    }
                }
            });
        if pill(ui, p, "clear", false).clicked() {
            params.status.clear_req.fetch_add(1, Ordering::Release);
            if let Ok(mut m) = params.snippet.mirror.lock() {
                m.name.clear();
            }
        }
    });
}

/// Loads the demo Voice and sets every control of a preset.
fn apply_preset(ctx: &Ctx, pr: &crate::presets::Preset) {
    let params = ctx.params();
    params.snippet.load(demo::voice_streamed());
    let set = |id: P, v: f64| ctx.automate(id, normalized(ctx, id, v));
    set(P::Record, 0.0);
    set(P::Pitch, pr.pitch as f64);
    set(P::Speed, pr.speed as f64);
    set(P::Length, pr.length as f64);
    set(P::Random, pr.random as f64);
    set(P::SpeedRandom, pr.speed_random as f64);
    set(P::Walk, pr.walk as f64);
    set(P::Envelope, (pr.envelope - 1) as f64);
    set(P::Grid, (pr.grid - 1) as f64);
    set(P::Mode, if pr.rhythmic { 1.0 } else { 0.0 });
    set(P::MonoStereo, pr.mono_stereo as f64);
    set(P::NeedleL, pr.needle_l as f64);
    set(P::NeedleR, pr.needle_r as f64);
    set(P::Fine, pr.fine as f64);
    set(P::Glass, pr.glass as f64);
    let st = &params.status;
    st.glass_req.store((pr.glass * 100.0) as u32, Ordering::Relaxed);
    st.glass_req_seq.fetch_add(1, Ordering::Release);
}

// --- Texture -----------------------------------------------------------------

fn texture_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    let rhythmic = params.mode.value() == GrainMode::Rhythmic;
    ui.horizontal(|ui| {
        label(ui, p, "TEXTURE");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pill(ui, p, "rhythmic (sync to beats)", rhythmic).clicked() {
                ctx.automate(P::Mode, normalized(ctx, P::Mode, 1.0));
            }
            if pill(ui, p, "flowing", !rhythmic).clicked() {
                ctx.automate(P::Mode, normalized(ctx, P::Mode, 0.0));
            }
        });
    });
    let w = ui.available_width();
    ui.add_space(6.0);
    label(ui, p, "grain speed");
    let grid = |painter: &egui::Painter, r: Rect| {
        // The black (even) and grey (triplet) fields of Rhythmic mode.
        for i in 0..14 {
            let v = i as f32 * 21.142857;
            let n0 = ((v - 10.5 - 1.0) / 285.0).clamp(0.0, 1.0);
            let n1 = ((v + 10.5 - 1.0) / 285.0).clamp(0.0, 1.0);
            let x0 = r.left() + 4.0 + n0 * (r.width() - 8.0);
            let x1 = r.left() + 4.0 + n1 * (r.width() - 8.0);
            let triplet = matches!(i, 1 | 3 | 5 | 7 | 9 | 11);
            let col = if triplet { Color32::from_gray(95) } else { Color32::from_gray(38) };
            painter.rect_filled(Rect::from_x_y_ranges((x0 + 1.0)..=(x1 - 1.0), (r.top() + 3.0)..=(r.bottom() - 3.0)), 1.0, col);
        }
    };
    let resp = hslider(ui, ctx, p, P::Speed, w, if rhythmic { Some(&grid) } else { None });
    if rhythmic && (resp.dragged() || resp.clicked()) && !ui.input(|i| i.modifiers.alt) {
        // The thumb sticks to the fields.
        let v = plain(ctx, P::Speed);
        let idx = ((v / 21.142_857_142_857_142) + 0.5).floor().clamp(0.0, 13.0);
        set_plain(ctx, P::Speed, (idx * 21.142_857_142_857_142).max(1.0));
    }
    ui.add_space(6.0);
    label(ui, p, "grain speed random");
    hslider(ui, ctx, p, P::SpeedRandom, w, None);
    ui.add_space(6.0);
    label(ui, p, "grain length");
    hslider(ui, ctx, p, P::Length, w, None);
    ui.add_space(8.0);
    // A small readout of the resulting texture.
    let speed = params.speed.value();
    let len_ms = breakpoint(to_short(params.length.value()), &tables::LENGTH_MAP) as i32 as f32 / 44.1;
    let layers = if rhythmic { None } else { Some(breakpoint(to_short(speed), &tables::FLOWING_MAP) * len_ms / 1000.0) };
    if let Some(l) = layers {
        ui.label(egui::RichText::new(format!("≈ {l:.0} overlapping grains")).size(10.5).color(p.dim));
    }
}

// --- Transposition -----------------------------------------------------------

fn transposition_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    label(ui, p, "TRANSPOSITION");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, p, "pitch grid");
        enum_combo::<crate::params::PitchGrid>(ui, ctx, P::Grid, params.grid.index() as usize, ui.available_width() - 8.0);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Pitch, "pitch");
        param_knob(ui, ctx, P::Random, "random");
        ui.vertical(|ui| {
            label(ui, p, "fine tune");
            ui.horizontal(|ui| {
                let f = plain(ctx, P::Fine);
                let cent = 2f64.powf(1.0 / 1200.0);
                if pill(ui, p, "−", false).clicked() {
                    ctx.automate(P::Fine, normalized(ctx, P::Fine, (f / cent).max(0.666666)));
                }
                if pill(ui, p, "0", (f - 1.0).abs() < 1e-6).clicked() {
                    ctx.automate(P::Fine, normalized(ctx, P::Fine, 1.0));
                }
                if pill(ui, p, "+", false).clicked() {
                    ctx.automate(P::Fine, normalized(ctx, P::Fine, (f * cent).min(1.5)));
                }
            });
            ui.label(egui::RichText::new(ctx.format_param(P::Fine)).size(11.0));
        });
    });
    ui.add_space(4.0);
    keyboard(ui, ctx, p);
}

/// The figure of manual page 7: the grid's notes in red, the band the dice
/// are thrown in (from Pitch, Random wide) in blue.
fn keyboard(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette) {
    let params = ctx.params();
    let c = cache(ui);
    let mut cg = c.0.lock().unwrap();
    let scales = cg.scales.get_or_insert_with(Scales::new).clone();
    drop(cg);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 46.0), Sense::hover());
    let painter = ui.painter_at(r);
    // −36 … +48 semitones, as wide as the grids go.
    let lo = -36i32;
    let hi = 48i32;
    let whites: Vec<i32> = (lo..=hi).filter(|n| matches!(n.rem_euclid(12), 0 | 2 | 4 | 5 | 7 | 9 | 11)).collect();
    let kw = r.width() / whites.len() as f32;
    let base = breakpoint(to_short(params.pitch.value()), &tables::PITCH_MAP) as f64;
    let range = breakpoint(to_short(params.random.value()), &tables::RANDOM_MAP) as f64;
    let st = |ratio: f64| 12.0 * ratio.max(1e-6).log2();
    let (b0, b1) = (st(base), st(base + range));
    let grid = params.grid.index() as usize;
    let in_grid = |n: i32| -> bool {
        if grid == 0 {
            return true;
        }
        let rr = tables::SEMITONE.powf(n as f64) as f32 as f64;
        scales.ratio[grid].iter().any(|&g| (g - rr).abs() < 1e-9)
    };
    let x_of = |semis: f64| -> f32 {
        let wi = whites.iter().filter(|&&w| (w as f64) < semis).count() as f32;
        r.left() + wi * kw
    };
    let band = Rect::from_x_y_ranges(x_of(b0)..=x_of(b1).max(x_of(b0) + 2.0), r.y_range());
    for (i, &n) in whites.iter().enumerate() {
        let k = Rect::from_min_size(pos2(r.left() + i as f32 * kw, r.top()), vec2(kw - 1.0, r.height()));
        let col = if grid != 0 && in_grid(n) { p.key_hit } else { Color32::from_gray(235) };
        painter.rect_filled(k, 1.0, col);
        if n == 0 {
            painter.circle_filled(pos2(k.center().x, k.bottom() - 6.0), 2.5, Color32::from_gray(60));
        }
    }
    for (i, &n) in whites.iter().enumerate() {
        if matches!(n.rem_euclid(12), 0 | 2 | 5 | 7 | 9) && n < hi {
            let x = r.left() + (i as f32 + 1.0) * kw;
            let k = Rect::from_center_size(pos2(x - 0.5, r.top() + r.height() * 0.31), vec2(kw * 0.62, r.height() * 0.62));
            let col = if grid != 0 && in_grid(n + 1) { p.key_hit } else { Color32::from_gray(20) };
            painter.rect_filled(k, 1.0, col);
        }
    }
    painter.rect_filled(band, 1.0, alpha(p.band, 0.35));
    painter.rect_stroke(band, 1.0, Stroke::new(1.5_f32, p.band), egui::StrokeKind::Inside);
}

// --- Output & modern extras --------------------------------------------------

fn output_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, classic: bool) {
    let params = ctx.params();
    let _ = classic;
    let text = p.dim;
    ui.label(egui::RichText::new("OUTPUT").size(11.0).strong().color(text));
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Mix, "mix");
        param_knob(ui, ctx, P::Output, "output");
        level_meter(ui, ctx, &[P::MeterL, P::MeterR], 70.0);
    });
    let agc = f32::from_bits(params.status.agc_gain.load(Ordering::Relaxed));
    ui.label(egui::RichText::new(format!("AGC {:+.1} dB", 20.0 * agc.max(1e-6).log10())).size(10.5).color(text))
        .on_hover_text("The original's automatic gain control (300-sample look-ahead)");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("grain clock").size(10.5).color(text));
        enum_combo::<crate::params::GrainClock>(ui, ctx, P::Clock, params.clock.index() as usize, 60.0);
    })
    .response
    .on_hover_text("The original made one grain per host buffer; this fixes that buffer (512 = the usual 2000s setting)");
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("beat sync").size(10.5).color(text));
        enum_combo::<crate::params::BeatSyncMode>(ui, ctx, P::BeatSync, params.beat_sync.index() as usize, 70.0);
    })
    .response
    .on_hover_text("Original: the 2000 timing, up to two buffers off the beat. Tight: on the host's grid");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the editor for the README.
    /// `cargo test --release -p riverrun --lib readme_screenshots -- --ignored`
    #[test]
    #[ignore]
    fn readme_screenshots() {
        let params = Arc::new(RiverrunParams::new());
        let d = demo::snippet(44100.0);
        if let Ok(mut m) = params.snippet.mirror.lock() {
            *m = d.clone();
        }
        params.snippet.bump_mirror();
        params.status.glass_px.store(18_000, Ordering::Relaxed);
        params.status.glass_w_px.store(1_100, Ordering::Relaxed);
        params.grid.set_index(1);
        params.random.set_value(55.0);
        params.pitch.set_value(420.0);
        let mut seq = 0u32;
        for i in 0..GRAIN_TRAIL {
            let pos = 0.38 + 0.03 * ((i * 7 % 11) as f32 / 11.0);
            let ratio = [0.5f32, 0.6, 0.75, 1.0, 1.19, 1.5][i % 6];
            params.status.grains[i].store(crate::params::pack_grain(pos, ratio, (i % 2) as u8), Ordering::Relaxed);
            seq += 1;
        }
        params.status.grain_seq.store(seq, Ordering::Relaxed);
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        std::fs::create_dir_all(&docs).unwrap();
        for (theme, file) in [(0u8, docs.join("riverrun.png")), (1, docs.join("riverrun-classic.png"))] {
            *params.theme.write().unwrap() = theme;
            let mut editor = crate::RiverrunPlugin::editor(params.clone());
            if let Some((px, w, h)) = editor.screenshot(params.clone()) {
                truce_test::save_png(&file, &px, w, h);
            }
        }
    }
}
