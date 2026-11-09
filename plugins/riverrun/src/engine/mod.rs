//! The Riverrun engine, translated from the i386 slice of `Riverrun.vst` 2.6.4.
//!
//! The original splits the work over three objects; their fields are kept here
//! under readable names, with the original offsets in the comments:
//!
//! * the process (`gyaywgjoxfgohfmpwutfuydpbiggdyv`): parameters → engine,
//!   walking Grain Glass, AGC, mono/stereo mix (`Process` @ 0x204c6);
//! * the engine (`ptoqcayexipwcepyhzpwknuemzcseph`): snippet, output buses,
//!   envelopes, pitch grids;
//! * the grain generator (`pdrabnexyxetetrmuacqttvvpynpown`): `Run` @ 0x1adda.
//!
//! `process_block(n)` is one call of the original `Process(in, out, n)`.

pub mod agc;
pub mod envelope;
pub mod ring;
pub mod tables;

use agc::Agc;
use envelope::{d2u, u2f};
use ring::Ring;
use tables::*;

/// Length of each output bus ring (`engine + 0x28 = 100000` in `InitBuffers`).
pub const BUS_LEN: u32 = 100_000;
/// The input screen is 472 pixels wide; needles and the Grain Glass live in
/// these units (`SetOverviewLength(0x1d8)`).
pub const SCREEN_PX: f32 = 472.0;
/// 2^-32 (`0x2f800000`), turns the LCG state into [0, 1).
const TWO_M32: f32 = f32::from_bits(0x2f80_0000);

/// The generator's random numbers: `x = x * 1664525 + 1013904223` (the
/// "quick and dirty" LCG of Numerical Recipes), inlined in `Run`.
#[inline]
pub fn lcg(x: u32) -> u32 {
    x.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f)
}

/// The fifteen automatable controls, in the original's own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// Param 0, 0…1000 (500 = original pitch).
    pub pitch: f32,
    /// Param 1, 1…286.
    pub speed: f32,
    /// Param 2.
    pub record: bool,
    /// Param 3, 1…286.
    pub length: f32,
    /// Param 4, 0…100.
    pub random: f32,
    /// Param 5, 0…286.
    pub speed_random: f32,
    /// Param 6, −5…5 (× real time).
    pub walk: f32,
    /// Param 7, 1…4.
    pub envelope: i32,
    /// Param 8, 1…23.
    pub grid: i32,
    /// Param 9.
    pub rhythmic: bool,
    /// Param 10, 0 (mono) … 93 (stereo).
    pub mono_stereo: f32,
    /// Param 13, 0…472.
    pub needle_l: f32,
    /// Param 14, 0…472.
    pub needle_r: f32,
    /// Param 15, 0.666666…1.5.
    pub fine: f32,
}

impl Default for Controls {
    /// The defaults from `tgxemus…::GetParameter` @ 0x1d618.
    fn default() -> Self {
        Self {
            pitch: 500.0,
            speed: 125.0,
            record: false,
            length: 100.0,
            random: 0.0,
            speed_random: 50.0,
            walk: 0.5,
            envelope: 2,
            grid: 1,
            rhythmic: false,
            mono_stereo: 93.0,
            needle_l: 20.0,
            needle_r: 270.0,
            fine: 1.0,
        }
    }
}

/// What the host says about time at the start of a block.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostTime {
    pub playing: bool,
    /// Tempo in BPM, 0 if unknown.
    pub bpm: f64,
    /// Position in quarter notes.
    pub ppq: f64,
    /// Position in samples (used to detect jumps).
    pub sample_pos: i64,
    /// The transport started or stopped since the previous block
    /// (`kVstTransportChanged`).
    pub transport_changed: bool,
}

/// How Rhythmic mode finds the beat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BeatSync {
    /// What the original does: when it (re)synchronises, it waits for a block
    /// whose start lies less than one block after a quarter note and places the
    /// first grain at `block start + elapsed part of the beat`. Grains therefore
    /// land up to two buffers late, by an amount that changes at every restart.
    #[default]
    Original,
    /// Grains on the division grid of the host's timeline.
    Tight,
}

