//! The editor, laid out like the 2000 original (interface by Captain Video):
//!
//! ┌───────────────────────────────────────────┬──────────────────┐
//! │ fader screen: 32 faders over the spectrum │ PERISCOPE ★★★    │
//! │ (red = low pass, yellow = high pass)      │ display release  │
//! │                                           │ magnetism        │
//! ├───────────────────────────────────────────┤ fader edit mode  │
//! │ overview: the whole spectrum, two needles │ brightness       │
//! └───────────────────────────────────────────┴──────────────────┘
//!
//! The spectroscope follows the original: the output of all channels summed
//! and scaled by 1/√channels ("DrawBuf"), a 4096-point FFT ("draw fft
//! size"), a green peak-type graph and a blue average graph whose speed is
//! the display release ("Curve Latency").

use crate::engine::{needle_hz, Controls, Filter, BANDS};
use crate::params::{fader_id, fader_label, hz_label, EditMode, PeriscopeParams, P, SCOPE};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use mission::fft::RealFft;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use truce::prelude::*;
use truce_core::editor::PluginContext;
use truce_egui::widgets::{level_meter, param_knob};

pub const WINDOW_W: u32 = 960;
pub const WINDOW_H: u32 = 620;
pub const MIN_W: u32 = 820;
pub const MIN_H: u32 = 540;
pub const MAX_W: u32 = 1900;
pub const MAX_H: u32 = 1300;

type Ctx = PluginContext<PeriscopeParams>;

/// Fader travel of the original screen: 263 pixels (`0x107`); the
/// parameter goes to 265, which only MIDI reaches (same gain as 263).
const TRAVEL: f32 = 263.0;
/// The analyser's FFT size ("draw fft size").
const DRAW_FFT: usize = 4096;

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    frame: Color32,
    side: Color32,
    screen: Color32,
    grid: Color32,
    dim: Color32,
    side_text: Color32,
    accent: Color32,
    accent_text: Color32,
    low: Color32,
    high: Color32,
    fader: Color32,
    peak: Color32,
    average: Color32,
    zero: Color32,
    header: Color32,
}

const MODERN: Palette = Palette {
    bg: Color32::from_rgb(26, 28, 32),
    frame: Color32::from_rgb(40, 44, 50),
    side: Color32::from_rgb(40, 44, 50),
    screen: Color32::from_rgb(8, 10, 12),
    grid: Color32::from_rgb(46, 52, 58),
    dim: Color32::from_rgb(128, 136, 144),
    side_text: Color32::from_rgb(214, 218, 222),
    accent: Color32::from_rgb(90, 210, 140),
    accent_text: Color32::from_rgb(8, 24, 14),
    low: Color32::from_rgb(235, 70, 60),
    high: Color32::from_rgb(250, 210, 60),
    fader: Color32::from_rgb(150, 156, 164),
    peak: Color32::from_rgb(70, 235, 110),
    average: Color32::from_rgb(70, 130, 255),
    zero: Color32::from_rgb(60, 200, 90),
    header: Color32::from_rgb(52, 56, 62),
};

/// The original: blue stripes, a brushed-metal frame around black screens,
/// a green side panel.
const CLASSIC: Palette = Palette {
    bg: Color32::from_rgb(24, 92, 200),
    frame: Color32::from_rgb(178, 182, 186),
    side: Color32::from_rgb(56, 168, 120),
    screen: Color32::from_rgb(0, 0, 0),
    grid: Color32::from_rgb(34, 40, 40),
    dim: Color32::from_rgb(60, 66, 70),
    side_text: Color32::from_rgb(12, 30, 20),
    accent: Color32::from_rgb(20, 20, 20),
    accent_text: Color32::from_rgb(240, 240, 240),
    low: Color32::from_rgb(220, 30, 30),
    high: Color32::from_rgb(250, 220, 40),
    fader: Color32::from_rgb(170, 170, 170),
    peak: Color32::from_rgb(40, 255, 60),
    average: Color32::from_rgb(40, 90, 255),
    zero: Color32::from_rgb(40, 220, 60),
    header: Color32::from_rgb(70, 72, 72),
};

fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = p.bg;
    v.window_fill = p.side;
    v.extreme_bg_color = p.screen;
    v.faint_bg_color = p.side;
    v.override_text_color = Some(p.side_text);
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
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.side_text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.side_text);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, p.side_text);
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
    ui.label(egui::RichText::new(text).size(11.0).color(p.side_text).strong());
}

// --- Editor state (lives in egui memory) ----------------------------------------

struct Analyser {
    fft: RealFft,
    buf: Vec<f32>,
    window: Vec<f32>,
    /// Power per bin (linear), instantaneous and averaged.
    peak: Vec<f32>,
    average: Vec<f32>,
    primed: bool,
}

impl Analyser {
    fn new() -> Self {
        let n = DRAW_FFT;
        Self {
            fft: RealFft::new(n),
            buf: vec![0.0; n],
            window: (0..n).map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos()) as f32).collect(),
            peak: vec![0.0; n / 2],
            average: vec![0.0; n / 2],
            primed: false,
        }
    }

    fn update(&mut self, params: &PeriscopeParams, release: f32) {
        let st = &params.status;
        let pos = st.scope_pos.load(Ordering::Relaxed);
        let n = DRAW_FFT;
        for i in 0..n {
            let k = (pos + SCOPE - n + i) % SCOPE;
            self.buf[i] = f32::from_bits(st.scope[k].load(Ordering::Relaxed)) * self.window[i];
        }
        self.fft.forward(&mut self.buf);
        // vDSP's forward FFT is 2·DFT; a full-scale sine under a Hann
        // window reads N/2 · 1/2 · 2 = N/2 in its bin.
        let norm = 1.0 / (n as f32 * 0.5);
        for k in 1..n / 2 {
            let (re, im) = (self.buf[2 * k] * norm, self.buf[2 * k + 1] * norm);
            let p = re * re + im * im;
            self.peak[k] = p;
            self.average[k] = if self.primed { release * self.average[k] + (1.0 - release) * p } else { p };
        }
        self.primed = true;
    }

    /// Level in dBFS at frequency `hz` (max over the bins it covers).
    fn level(v: &[f32], hz0: f64, hz1: f64, sr: f64) -> f32 {
        let bin = sr / DRAW_FFT as f64;
        let a = ((hz0 / bin).floor() as usize).clamp(1, v.len() - 1);
        let b = ((hz1 / bin).ceil() as usize).clamp(a, v.len() - 1);
        let m = v[a..=b].iter().copied().fold(0.0f32, f32::max);
        10.0 * m.max(1e-14).log10()
    }
}

struct Drag {
    /// Fader values when the button went down (`local_68`).
    orig: [i64; BANDS],
    /// The fader first clicked (Amplify / Exaggerate scale by it).
    first: usize,
    /// Last fader and value touched, for Free draw's interpolation.
    last: Option<(usize, i64)>,
}

struct State {
    analyser: Analyser,
    filter: Filter,
    filter_key: Option<Controls>,
    response: Vec<f32>,
    drag: Option<Drag>,
    needle_drag: Option<usize>,
    /// The fader whose label pops up, and until when (3 s after release).
    label: Option<(usize, Instant)>,
    pending_seen: u32,
    last_frame: Instant,
}

impl State {
    fn new(sr: f64) -> Self {
        Self {
            analyser: Analyser::new(),
            filter: Filter::new(sr),
            filter_key: None,
            response: Vec::new(),
            drag: None,
            needle_drag: None,
            label: None,
            pending_seen: 0,
            last_frame: Instant::now(),
        }
    }
}

type Shared = Arc<Mutex<State>>;

// --- Geometry ------------------------------------------------------------------