/// One grain as reported to the editor.
#[derive(Clone, Copy, Debug, Default)]
pub struct GrainInfo {
    /// Read position, 0…1 of the input screen.
    pub pos: f32,
    /// Playback ratio.
    pub ratio: f32,
    /// Output bus (0 = left, 1 = right).
    pub bus: u8,
}

pub struct Engine {
    // ---- process (gyayw) ----
    sr: f64,
    snap: u32,
    max_grain: u32,
    walk_samples: i32,  // +0x84
    needle_l: u32,      // +0x88
    needle_r: u32,      // +0x8c
    env_choice: i32,    // +0x90
    spq: f64,           // +0xa0 samples per quarter
    bpm: f64,           // +0xa8
    last_speed: f32,    // +0xcc
    was_sync: bool,     // +0xd4
    /// Grain Glass left edge in screen pixels, +0xd8.
    pub glass: f32,
    last_host_pos: i64, // +0xdc
    stereo_k: f32,
    /// See [`BeatSync`].
    pub beat_sync: BeatSync,
    agc: Agc,
    rec_tmp: Vec<f32>,

    // ---- engine (ptoqca) ----
    out_pos: i32,           // +0x18
    grain: Vec<f32>,        // +0x1c
    rng: u32,               // +0x20
    bus_len: u32,           // +0x28
    pub snippet: Ring,      // +0x2c
    buses: [Ring; 2],       // +0x30
    sync: bool,             // +0x3c
    ticks_pos: f64,         // +0x40
    ticks_per_block: f64,   // +0x48
    resync: bool,           // +0x50
    playing: bool,          // +0x51
    shape: i16,             // +0x52
    attack: f64,            // +0x54
    decay: f64,             // +0x5c
    env: Vec<f32>,          // +0x64
    master: Vec<f32>,       // +0x68
    env_len: u32,           // +0x6c
    env_dirty: bool,        // +0x70
    glass_pos: f64,         // +0x74 (samples from the oldest sample)
    glass_end: i32,         // +0x7c
    scales: Scales,         // +0x80
    grid: i16,              // +0x5e7c
    spq_engine: f64,        // +0x5e80

    // ---- generator (pdrab) ----
    g_pitch: f64,        // +0x1c
    g_range: f64,        // +0x24
    g_fine: f64,         // +0x2c
    g_len: f64,          // +0x40
    g_jitter: i32,       // +0x48
    t: f64,              // +0x4c next grain onset (bus coordinates)
    mode: i32,           // +0x54
    interval: f64,       // +0x58
    new_interval: f64,   // +0x60
    last_t: f64,         // +0x68

    // ---- for the editor ----
    /// Grain Glass width in pixels (param 12 in the original).
    pub glass_width_px: i32,
    /// Grains started in the last block (bounded).
    pub recent: Vec<GrainInfo>,
    /// Grains started in total.
    pub grain_count: u64,
}

impl Engine {
    /// `InitBuffers` @ 0x2013c + `ptoqca…::Init` @ 0x1c020.
    pub fn new(sample_rate: f64) -> Self {
        let sr = sample_rate;
        let snap = d2u(sr * 4.65); // gSnapShotSize
        let max_grain = d2u(sr); // gMaxGrainLength
        let shape_len = d2u(sr); // gShapeLength
        // Above 48 kHz a 100000-sample bus is shorter than the longest grain
        // plus the longest jitter; the original then overwrote its own output.
        let need = max_grain as f64 + 30000.0 * sr / 44100.0 + 8192.0;
        let bus_len = if need > BUS_LEN as f64 { need.ceil() as u32 } else { BUS_LEN };

        let mut e = Self {
            sr,
            snap,
            max_grain,
            walk_samples: 0,
            needle_l: 0,
            needle_r: snap,
            env_choice: -1,
            spq: sr * 0.5,
            bpm: 0.0,
            last_speed: -1.0,
            was_sync: true,
            glass: 150.0,
            last_host_pos: 0,
            stereo_k: 0.0,
            beat_sync: BeatSync::Original,
            agc: Agc::new(sr),
            rec_tmp: vec![0.0; 8192],

            out_pos: 0,
            grain: vec![0.0; max_grain as usize + 100],
            rng: 0,
            bus_len,
            snippet: Ring::new(snap as usize),
            buses: [Ring::new(bus_len as usize), Ring::new(bus_len as usize)],
            sync: true,
            ticks_pos: 0.0,
            ticks_per_block: 0.0,
            resync: true,
            playing: false,
            shape: envelope::SHAPE_SHARP,
            attack: 0.1,
            decay: 0.8,
            env: vec![0.0; max_grain as usize + 100],
            master: vec![0.0; shape_len as usize],
            env_len: u32::MAX,
            env_dirty: true,
            glass_pos: snap as f64 * 0.5,
            glass_end: (snap as f64 * 0.5) as i32,
            scales: Scales::new(),
            grid: 1,
            spq_engine: 0.0,

            g_pitch: 1.0,
            g_range: 0.0,
            g_fine: 0.0,
            g_len: 500.0,
            g_jitter: 1,
            t: 0.0,
            mode: 2,
            interval: 500.0,
            new_interval: 500.0,
            last_t: 0.0,

            glass_width_px: 0,
            recent: Vec::with_capacity(64),
            grain_count: 0,
        };
        envelope::init_master(&mut e.master, e.shape, e.attack, e.decay);
        e
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }
    pub fn snapshot_len(&self) -> usize {
        self.snap as usize
    }
    pub fn needles_px(&self) -> (f32, f32) {
        let k = u2f(self.snap) / SCREEN_PX;
        (self.needle_l as f32 / k, self.needle_r as f32 / k)
    }
    pub fn agc_gain(&self) -> f32 {
        self.agc.gain
    }
    pub fn rng(&self) -> u32 {
        self.rng
    }
    pub fn set_rng(&mut self, seed: u32) {
        self.rng = seed;
    }

    /// Internal state in the order the emulator harness prints it
    /// (`reverse_engineering/riverrun/tools/emulate_riverrun.py --trace`).
    pub fn trace(&self) -> String {
        format!(
            "t={:.6} last_t={:.6} interval={:.6} new={:.6} len={:.6} pitch={:.6} range={:.6} fine={:.6} jit={} \
             glass_pos={:.6} glass_end={} glass={:.6} nl={} nr={} rng={:#x} out_pos={} shape={} env_len={} agc={:.9} pk={:.9}",
            self.t, self.last_t, self.interval, self.new_interval, self.g_len, self.g_pitch, self.g_range,
            self.g_fine, self.g_jitter, self.glass_pos, self.glass_end, self.glass, self.needle_l,
            self.needle_r, self.rng, self.out_pos, self.shape, self.env_len, self.agc.gain, self.agc.last_peak
        )
    }

    /// `SetCursorLeft(float)` @ 0x1fbe0: the editor (or param 11, or a MIDI
    /// note) moves the Grain Glass.
    pub fn set_glass(&mut self, px: f32) {
        self.glass = px;
    }

    /// One call of the original `Process` with `n` samples.
    pub fn process_block(
        &mut self,
        c: &Controls,
        host: &HostTime,
        input: [&[f32]; 2],
        stereo_in: bool,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let n = out_l.len();
        if c.record {
            self.record(&input[0][..n], if stereo_in { Some(&input[1][..n]) } else { None });
        }
        self.begin_block(c, host, n);
        self.render(out_l, out_r);
    }

    /// The recording part of `Process`: input → snippet (mono, or
    /// `(L + R) * 0.707` for a stereo input).
    pub fn record(&mut self, l: &[f32], r: Option<&[f32]>) {
        let mut done = 0;
        while done < l.len() {
            let m = (l.len() - done).min(self.rec_tmp.len());
            let tmp = &mut self.rec_tmp[..m];
            match r {
                Some(r) => {
                    for i in 0..m {
                        tmp[i] = ((l[done + i] + r[done + i]) as f64 * 0.707) as f32;
                    }
                }
                None => tmp.copy_from_slice(&l[done..done + m]),
            }
            self.snippet.put(tmp);
            done += m;
        }
    }