/// Band edges as the fader screen shows them: x = 0 … 32 across the faders,
/// fader k spanning [k, k + 1]; frequencies in between interpolate in log.
struct Bands {
    edges: [f64; BANDS + 1],
}

impl Bands {
    fn new(freqs: &[f32; BANDS]) -> Self {
        let mut edges = [0.0f64; BANDS + 1];
        // Fader 0 is the low pass: from an octave… below the first edge.
        let f0 = freqs[0].max(1.0) as f64;
        let f1 = freqs[1].max(freqs[0] + 1.0) as f64;
        edges[0] = (f0 * f0 / f1).max(f0 * 0.25).max(1.0);
        for k in 0..BANDS {
            edges[k + 1] = (freqs[k] as f64).max(edges[k] * 1.0001);
        }
        Self { edges }
    }

    fn hz_of(&self, x: f64) -> f64 {
        let k = (x.floor() as usize).min(BANDS - 1);
        let t = x - k as f64;
        self.edges[k] * (self.edges[k + 1] / self.edges[k]).powf(t)
    }

    fn centre(&self, k: usize) -> f64 {
        (self.edges[k] * self.edges[k + 1]).sqrt()
    }
}

/// Spectrum level → fader units, 2.5 units per dB like the fader curve
/// above −48 dB: −108 dBFS at the bottom, 0 dBFS at the top.
fn level_units(db: f32) -> f32 {
    135.0 + 2.5 * (db + 54.0)
}

// --- Editor ----------------------------------------------------------------------

pub fn periscope_ui(ui: &mut egui::Ui, ctx: &Ctx) {
    let params: &Arc<PeriscopeParams> = ctx.params();
    let classic = params.theme.read().map(|t| *t == 1).unwrap_or(false);
    let p = if classic { CLASSIC } else { MODERN };
    let v = visuals(&p);
    ui.ctx().set_visuals(v.clone());
    ui.style_mut().visuals = v;

    let sr = params.sample_rate();
    let id = egui::Id::new("periscope-state");
    let shared: Shared = ui.data_mut(|d| d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::new(sr)))).clone());
    let mut state = shared.lock().unwrap();
    commit_pending(ctx, &mut state);
    let now = Instant::now();
    let dt = now.duration_since(state.last_frame).as_secs_f32().clamp(0.0, 0.5);
    state.last_frame = now;
    // "Curve Latency" per 1/30 s frame, made frame-rate independent.
    let release = params.release.value().clamp(0.0, 0.999).powf(dt * 30.0);
    state.analyser.update(params, release);

    let full = ui.max_rect();
    if classic {
        let painter = ui.painter();
        painter.rect_filled(full, 0.0, p.bg);
        let mut y = full.top();
        while y < full.bottom() {
            painter.rect_filled(Rect::from_min_size(pos2(full.left(), y), vec2(full.width(), 2.0)), 0.0, Color32::from_rgb(10, 50, 150));
            y += 4.0;
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(12)))
        .show_inside(ui, |ui| {
            let h = ui.available_height();
            ui.horizontal_top(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let side = 200.0;
                let w = ui.available_width() - side - gap;
                ui.vertical(|ui| {
                    egui::Frame::NONE.fill(p.frame).corner_radius(5.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_min_size(vec2(w - 20.0, h - 20.0));
                        ui.set_max_size(vec2(w - 20.0, h - 20.0));
                        let top = ((h - 20.0) * 0.73).floor();
                        fader_screen(ui, ctx, &p, &mut state, vec2(w - 20.0, top));
                        ui.add_space(8.0);
                        let rest = ui.available_height();
                        overview(ui, ctx, &p, &mut state, vec2(w - 20.0, rest));
                    });
                });
                egui::Frame::NONE.fill(p.side).corner_radius(5.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_min_size(vec2(side - 20.0, h - 20.0));
                    ui.set_max_size(vec2(side - 20.0, h - 20.0));
                    ui.vertical(|ui| side_panel(ui, ctx, &p, classic));
                });
            });
        });
    drop(state);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// Values the audio thread holds that the host doesn't know yet (needles
/// pushed apart by `UpdateFreqs`, MIDI): write them into the parameters, as
/// the original's `SetParamFloat` sends them to the host.
fn commit_pending(ctx: &Ctx, state: &mut State) {
    let st = &ctx.params().status;
    let seq = st.pending.load(Ordering::Relaxed);
    if seq == state.pending_seen || state.drag.is_some() || state.needle_drag.is_some() {
        return;
    }
    state.pending_seen = seq;
    let params = ctx.params();
    let lo = f32::from_bits(st.low.load(Ordering::Relaxed));
    let hi = f32::from_bits(st.high.load(Ordering::Relaxed));
    if lo != params.low.value() {
        ctx.automate(P::Low, normalized(ctx, P::Low, lo as f64));
    }
    if hi != params.high.value() {
        ctx.automate(P::High, normalized(ctx, P::High, hi as f64));
    }
    for k in 0..BANDS {
        let v = st.faders[k].load(Ordering::Relaxed) as i64;
        if v != params.fader(k).value() {
            ctx.automate(fader_id(k), normalized(ctx, fader_id(k), v as f64));
        }
    }
}

/// The edit mode in force: the one clicked, or the one a modifier selects
/// (`PeriscopeEditMode::Idle` @ 0x19d40).
fn edit_mode(ui: &egui::Ui, params: &PeriscopeParams) -> usize {
    let m = ui.input(|i| i.modifiers);
    if m.mac_cmd {
        if m.shift { 4 } else { 0 }
    } else if m.ctrl {
        1
    } else if m.alt {
        2
    } else if m.shift {
        3
    } else {
        params.edit_mode.index() as usize
    }
}