    /// Replaces the input screen with `data` (oldest sample first), like
    /// `hgiovh…::LoadSnippet` @ 0x1c9b2: truncated or zero-padded to the
    /// snapshot length and written in one `Put` (so the put position ends where
    /// it started). Allocation-free; `src_rate` ≠ engine rate resamples
    /// linearly (the original did not convert).
    pub fn load_snippet_at(&mut self, data: &[f32], src_rate: f64) {
        let len = self.snippet.len as usize;
        let put = self.snippet.put as usize;
        let step = if src_rate > 0.0 { src_rate / self.sr } else { 1.0 };
        let same = (step - 1.0).abs() < 1e-12;
        for i in 0..len {
            let v = if same {
                data.get(i).copied().unwrap_or(0.0)
            } else {
                let p = i as f64 * step;
                let k = p as usize;
                match data.get(k) {
                    Some(&a) => {
                        let b = data.get(k + 1).copied().unwrap_or(a);
                        a + (b - a) * (p - k as f64) as f32
                    }
                    None => 0.0,
                }
            };
            self.snippet.buf[(put + i) % len] = v;
        }
    }

    pub fn load_snippet(&mut self, data: &[f32]) {
        self.load_snippet_at(data, self.sr);
    }

    /// Clears the input screen.
    pub fn clear_snippet(&mut self) {
        self.snippet.clear();
    }

    /// The input screen, oldest sample first (`GetSnippet` @ 0x1c692), into a
    /// vector whose capacity the caller reserved (no allocation then).
    pub fn copy_snippet_into(&self, out: &mut Vec<f32>) {
        let p = self.snippet.put as usize;
        out.clear();
        out.extend_from_slice(&self.snippet.buf[p..]);
        out.extend_from_slice(&self.snippet.buf[..p]);
    }