fn fader_screen(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, state: &mut State, size: egui::Vec2) {
    let params = ctx.params().clone();
    let st = &params.status;
    let sr = params.sample_rate();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, p.screen);
    let inner = rect.shrink2(vec2(6.0, 8.0));
    let fw = inner.width() / BANDS as f32;
    let yv = |v: f32| inner.bottom() - (v / TRAVEL).clamp(0.0, 1.03) * inner.height();
    let bright = params.brightness.value();

    let freqs: [f32; BANDS] = std::array::from_fn(|k| f32::from_bits(st.freqs[k].load(Ordering::Relaxed)));
    let bands = Bands::new(&freqs);
    let faders: [i64; BANDS] = std::array::from_fn(|k| st.faders[k].load(Ordering::Relaxed) as i64);

    // Spectrum behind the faders: blue average (filled), green peak (line).
    let steps = (inner.width() as usize).max(32);
    let graph = |v: &[f32]| -> Vec<Pos2> {
        (0..=steps)
            .map(|i| {
                let x0 = i as f64 / steps as f64 * BANDS as f64;
                let x1 = (i + 1) as f64 / steps as f64 * BANDS as f64;
                let db = Analyser::level(v, bands.hz_of(x0), bands.hz_of(x1.min(BANDS as f64)), sr);
                pos2(inner.left() + (x0 as f32) * fw, yv(level_units(db)))
            })
            .collect()
    };
    let avg = graph(&state.analyser.average);
    for w in avg.windows(2) {
        let quad = vec![w[0], w[1], pos2(w[1].x, inner.bottom()), pos2(w[0].x, inner.bottom())];
        painter.add(egui::Shape::convex_polygon(quad, alpha(p.average, 0.35), Stroke::NONE));
    }
    painter.add(egui::Shape::line(avg, Stroke::new(1.2_f32, alpha(p.average, 0.9))));
    painter.add(egui::Shape::line(graph(&state.analyser.peak), Stroke::new(1.0_f32, alpha(p.peak, 0.25 + 0.75 * bright))));

    // The 0 dB line (the thin green line).
    let y0 = yv(135.0);
    painter.line_segment([pos2(inner.left(), y0), pos2(inner.right(), y0)], Stroke::new(1.0_f32, alpha(p.zero, 0.3 + 0.6 * bright)));

    // The faders.
    for k in 0..BANDS {
        let x = inner.left() + (k as f32 + 0.5) * fw;
        let col = if k == 0 { p.low } else if k == BANDS - 1 { p.high } else { p.fader };
        let y = yv(faders[k] as f32);
        painter.line_segment([pos2(x, inner.bottom()), pos2(x, y)], Stroke::new((fw * 0.18).clamp(1.5, 4.0), alpha(col, 0.55)));
        let cap = Rect::from_center_size(pos2(x, y), vec2((fw * 0.7).max(6.0), 6.0));
        painter.rect_filled(cap, 1.5, col);
    }

    // The equaliser's actual response (thin white), recomputed when it changes.
    let key = Controls {
        low: f32::from_bits(st.low.load(Ordering::Relaxed)),
        high: f32::from_bits(st.high.load(Ordering::Relaxed)),
        faders: std::array::from_fn(|k| faders[k] as i32),
    };
    if state.filter_key != Some(key) {
        let mut c = key;
        state.filter = Filter::new(sr);
        state.filter.update(&mut c);
        state.filter_key = Some(key);
        state.response = (0..=steps)
            .map(|i| {
                let hz = bands.hz_of(i as f64 / steps as f64 * BANDS as f64);
                20.0 * state.filter.gain_at(hz as f32).max(1e-9).log10()
            })
            .collect();
    }
    if state.response.len() == steps + 1 {
        let pts: Vec<Pos2> = state
            .response
            .iter()
            .enumerate()
            .map(|(i, db)| pos2(inner.left() + i as f32 / steps as f32 * inner.width(), yv(db_to_units(*db))))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.0_f32, alpha(Color32::WHITE, 0.45))));
    }

    // Dragging (`PeriscopeControl::ClickInFaders` @ 0x16d50).
    let mode = edit_mode(ui, &params);
    if resp.drag_started() || (resp.is_pointer_button_down_on() && state.drag.is_none()) {
        if let Some(pt) = resp.interact_pointer_pos() {
            let j = (((pt.x - inner.left()) / fw).floor() as i64).clamp(0, BANDS as i64 - 1) as usize;
            for k in 0..BANDS {
                ctx.begin_edit(fader_id(k));
            }
            state.drag = Some(Drag { orig: faders, first: j, last: None });
        }
    }
    if let (Some(drag), Some(pt)) = (state.drag.as_mut(), resp.interact_pointer_pos()) {
        let j = (((pt.x - inner.left()) / fw).floor() as i64).clamp(0, BANDS as i64 - 1) as usize;
        let v = (TRAVEL * (inner.bottom() - pt.y) / inner.height()).round().clamp(0.0, TRAVEL) as i64;
        let set = |k: usize, val: i64| ctx.set_param(fader_id(k), normalized(ctx, fader_id(k), val.clamp(0, 265) as f64));
        let o = drag.orig;
        match mode {
            1 => (0..BANDS).for_each(|k| set(k, v.max(0))),
            2 => {
                // Magnetic: a raised cosine `magnetism` faders wide, with
                // the original's π of 3.1415926528.
                let g = params.magnetism.value() as f64;
                let delta = (v as f32 - o[j] as f32) as f64;
                for k in 0..BANDS {
                    let d = (k as f64 - j as f64).abs();
                    let val = if d < g {
                        ((((d * 3.1415926528) / g).cos() * 0.5 + 0.5) * delta + o[k] as f64) as i64
                    } else {
                        o[k]
                    };
                    set(k, val);
                }
            }
            3 => {
                let delta = v - o[drag.first];
                (0..BANDS).for_each(|k| set(k, (o[k] as f32 + delta as f32) as i64));
            }
            4 => {
                if o[drag.first] > 0 {
                    let f = v as f32 / o[drag.first] as f32;
                    (0..BANDS).for_each(|k| set(k, (o[k] as f32 * f) as i64));
                }
            }
            _ => {
                set(j, v);
                if let Some((lj, lv)) = drag.last {
                    let span = j as i64 - lj as i64;
                    if span.abs() > 1 {
                        let step = (v - lv) as f32 / span as f32;
                        let mut acc = lv as f32;
                        let dir = span.signum();
                        let mut k = lj as i64 + dir;
                        while k != j as i64 {
                            acc += step;
                            set(k as usize, acc as i64);
                            k += dir;
                        }
                    }
                }
            }
        }
        drag.last = Some((j, v));
        state.label = Some((j, Instant::now()));
    }
    if state.drag.is_some() && (resp.drag_stopped() || !resp.is_pointer_button_down_on()) {
        for k in 0..BANDS {
            ctx.end_edit(fader_id(k));
        }
        state.drag = None;
        if let Some((j, _)) = state.label {
            state.label = Some((j, Instant::now()));
        }
    }

    // The popup label: centre frequency and gain, fading 3 s after release.
    if let Some((j, t)) = state.label {
        let age = if state.drag.is_some() { 0.0 } else { t.elapsed().as_secs_f32() };
        if age < 3.0 {
            let a = (3.0 - age).min(1.0);
            let x = inner.left() + (j as f32 + 0.5) * fw;
            let y = yv(faders[j] as f32) - 10.0;
            let hz = match j {
                0 => format!("< {}", hz_label(freqs[0] as f64)),
                k if k == BANDS - 1 => format!("> {}", hz_label(freqs[BANDS - 2] as f64)),
                k => hz_label(bands.centre(k)),
            };
            let text = format!("{hz}\n{}", fader_label(faders[j]));
            let anchor = if x > inner.center().x { Align2::RIGHT_BOTTOM } else { Align2::LEFT_BOTTOM };
            let galley = painter.layout_no_wrap(text.clone(), FontId::monospace(10.5), alpha(Color32::WHITE, a));
            let r = anchor.anchor_size(pos2(x, y.max(inner.top() + 30.0)), galley.size()).expand(4.0);
            painter.rect_filled(r, 3.0, alpha(Color32::from_rgb(20, 24, 28), 0.85 * a));
            painter.galley(r.min + vec2(4.0, 4.0), galley, Color32::WHITE);
        }
    }
    resp.on_hover_text(match mode {
        1 => "Flat: all faders in a horizontal line (ctrl)",
        2 => "Magnetic: pull neighbouring faders along; width = magnetism (option)",
        3 => "Amplify: move the whole curve up or down (shift)",
        4 => "Exaggerate: scale every fader by the same factor (⌘ shift)",
        _ => "Free draw: drag across the faders (⌘)",
    });
}