    pub fn snippet_linear(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.snap as usize);
        self.copy_snippet_into(&mut v);
        v
    }

    /// Everything `Process` does before reading the output: parameters → engine,
    /// tempo, Grain Glass, envelope, then `Run`.
    pub fn begin_block(&mut self, c: &Controls, host: &HostTime, n: usize) {
        let speed_floor = ((c.speed as f64) + 0.01).floor() as f32;

        // ---- tempo context (0x204f8…0x206e9) ----
        let old_bpm = self.bpm;
        let bpm = if host.bpm > 0.0 { host.bpm } else if self.bpm > 0.0 { self.bpm } else { 120.0 };
        self.bpm = bpm;
        self.spq = self.sr / (bpm / 60.0); // adodg…::SetTempo @ 0xdf9a
        self.playing = host.playing;
        let ticks_per_block = (n as f64 / self.spq) * 480.0;
        let mut resync = false;
        if !self.was_sync && c.rhythmic {
            resync = true;
        }
        if old_bpm != bpm {
            resync = true;
        }
        if host.transport_changed {
            resync = true;
        }
        if speed_floor != self.last_speed {
            resync = true;
            self.last_speed = speed_floor;
        }
        // `QuarterPosFraction`: the part of the current quarter already played
        // (`ppq - (int)ppq` in VSTMissionHost::GetTimeInfo @ 0x26998).
        let ticks_pos = (host.ppq - host.ppq.trunc()) * 480.0;
        let d = (host.sample_pos as i32) as f64 - self.last_host_pos as f64;
        let jumped = (n as f64) < d || d <= 0.0;
        // The original also "jumps" every block while the transport is stopped
        // (the position does not move); Tight only resyncs on real jumps.
        if jumped && (self.beat_sync == BeatSync::Original || host.playing) {
            resync = true;
        }
        if resync {
            self.resync = true;
        }
        if self.resync {
            self.spq_engine = self.spq;
        }
        self.last_host_pos = host.sample_pos as i32 as i64;

        // Leaving Rhythmic drops the AGC gain to a fifth (0x206f9).
        if !c.rhythmic && self.was_sync {
            self.agc.gain = (self.agc.gain as f64 * 0.2) as f32;
        }
        self.was_sync = c.rhythmic;

        // ---- grain length, pitch, fine tune ----
        let glen = breakpoint(to_short(c.length), &LENGTH_MAP) as i32;
        self.g_len = (self.sr / 44100.0) * (glen as f32 as f64);
        self.g_pitch = breakpoint(to_short(c.pitch), &PITCH_MAP) as f64;
        self.g_fine = c.fine as f64;

        // ---- grain spacing ----
        if !c.rhythmic {
            self.sync = false;
            let rate = breakpoint(to_short(c.speed), &FLOWING_MAP);
            self.new_interval = self.sr / rate as f64;
        } else {
            self.sync = true;
            let mut idx = ((c.speed as f64) / 21.142_857_142_857_142 + 0.5) as i32 as i16;
            if idx > 13 {
                idx = 13;
            }
            let (num, den) = RHYTHM_DIVISIONS[idx.max(0) as usize];
            self.new_interval = (den / num) * self.spq;
            self.ticks_per_block = ticks_per_block;
            self.ticks_pos = ticks_pos;
            if self.resync && self.beat_sync == BeatSync::Tight {
                // Not in the original: put the first grain on the division grid.
                let q = den / num;
                let ticks = if host.playing && host.bpm > 0.0 {
                    let next = (host.ppq / q).ceil() * q;
                    ((next - host.ppq) * 480.0).max(0.0)
                } else {
                    0.0
                };
                self.ticks_pos = ticks;
                self.ticks_per_block = f64::INFINITY;
            }
        }
        self.mode = 2; // two output buses

        self.g_range = breakpoint(to_short(c.random), &RANDOM_MAP) as f64;
        let tr = breakpoint(to_short(c.speed_random), &TIMING_MAP) as i32;
        // The original divides by this; below 44.1 kHz it could reach 0.
        self.g_jitter = ((tr as f64 * (self.sr / 44100.0)) as f32 as i32).max(1);
        let mut glen_f = self.g_len as f32;

        self.grid = (c.grid - 1) as i16;

        // ---- the walking Grain Glass (0x20bf3…0x21171) ----
        self.walk_samples = (u2f(n as u32) * c.walk) as i32;
        let px = u2f(self.snap) / SCREEN_PX;
        self.needle_l = f2u((c.needle_l as i32) as f32 * px);
        self.needle_r = f2u((c.needle_r as i32) as f32 * px).min(self.snap);
        self.glass_pos = (px * self.glass) as f64 + self.walk_samples as f64;
        if (self.needle_l as f64) > self.glass_pos {
            self.glass_pos = self.needle_r as f64;
        }
        if (self.needle_r as f64) < self.glass_pos {
            self.glass_pos = self.needle_l as f64;
        }
        let snapd = self.snap as f64;
        if snapd < glen_f as f64 + self.glass_pos {
            if self.walk_samples as f64 <= 0.0 {
                self.glass_pos = (u2f(self.snap) - glen_f) as f64;
                if self.glass_pos < self.needle_l as f64 {
                    glen_f = u2f(self.snap.wrapping_sub(self.needle_l));
                    self.glass_pos = self.needle_l as f64;
                    self.g_len = glen_f as f64;
                }
            } else {
                self.glass_pos = self.needle_l as f64;
                if snapd < glen_f as f64 + self.glass_pos {
                    glen_f = (snapd - self.glass_pos) as f32;
                    self.g_len = glen_f as f64;
                }
            }
        }
        self.glass_end = (self.glass_pos.ceil() + 1.0) as i32;
        self.glass = ((self.glass_pos as f32) / u2f(self.snap)) * SCREEN_PX;
        self.glass_width_px = (((glen_f / u2f(self.snap)) * SCREEN_PX) as i32) % 0x1d8;

        // ---- envelope menu (0x211d0) ----
        if c.envelope != self.env_choice {
            if let Some((shape, a, d)) = envelope::menu_to_shape(c.envelope) {
                self.shape = shape;
                self.attack = a;
                self.decay = d;
            }
            envelope::init_master(&mut self.master, self.shape, self.attack, self.decay);
            self.env_dirty = true;
            self.env_choice = c.envelope;
        }

        self.stereo_k = ((c.mono_stereo as f64) / -93.0 + 1.0) as f32;

        self.recent.clear();
        self.run(n as u32);
    }

    /// The rest of `Process`: read the buses, AGC, mono/stereo.
    /// May be called in several pieces that together make up the block.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        self.buses[0].get_and_clear(out_l);
        self.buses[1].get_and_clear(out_r);
        self.agc.process(out_l, out_r);
        let k = self.stereo_k;
        for (l, r) in out_l.iter_mut().zip(out_r.iter_mut()) {
            let (a, b) = (*l, *r);
            *l = k * b + a;
            *r = a * k + b;
        }
        self.out_pos = self.buses[0].get;
        if self.resync {
            self.spq_engine = self.spq;
        }
    }

    /// `pdrab…::Run(unsigned long)` @ 0x1adda: makes one grain (if one is due in
    /// this block) and spreads copies of it over the output buses.
    fn run(&mut self, n: u32) {
        let block_start = self.out_pos;
        let block_end = n.wrapping_add(block_start as u32);
        let mut rng = self.rng;

        let mut t;
        if !self.sync {
            self.resync = false;
            self.interval = self.new_interval;
            t = self.t;
        } else if self.resync && (self.ticks_pos < self.ticks_per_block || !self.playing) {
            self.t = (self.ticks_pos / 480.0) * self.spq_engine + block_start as f64;
            self.resync = false;
            self.interval = self.new_interval;
            t = self.t;
            if t - self.last_t < (self.sr as f32) as f64 * 0.05 {
                t += self.new_interval;
                self.t = t;
            }
        } else {
            t = self.t;
        }

        let bs = block_start as f64;
        let be = block_end as f64;
        if bs <= t {
            let mut n_g = 0u32;
            if t < be {
                // ---- make the grain ----
                rng = lcg(rng);
                let w = ((self.glass_end as f64 - self.glass_pos) as i32 as u32).max(1);
                let start = (self.snippet.put as f64 + self.glass_pos + (rng % w) as f64) as i32;
                self.snippet.set_get(start);
                let read_from = self.snippet.get;

                let mut p = (((u2f(rng) * TWO_M32) * self.g_range as f32) as f64 + self.g_pitch) as f32;
                if self.grid != 0 {
                    p = self.scales.snap(self.grid, p);
                }
                let ratio = (p as f64 * self.g_fine) as f32;
                let mut len = self.g_len / ratio as f64;
                let maxg = self.max_grain as f64;
                if len > maxg {
                    len = maxg;
                }
                n_g = d2u(len);
                self.snippet.get_pitched(&mut self.grain, ratio, n_g as usize);

                let a = d2u(n_g as f64 * self.attack);
                let d = d2u(n_g as f64 * self.decay);
                if n_g != self.env_len || self.env_dirty {
                    envelope::fill_grain_env(&mut self.env, &self.master, self.shape, n_g, a, d);
                    self.env_len = n_g;
                    self.env_dirty = false;
                }
                envelope::apply(&mut self.grain, &self.env, self.shape, n_g, a, d);

                let pos = ((read_from - self.snippet.put).rem_euclid(self.snippet.len)) as f32
                    / self.snap as f32;
                if self.recent.len() < self.recent.capacity() {
                    self.recent.push(GrainInfo { pos, ratio, bus: 0 });
                }
            }
            // ---- scatter copies over the buses ----
            let interval = self.interval;
            let mut first = true;
            while bs <= t && t < be {
                let r1 = lcg(rng);
                let bus = choose_bus(self.mode, r1);
                rng = lcg(r1);
                let at = (t as i32).wrapping_add((rng % self.g_jitter as u32) as i32);
                let b = &mut self.buses[bus.min(1)];
                b.set_put(at);
                b.add_grain(&self.grain[..n_g as usize]);
                self.last_t = t;
                t += interval;
                self.grain_count += 1;
                if first {
                    if let Some(g) = self.recent.last_mut() {
                        g.bus = bus as u8;
                    }
                    first = false;
                }
            }
        }
        if self.bus_len <= block_end {
            t -= self.bus_len as f64;
        }
        self.t = t;
        self.rng = rng;
    }
}