/// Gain in dB → fader units (inverse of the fader curve).
pub fn db_to_units(db: f32) -> f32 {
    use crate::engine::FADER_CURVE;
    let g = 10f32.powf(db / 20.0);
    if g <= 0.0 {
        return 0.0;
    }
    for s in (2..FADER_CURVE.len()).step_by(2) {
        let (x0, g0, x1, g1) = (FADER_CURVE[s - 2], FADER_CURVE[s - 1], FADER_CURVE[s], FADER_CURVE[s + 1]);
        if g <= g1 {
            return x0 + (g - g0) / (g1 - g0) * (x1 - x0);
        }
    }
    265.0
}

fn overview(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, state: &mut State, size: egui::Vec2) {
    let params = ctx.params().clone();
    let st = &params.status;
    let sr = params.sample_rate();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, p.screen);
    let inner = rect.shrink2(vec2(8.0, 6.0));
    // x is linear in needle units: 0 = 18 Hz … 418 = Nyquist.
    let xp = |pos: f32| inner.left() + pos / 418.0 * inner.width();
    let posx = |x: f32| (x - inner.left()) / inner.width() * 418.0;
    for f in [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0] {
        if f >= sr * 0.5 {
            break;
        }
        let x = xp(crate::engine::hz_needle(f as f32, sr));
        painter.line_segment([pos2(x, inner.top()), pos2(x, inner.bottom())], Stroke::new(1.0_f32, alpha(p.grid, 0.8)));
        let t = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        painter.text(pos2(x + 2.0, inner.bottom()), Align2::LEFT_BOTTOM, t, FontId::proportional(9.0), alpha(Color32::GRAY, 0.8));
    }
    let lo = f32::from_bits(st.low.load(Ordering::Relaxed));
    let hi = f32::from_bits(st.high.load(Ordering::Relaxed));
    painter.rect_filled(Rect::from_x_y_ranges(xp(lo)..=xp(hi.max(lo)), inner.y_range()), 0.0, alpha(Color32::WHITE, 0.05));

    let steps = (inner.width() as usize).max(32);
    let y = |db: f32| inner.bottom() - ((db + 108.0) / 108.0).clamp(0.0, 1.0) * inner.height();
    let line = |v: &[f32]| -> Vec<Pos2> {
        (0..=steps)
            .map(|i| {
                let p0 = i as f32 / steps as f32 * 418.0;
                let p1 = (i + 1) as f32 / steps as f32 * 418.0;
                let db = Analyser::level(v, needle_hz(p0, sr) as f64, needle_hz(p1, sr) as f64, sr);
                pos2(xp(p0), y(db))
            })
            .collect()
    };
    painter.add(egui::Shape::line(line(&state.analyser.average), Stroke::new(1.2_f32, alpha(p.average, 0.9))));
    painter.add(egui::Shape::line(line(&state.analyser.peak), Stroke::new(1.0_f32, alpha(p.peak, 0.25 + 0.75 * params.brightness.value()))));

    for (pos, col, name) in [(lo, p.low, "low"), (hi, p.high, "high")] {
        let x = xp(pos);
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0_f32, col));
        let tri = vec![pos2(x - 6.0, rect.top()), pos2(x + 6.0, rect.top()), pos2(x, rect.top() + 9.0)];
        painter.add(egui::Shape::convex_polygon(tri, col, Stroke::NONE));
        if state.needle_drag.is_some() || resp.hovered() {
            let anchor = if name == "low" { Align2::RIGHT_TOP } else { Align2::LEFT_TOP };
            let dx = if name == "low" { -5.0 } else { 5.0 };
            painter.text(pos2(x + dx, rect.top() + 10.0), anchor, hz_label(needle_hz(pos, sr) as f64), FontId::monospace(10.0), col);
        }
    }

    if let Some(pt) = resp.interact_pointer_pos() {
        if state.needle_drag.is_none() && (resp.drag_started() || resp.is_pointer_button_down_on()) {
            let which = if (pt.x - xp(lo)).abs() <= (pt.x - xp(hi)).abs() { 0 } else { 1 };
            ctx.begin_edit(if which == 0 { P::Low } else { P::High });
            state.needle_drag = Some(which);
        }
        if let Some(which) = state.needle_drag {
            let id = if which == 0 { P::Low } else { P::High };
            ctx.set_param(id, normalized(ctx, id, posx(pt.x) as f64));
        }
    }
    if let Some(which) = state.needle_drag {
        if resp.drag_stopped() || !resp.is_pointer_button_down_on() {
            ctx.end_edit(if which == 0 { P::Low } else { P::High });
            state.needle_drag = None;
        }
    }
    resp.on_hover_text("Drag the red (low) and yellow (high) needles: the fader screen shows and processes what lies between them");
}