/// `(unsigned long)(float)`, saturating, as GCC emits it.
#[inline]
fn f2u(v: f32) -> u32 {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= 4_294_967_296.0 {
        u32::MAX
    } else {
        v as u32
    }
}

/// Output bus for one grain copy (`Run` @ 0x1b3b0): stereo picks by the sign
/// bit, quad and 5.0 split the 32-bit range in equal parts.
#[inline]
fn choose_bus(mode: i32, r: u32) -> usize {
    match mode {
        2 => usize::from((r as i32) < 0),
        3 => {
            if (r as i32) < 0 {
                if r < 0xbfff_ffff { 1 } else { 3 }
            } else if r > 0x3fff_ffff {
                0
            } else {
                2
            }
        }
        4 => match r {
            0..0x3333_3333 => 0,
            0x3333_3333..0x6666_6666 => 1,
            0x6666_6666..0x9999_9999 => 2,
            0x9999_9999..0xcccc_cccc => 3,
            _ => 4,
        },
        1 => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vowel(sr: f64, secs: f64) -> Vec<f32> {
        (0..(sr * secs) as usize)
            .map(|i| {
                let t = i as f64 / sr;
                ((t * 220.0 * std::f64::consts::TAU).sin() * 0.5) as f32
            })
            .collect()
    }

    #[test]
    fn silent_until_something_is_in_the_screen() {
        let mut e = Engine::new(44100.0);
        let c = Controls::default();
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        for _ in 0..50 {
            e.process_block(&c, &HostTime::default(), [&[0.0; 512], &[0.0; 512]], false, &mut l, &mut r);
            assert!(l.iter().chain(&r).all(|&s| s == 0.0));
        }
    }

    #[test]
    fn makes_grains_from_a_loaded_snippet() {
        let mut e = Engine::new(44100.0);
        e.load_snippet(&vowel(44100.0, 4.0));
        let c = Controls::default();
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        let mut energy = 0.0f64;
        for _ in 0..200 {
            e.process_block(&c, &HostTime::default(), [&[0.0; 512], &[0.0; 512]], false, &mut l, &mut r);
            energy += l.iter().chain(&r).map(|&s| (s * s) as f64).sum::<f64>();
            assert!(l.iter().chain(&r).all(|s| s.is_finite()));
        }
        assert!(energy > 1.0);
        assert!(e.grain_count > 100);
    }

    #[test]
    fn rhythmic_mode_locks_to_the_grid() {
        let mut e = Engine::new(48000.0);
        e.load_snippet(&vowel(48000.0, 4.0));
        e.beat_sync = BeatSync::Tight;
        let c = Controls { rhythmic: true, speed: 4.0 * 21.142857, speed_random: 0.0, ..Controls::default() };
        // Quarter notes at 120 BPM = 24000 samples apart.
        let mut onsets = vec![];
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        let mut pos = 0i64;
        for _ in 0..1000 {
            let host = HostTime { playing: true, bpm: 120.0, ppq: pos as f64 / 24000.0, sample_pos: pos, transport_changed: pos == 0 };
            let before = e.grain_count;
            e.process_block(&c, &host, [&[0.0; 256], &[0.0; 256]], false, &mut l, &mut r);
            if e.grain_count > before {
                onsets.push(pos);
            }
            pos += 256;
        }
        assert!(onsets.len() >= 9);
        for w in onsets.windows(2).skip(1) {
            let d = w[1] - w[0];
            assert!((d - 24000).abs() <= 256, "spacing {d}");
        }
    }
}