fn side_panel(ui: &mut egui::Ui, ctx: &Ctx, p: &Palette, classic: bool) {
    let params = ctx.params();
    ui.label(egui::RichText::new("PERISCOPE").size(24.0).strong().italics().color(p.side_text));
    ui.label(egui::RichText::new("★★★  3K").size(13.0).strong().color(p.side_text));
    ui.label(egui::RichText::new("32-band phase-correct zoomable EQ · nautilus3000").size(9.5).color(alpha(p.side_text, 0.75)));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Release, "release");
        param_knob(ui, ctx, P::Magnetism, "magnetism");
    });
    ui.add_space(4.0);
    label(ui, p, "fader edit mode");
    let current = edit_mode(ui, params);
    let keys = ["⌘", "ctrl", "opt", "shift", "⌘ shift"];
    for (i, name) in EditMode::variant_names().iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(44.0, 18.0), egui::Label::new(egui::RichText::new(keys[i]).size(10.5).color(alpha(p.side_text, 0.8))));
            let on = i == current;
            let (fill, fg) = if on { (p.accent, p.accent_text) } else { (alpha(Color32::BLACK, 0.2), p.side_text) };
            let b = egui::Button::new(egui::RichText::new(name.to_lowercase()).size(11.0).color(fg)).fill(fill).min_size(vec2(110.0, 18.0));
            if ui.add(b).clicked() {
                ctx.automate(P::EditMode, normalized(ctx, P::EditMode, i as f64));
            }
        });
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        param_knob(ui, ctx, P::Brightness, "brightness");
        level_meter(ui, ctx, &[P::MeterL, P::MeterR], 70.0);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new("flat").size(11.0)).on_hover_text("All faders to 0 dB").clicked() {
            for k in 0..BANDS {
                ctx.automate(fader_id(k), normalized(ctx, fader_id(k), 135.0));
            }
        }
        if ui.button(egui::RichText::new(if classic { "classic" } else { "modern" }).size(11.0)).on_hover_text("Theme").clicked() {
            if let Ok(mut t) = params.theme.write() {
                *t = if classic { 0 } else { 1 };
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test --release -p periscope --lib readme_screenshots -- --ignored`
    #[test]
    #[ignore]
    fn readme_screenshots() {
        let params = Arc::new(PeriscopeParams::new());
        let curve = [
            60, 110, 150, 170, 175, 165, 150, 140, 135, 120, 90, 40, 0, 40, 95, 130, 140, 150, 160, 180, 200, 190, 170, 150, 140,
            135, 135, 130, 120, 110, 100, 80,
        ];
        let mut c = Controls { low: 60.0, high: 360.0, faders: curve };
        let mut f = Filter::new(44100.0);
        f.update(&mut c);
        let st = &params.status;
        st.low.store(60.0f32.to_bits(), Ordering::Relaxed);
        st.high.store(360.0f32.to_bits(), Ordering::Relaxed);
        for k in 0..BANDS {
            params.fader(k).set_value(curve[k] as i64);
            st.faders[k].store(curve[k] as u32, Ordering::Relaxed);
            st.freqs[k].store(f.freqs[k].to_bits(), Ordering::Relaxed);
        }
        // Pink-ish noise plus a few tones in the scope.
        let mut seed = 1u32;
        let mut pink = 0.0f32;
        for i in 0..SCOPE {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let w = (seed >> 9) as f32 / (1 << 23) as f32 - 0.5;
            pink = 0.97 * pink + 0.1 * w;
            let t = i as f32 / 44100.0;
            let s = pink + 0.2 * (t * 220.0 * std::f32::consts::TAU).sin() + 0.05 * (t * 3150.0 * std::f32::consts::TAU).sin();
            st.scope[i].store(s.to_bits(), Ordering::Relaxed);
        }
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        std::fs::create_dir_all(&docs).unwrap();
        for (theme, file) in [(0u8, docs.join("periscope.png")), (1, docs.join("periscope-classic.png"))] {
            *params.theme.write().unwrap() = theme;
            let mut editor = crate::PeriscopePlugin::editor(params.clone());
            if let Some((px, w, h)) = editor.screenshot(params.clone()) {
                truce_test::save_png(&file, &px, w, h);
            }
        }
    }
}
