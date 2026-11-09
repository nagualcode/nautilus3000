//! The Deep Phase Nine engine, translated from the i386 slice of
//! `DeepPhaseNine.vst` 2.6.4 (see `reverse_engineering/deepphasenine/`).
//!
//! * [`Phaser`] is `bqoibkkycavvbtrbgpvkncpnvmxives`: the LFO and one or two
//!   chains of identical second-order all-pass sections (`UpdateOneBuffer`
//!   @ 0x1aed0, `UpdateTwoBuffers` @ 0x1b616);
//! * [`Store`] holds the private parameters through which the phaser talks to
//!   itself and to the editor (it writes the LFO value into "LFO Position" and
//!   reads it back);
//! * [`DeepPhaseNine`] is the process (`jwtrfajr…::Process` @ 0x1c784):
//!   parameters → phaser, the channel layouts, and the look-ahead limiter.

use mission::agc::Agc;

/// `(float)M_PI`, the π of the phaser (`0x40490fdb`).
const PI_F: f32 = std::f32::consts::PI;
/// The same constant widened, used to divide `acos` results.
const PI_FD: f64 = std::f32::consts::PI as f64;

/// Control rate: LFO and coefficients are recomputed every 123 samples.
pub const TICK: i32 = 123;
/// Section slots per channel in the object (`0x30` floats per array).
pub const MAX_SECTIONS: usize = 48;

/// Beats per LFO cycle in "sync to beats" (`Process::C_149`, 85 doubles).
pub const SYNC_BEATS: [f64; 85] = [
    0.03125, 0.035714285714285712, 0.0375, 0.041666666666666664, 0.046875, 0.05,
    0.055555555555555552, 0.0625, 0.071428571428571425, 0.075, 0.083333333333333329, 0.09375,
    0.1, 0.1111111111111111, 0.125, 0.14285714285714285, 0.15, 0.16666666666666666, 0.1875, 0.2,
    0.22222222222222221, 0.25, 0.2857142857142857, 0.3, 0.33333333333333331, 0.375, 0.4,
    0.44444444444444442, 0.5, 0.5714285714285714, 0.6, 0.66666666666666663, 0.75, 0.8,
    0.88888888888888884, 1.0, 1.1428571428571428, 1.2, 1.3333333333333333, 1.5, 1.6,
    1.7777777777777777, 2.0, 2.2857142857142856, 2.4, 2.6666666666666665, 3.0, 3.2,
    3.5555555555555554, 4.0, 4.5714285714285712, 4.8, 5.333333333333333, 6.0, 6.4,
    7.1111111111111107, 8.0, 9.1428571428571423, 9.6, 10.666666666666666, 12.0, 12.8,
    14.222222222222221, 16.0, 18.285714285714285, 19.2, 21.333333333333332, 24.0, 25.6,
    28.444444444444443, 32.0, 36.571428571428569, 38.4, 42.666666666666664, 48.0, 51.2,
    56.888888888888886, 64.0, 73.142857142857139, 76.8, 85.333333333333329, 96.0, 102.4,
    113.77777777777777, 128.0,
];

pub const LFO_NAMES: [&str; 5] = ["Sine", "Triangle", "Sample & Hold", "OneShot Sine", "OneShot Triangle"];

#[inline]
fn cosf(x: f32) -> f32 {
    (x as f64).cos() as f32
}
#[inline]
fn sinf(x: f32) -> f32 {
    (x as f64).sin() as f32
}

/// The private parameters 0x0c…0x16 (`GetPrivateParameter` @ 0x1a60c), with
/// the store's semantics: a float only changes inside its range and when it
/// differs, a long only when it differs.
#[derive(Clone, Debug)]
pub struct Store {
    /// 0x0c "LFO Position" (float 0…1000).
    pub lfo_pos: f32,
    /// 0x0d "UIF Position": the ball dragged in the editor.
    pub uif_pos: f32,
    /// 0x12 "LFO Direction" (±1).
    pub lfo_dir: i32,
    /// 0x13 "UIF Direction".
    pub uif_dir: i32,
    /// 0x14 "User update": the LFO shape just changed.
    pub user_update: i32,
    /// 0x15 "At end": a one-shot has arrived.
    pub at_end: i32,
    /// 0x16 "Hold": the editor holds the ball.
    pub hold: i32,
}

impl Default for Store {
    fn default() -> Self {
        Self { lfo_pos: 0.0, uif_pos: 0.0, lfo_dir: 1, uif_dir: 1, user_update: 0, at_end: 0, hold: 0 }
    }
}

impl Store {
    fn set_lfo_pos(&mut self, v: f32) {
        if v <= 1000.0 && 0.0 <= v && v != self.lfo_pos {
            self.lfo_pos = v;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Coefs {
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
}

#[derive(Clone)]
struct Chain {
    x1: [f32; MAX_SECTIONS],
    x2: [f32; MAX_SECTIONS],
    y1: [f32; MAX_SECTIONS],
    y2: [f32; MAX_SECTIONS],
}

impl Default for Chain {
    fn default() -> Self {
        Self { x1: [0.0; MAX_SECTIONS], x2: [0.0; MAX_SECTIONS], y1: [0.0; MAX_SECTIONS], y2: [0.0; MAX_SECTIONS] }
    }
}

impl Chain {
    /// One sample through `sections` identical biquads; returns the output.
    #[inline]
    fn run(&mut self, input: f32, c: &Coefs, sections: i32) -> f32 {
        let x1 = self.x1[0];
        let x2 = self.x2[0];
        self.x2[0] = x1;
        self.x1[0] = input;
        let y2 = self.y2[0];
        self.y2[0] = self.y1[0];
        let mut y = ((input * c.b0 + x1 * c.b1 + c.b2 * x2) - c.a1 * self.y1[0]) - c.a2 * y2;
        self.y1[0] = y;
        let mut k = 1usize;
        while (k as i32) < sections {
            let x1 = self.x1[k];
            self.x1[k] = y;
            let x2 = self.x2[k];
            self.x2[k] = x1;
            let y2 = self.y2[k];
            self.y2[k] = self.y1[k];
            y = ((y * c.b0 + x1 * c.b1 + c.b2 * x2) - c.a1 * self.y1[k]) - c.a2 * y2;
            self.y1[k] = y;
            k += 1;
        }
        y
    }
}

/// `bqoibkkycavvbtrbgpvkncpnvmxives`, field offsets in the comments.
#[derive(Clone)]
pub struct Phaser {
    lo: f32,             // +0x00 low  / (sr/2)
    hi: f32,             // +0x04 high / (sr/2)
    feedback: f32,       // +0x08
    pub phase: f32,      // +0x0c 0…1, bouncing
    offset: f32,         // +0x10 stereo offset (degrees / 180)
    sr: f32,             // +0x14
    prev_phase: f32,     // +0x18
    rate: f32,           // +0x1c phase step per tick
    pub dir: i32,        // +0x20
    shape: i32,          // +0x24 1…5
    depth: f32,          // +0x28
    stages: i32,         // +0x2c 2 × notches
    fb: [f32; 2],        // +0x30 / +0x34
    chain: [Chain; 2],   // +0x38…
    tick: i32,           // +0x638
    /// Notch centre of the left chain, 0…1 of Nyquist (+0x63c).
    pub freq: f32,
    coefs: [Coefs; 2],   // +0x640 / +0x658
    spread: f32,         // +0x670
    rng: u32,            // +0x674
    oneshot_end: f32,    // +0x678
    oneshot_dir: i32,    // +0x67c
}

impl Phaser {
    /// The constructor @ 0x1c194, plus `NumStages(6)` from `InitBuffers`.
    pub fn new(sample_rate: f32) -> Self {
        let half = sample_rate * 0.5;
        Self {
            lo: 440.0 / half,
            hi: 1600.0 / half,
            feedback: 0.7,
            phase: 0.0,
            offset: 0.0,
            sr: sample_rate,
            prev_phase: 0.0,
            rate: (0.5 / sample_rate) * 246.0,
            dir: 1,
            shape: 1,
            depth: 1.0,
            stages: 6,
            fb: [0.0; 2],
            chain: [Chain::default(), Chain::default()],
            tick: 0,
            freq: 0.5,
            coefs: [Coefs::default(); 2],
            spread: 0.0,
            rng: 0x0023_db21,
            oneshot_end: 0.0,
            oneshot_dir: 0,
        }
    }

    pub fn set_range(&mut self, low_hz: f32, high_hz: f32) {
        self.lo = low_hz / (self.sr * 0.5);
        self.hi = high_hz / (self.sr * 0.5);
    }
    pub fn set_rate(&mut self, hz: f32) {
        self.rate = (hz / self.sr) * 246.0;
    }
    pub fn set_spread(&mut self, s: f32) {
        self.spread = s;
    }
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
    }
    pub fn set_phase(&mut self, p: f32) {
        self.phase = p;
    }
    pub fn set_direction(&mut self, d: i32) {
        self.dir = d;
    }
    pub fn set_offset(&mut self, o: f32) {
        self.offset = o;
    }
    pub fn set_feedback(&mut self, f: f32) {
        self.feedback = f;
    }
    pub fn set_stages(&mut self, n: i32) {
        self.stages = n;
    }
    pub fn set_depth(&mut self, d: f32) {
        self.depth = d;
    }
    pub fn set_oneshot_end(&mut self, e: f32) {
        self.oneshot_end = e;
    }
    pub fn set_oneshot_direction(&mut self, d: i32) {
        self.oneshot_dir = d;
    }

    /// `lfoShape(short)` @ 0x1ad6e.
    pub fn set_shape(&mut self, shape: i32, st: &mut Store) {
        if shape != self.shape {
            st.user_update = 1;
            if (shape - 4) as u16 > 1 {
                st.at_end = 0;
            }
            if shape == 3 {
                self.phase = 0.0;
                self.dir = 1;
            }
        }
        self.shape = shape;
    }

    /// `acos(−(2e − 1)) / π`: the phase at which a sine reaches `e`.
    #[inline]
    fn acos_phase(e: f32) -> f64 {
        (-((e + e) - 1.0) as f64).acos() / PI_FD
    }

    /// The LFO step of a tick (identical in both update functions except
    /// for the mono version's read of the dragged ball, see `mono`).
    fn advance(&mut self, st: &mut Store, mono: bool) -> bool {
        if st.hold != 0 {
            self.dir = st.uif_dir;
            if self.shape == 1 || self.shape == 4 {
                // The mono version reads the float "UIF Position" with
                // GetParamLong, which returns 0: the phase snaps to 0.
                let p = if mono { 0.0 } else { st.uif_pos };
                self.phase = Self::acos_phase(p) as f32;
            } else {
                self.phase = st.uif_pos;
            }
        } else if self.shape == 3 && st.user_update != 0 {
            self.phase = 0.0;
        }
        if st.at_end == 0 && st.hold == 0 {
            self.prev_phase = self.phase;
            self.phase = self.dir as f32 * self.rate + self.phase;
            if (self.shape - 4) as u32 <= 1 && st.user_update == 0 {
                let end = if self.shape == 4 { Self::acos_phase(self.oneshot_end) } else { self.oneshot_end as f64 };
                let od = self.oneshot_dir;
                let arrived = if od >= 1 {
                    (self.prev_phase as f64) <= end && !((self.phase as f64) < end)
                } else if od != 0 {
                    end <= self.prev_phase as f64 && !(end < self.phase as f64)
                } else {
                    false
                };
                if arrived {
                    self.dir = od;
                    self.phase = end as f32;
                    st.at_end = 1;
                }
            }
        }
        let mut wrapped = false;
        if 0.0 > self.phase {
            wrapped = st.lfo_dir != 0;
            self.dir = 1;
            self.phase = -self.phase;
            if self.phase > 1.0 {
                self.phase = 1.0;
                self.dir = -1;
            }
        }
        if self.phase > 1.0 {
            self.dir = -1;
            self.phase = 2.0 - self.phase;
            if 0.0 > self.phase {
                // GetParamLong on the float "LFO Position" is 0.
                self.phase = 0.0;
                self.dir = 1;
            }
        }
        wrapped
    }

    /// Publishes the LFO value (`SetParamFloat(0xc)`, `SetParamLong(0x12)`).
    fn publish(&mut self, st: &mut Store, wrapped: bool, mono: bool) {
        match self.shape {
            2 | 5 => {
                st.set_lfo_pos(self.phase);
                st.lfo_dir = self.dir;
            }
            1 | 4 => {
                let v = ((1.0 - ((self.phase * PI_F) as f64).cos()) * 0.5) as f32;
                st.set_lfo_pos(v);
                st.lfo_dir = self.dir;
            }
            3 if wrapped => {
                self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
                let dir = if self.rng & 0x10 == 0 { -1 } else { 1 };
                let v = (self.rng as f64 / 4_294_967_295.0) as f32;
                if mono {
                    st.lfo_dir = dir;
                    st.set_lfo_pos(v);
                } else {
                    st.set_lfo_pos(v);
                    st.lfo_dir = dir;
                }
            }
            _ => {}
        }
    }

    /// Second-order all-pass with poles at radius `1 − 0.95·spread`.
    fn coefs(&self, v: f32) -> (f32, Coefs) {
        let w = v * (self.hi - self.lo) + self.lo;
        (w, section(w, self.spread))
    }
}

/// One all-pass section for centre `w` (fraction of Nyquist) and spread
/// (0.01…0.5): `(a1, a2, b0, b1, b2)`. Public for the editor's display.
pub fn section_coefs(w: f32, spread: f32) -> [f32; 5] {
    let c = section(w, spread);
    [c.a1, c.a2, c.b0, c.b1, c.b2]
}

fn section(w: f32, spread: f32) -> Coefs {
    {
        let r = (spread as f64 * -0.95 + 1.0) as f32;
        let x = w * PI_F;
        let c = cosf(x);
        let s = sinf(x);
        let rc = r * c;
        let rs = r * s;
        let a1 = rc * -2.0;
        let a2 = rc * rc + rs * rs;
        let inv = 1.0 / r;
        let c2 = c * inv;
        let s2 = s * inv;
        let b1 = c2 * -2.0;
        let b2 = c2 * c2 + s2 * s2;
        let g = ((a1 + 1.0) + a2) / (b2 + (b1 + 1.0));
        Coefs { a1, a2, b0: g, b1: g * b1, b2: g * b2 }
    }
}

impl Phaser {

    /// Right channel: the phase shifted by the stereo offset, folded.
    fn right_value(&self, st: &Store) -> f32 {
        let mut v = if self.shape == 3 {
            ((self.rng as f64 / 4_294_967_295.0) - (st.lfo_dir as f32 * self.offset) as f64) as f32
        } else {
            self.phase - self.dir as f32 * self.offset
        };
        if !(v >= 0.0) && !v.is_nan() {
            v = -v;
        }
        if v > 1.0 {
            v = 2.0 - v;
        }
        if self.shape == 1 || self.shape == 4 {
            v = ((1.0 - ((v * PI_F) as f64).cos()) * 0.5) as f32;
        }
        v
    }

    #[inline]
    fn on_tick(&self) -> bool {
        let t = self.tick;
        t == (t / TICK) * TICK
    }

    /// `UpdateTwoBuffers` @ 0x1b616, in place.
    pub fn process_stereo(&mut self, st: &mut Store, l: &mut [f32], r: &mut [f32]) {
        let sections = self.stages / 2;
        for i in 0..l.len() {
            if self.on_tick() {
                let wrapped = self.advance(st, false);
                self.publish(st, wrapped, false);
                let (w, c) = self.coefs(st.lfo_pos);
                self.freq = w;
                self.coefs[0] = c;
                let rv = self.right_value(st);
                let (_, c) = self.coefs(rv);
                self.coefs[1] = c;
            }
            self.tick = self.tick.wrapping_add(1);

            let x = l[i];
            let input = self.fb[0] * self.feedback + x;
            let y = self.chain[0].run(input, &self.coefs[0], sections);
            self.fb[0] = (y as f64 * 0.1 + self.fb[0] as f64 * 0.9) as f32;
            l[i] = y * self.depth + x;

            let x = r[i];
            let input = self.fb[1] * self.feedback + x;
            let y = self.chain[1].run(input, &self.coefs[1], sections);
            self.fb[1] = y;
            r[i] = y * self.depth + x;
        }
    }

    /// `UpdateOneBuffer` @ 0x1aed0, in place.
    pub fn process_mono(&mut self, st: &mut Store, x: &mut [f32]) {
        let sections = self.stages / 2;
        for s in x.iter_mut() {
            if self.on_tick() {
                let wrapped = self.advance(st, true);
                self.publish(st, wrapped, true);
                let (w, c) = self.coefs(st.lfo_pos);
                self.freq = w;
                self.coefs[0] = c;
            }
            self.tick = self.tick.wrapping_add(1);
            let input = self.fb[0] * self.feedback + *s;
            let y = self.chain[0].run(input, &self.coefs[0], sections);
            self.fb[0] = y;
            *s = y * self.depth + *s;
        }
    }

    /// The right chain's centre, for the display.
    pub fn freq_right(&self, st: &Store) -> f32 {
        self.coefs(self.right_value(st)).0
    }
}

/// The controls, in the original's units (`orjogk…::GetParameter` @ 0x19e56).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// 0: 1…24
    pub notches: i32,
    /// 1: −0.95…0.95
    pub feedback: f32,
    /// 2: 0.01…2000 Hz
    pub low: f32,
    /// 3: 200…20000 Hz
    pub high: f32,
    /// 4: 0…100
    pub spread: f32,
    /// 5: 0…1 ("WetDry", really the depth of the phased signal)
    pub wet_dry: f32,
    /// 6: 1…5
    pub lfo_choice: i32,
    /// 7: 0…70
    pub lfo_speed: i32,
    /// 8: 0…1
    pub oneshot_end: f32,
    /// 9: −1…1
    pub oneshot_dir: i32,
    /// 10
    pub sync: bool,
    /// 11: −180…180 degrees
    pub stereo_offset: i32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            notches: 3,
            feedback: 0.8,
            low: 100.0,
            high: 4000.0,
            spread: 35.0,
            wet_dry: 1.0,
            lfo_choice: 1,
            lfo_speed: 21,
            oneshot_end: 0.5,
            oneshot_dir: -1,
            sync: false,
            stereo_offset: 0,
        }
    }
}

impl Controls {
    /// The getters clamp to the parameter ranges (`GetParamFloat` / `GetParamLong`).
    fn clamped(&self) -> Self {
        Self {
            notches: self.notches.clamp(1, 24),
            feedback: self.feedback.clamp(-0.95, 0.95),
            low: self.low.clamp(0.01, 2000.0),
            high: self.high.clamp(200.0, 20000.0),
            spread: self.spread.clamp(0.0, 100.0),
            wet_dry: self.wet_dry.clamp(0.0, 1.0),
            lfo_choice: self.lfo_choice.clamp(1, 5),
            lfo_speed: self.lfo_speed.clamp(0, 70),
            oneshot_end: self.oneshot_end.clamp(0.0, 1.0),
            oneshot_dir: self.oneshot_dir.clamp(-1, 1),
            sync: self.sync,
            stereo_offset: self.stereo_offset.clamp(-180, 180),
        }
    }

    /// LFO frequency in Hz (free running) or beats per cycle (sync).
    pub fn lfo_rate_hz(&self, sample_rate: f64, samples_per_quarter: f64) -> f64 {
        let c = self.clamped();
        if c.lfo_choice == 3 {
            if !c.sync {
                2f64.powf(c.lfo_speed as f64 / 5.0) * 0.03125
            } else {
                // The original indexes the table with 0x3f − speed, which runs
                // off its start above 63; clamped here.
                sample_rate / (samples_per_quarter * SYNC_BEATS[(0x3f - c.lfo_speed).max(0) as usize])
            }
        } else if !c.sync {
            2f64.powf(c.lfo_speed as f64 / 5.0) * 0.003_906_25
        } else {
            sample_rate / (samples_per_quarter * SYNC_BEATS[(0x54 - c.lfo_speed) as usize])
        }
    }
}

/// What the host says about time at the start of a block.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostTime {
    pub playing: bool,
    pub bpm: f64,
    pub ppq: f64,
    pub transport_changed: bool,
}

/// Channel layouts: the original variants 0 (mono), 1 (mono → stereo) and
/// 2 (stereo). Quad / 5.0 / 5.1 used two or three phasers with their LFOs a
/// third apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Mono,
    MonoToStereo,
    Stereo,
}

pub struct DeepPhaseNine {
    sr: f64,
    pub layout: Layout,
    pub phaser: Phaser,
    pub store: Store,
    agc: Agc,
    spq: f64,
    last_speed: i32,
}

impl DeepPhaseNine {
    /// `InitBuffers` @ 0x1e104.
    pub fn new(sample_rate: f64, layout: Layout) -> Self {
        let mut phaser = Phaser::new(sample_rate as f32);
        phaser.set_phase(0.0);
        let channels = if layout == Layout::Mono { 1 } else { 2 };
        Self {
            sr: sample_rate,
            layout,
            phaser,
            store: Store::default(),
            agc: Agc::deep_phase_nine(channels, sample_rate),
            spq: sample_rate * 0.5,
            last_speed: 0,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    pub fn limiter_gain(&self) -> f32 {
        self.agc.gain
    }

    /// One call of the original `Process`. `out` holds the input on entry
    /// (one channel for Mono, two otherwise; for MonoToStereo both carry the
    /// mono input).
    pub fn process(&mut self, c: &Controls, host: &HostTime, l: &mut [f32], r: &mut [f32]) {
        let c = c.clamped();
        if host.bpm > 0.0 {
            self.spq = self.sr / (host.bpm / 60.0);
        }
        let mut changed = host.transport_changed;
        if !changed && self.last_speed != c.lfo_speed {
            changed = true;
        }
        self.last_speed = c.lfo_speed;

        let spread = c.spread.max(2.0) / 200.0;
        self.phaser.set_shape(c.lfo_choice, &mut self.store);
        let rate = c.lfo_rate_hz(self.sr, self.spq);
        if c.lfo_choice == 3 && c.sync && changed {
            // Sample & hold, synced: restart the bounce from the next beat.
            let to_next = ((1.0 - (host.ppq - host.ppq.trunc())) * self.spq) as i32;
            let d = (to_next as f64 + to_next as f64) / self.spq;
            if d <= 1.0 {
                self.phaser.set_phase(d as f32);
                self.phaser.set_direction(-1);
            } else {
                self.phaser.set_phase((2.0 - d) as f32);
                self.phaser.set_direction(1);
            }
        }
        let p = &mut self.phaser;
        p.set_rate(rate as f32);
        p.set_spread(spread);
        p.set_range(c.low, c.high);
        p.set_feedback(c.feedback);
        p.set_sample_rate(self.sr as f32);
        p.set_stages(c.notches * 2);
        p.set_oneshot_end(c.oneshot_end);
        p.set_oneshot_direction(c.oneshot_dir);
        p.set_depth(c.wet_dry);

        match self.layout {
            Layout::Mono => {
                p.process_mono(&mut self.store, l);
                self.store.user_update = 0;
                self.agc.process(&mut [l]);
            }
            Layout::MonoToStereo | Layout::Stereo => {
                p.set_offset(c.stereo_offset as f32 / 180.0);
                p.process_stereo(&mut self.store, l, r);
                self.store.user_update = 0;
                self.agc.process(&mut [l, r]);
            }
        }
    }

    /// The editor holds the yellow ball at `pos` (the oval's 0…1 position);
    /// `None` releases it (a one-shot then travels to its socket again).
    pub fn hold(&mut self, held: Option<(f32, i32)>) {
        match held {
            Some((pos, dir)) => {
                self.store.hold = 1;
                self.store.uif_pos = pos.clamp(0.0, 1.0);
                self.store.uif_dir = dir;
            }
            None => {
                if self.store.hold != 0 {
                    self.store.hold = 0;
                    self.store.at_end = 0;
                }
            }
        }
    }

    pub fn trace(&self) -> String {
        let p = &self.phaser;
        format!(
            "phase={:.7} dir={} rate={:.9} lo={:.7} hi={:.7} fb={:.6} tick={} lfo_pos={:.7} lfo_dir={} rng={:#x} fbl={:.7} fbr={:.7} agc={:.7}",
            p.phase, p.dir, p.rate, p.lo, p.hi, p.feedback, p.tick, self.store.lfo_pos, self.store.lfo_dir, p.rng,
            p.fb[0], p.fb[1], self.agc.gain
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize) -> Vec<f32> {
        let mut x = 1u32;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect()
    }

    #[test]
    fn allpass_sections_are_allpass() {
        let c = section(0.3, 0.0);
        // |H(e^jw)| = 1 at a few frequencies
        for w in [0.1f64, 0.7, 1.9, 3.0] {
            let z1 = num(w, 1.0);
            let num_ = (c.b0 as f64, c.b1 as f64, c.b2 as f64);
            let den = (1.0, c.a1 as f64, c.a2 as f64);
            let mag = |k: (f64, f64, f64)| {
                let re = k.0 + k.1 * z1.0 + k.2 * (2.0 * w).cos();
                let im = -(k.1 * w.sin() + k.2 * (2.0 * w).sin());
                (re * re + im * im).sqrt()
            };
            assert!((mag(num_) / mag(den) - 1.0).abs() < 1e-3, "w={w}");
        }
        fn num(w: f64, _: f64) -> (f64, f64) {
            (w.cos(), w.sin())
        }
    }

    #[test]
    fn makes_notches_and_stays_bounded() {
        let mut d = DeepPhaseNine::new(44100.0, Layout::Stereo);
        let c = Controls { feedback: 0.95, notches: 24, ..Controls::default() };
        let x = noise(44100);
        let (mut l, mut r) = (x.clone(), x.clone());
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            d.process(&c, &HostTime::default(), a, b);
        }
        assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() < 1.5));
        assert!(l.iter().zip(&x).any(|(a, b)| (a - b).abs() > 1e-3));
    }

    #[test]
    fn lfo_speed_range_matches_the_manual() {
        let c = Controls { lfo_speed: 0, ..Controls::default() };
        assert_eq!(c.lfo_rate_hz(44100.0, 22050.0), 1.0 / 256.0);
        let c = Controls { lfo_speed: 60, ..Controls::default() };
        assert_eq!(c.lfo_rate_hz(44100.0, 22050.0), 16.0);
    }
}

/// The original's `Process` runs inside `DenormalFlush` (flush-to-zero and
/// denormals-are-zero in MXCSR). The plug-in sets the same mode on the audio
/// thread; this sets it for offline use. Returns the previous state.
pub fn flush_denormals(on: bool) -> bool {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fpcr: u64;
        std::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        let was = fpcr & (1 << 24) != 0;
        if on { fpcr |= 1 << 24 } else { fpcr &= !(1 << 24) }
        std::arch::asm!("msr fpcr, {}", in(reg) fpcr);
        was
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    unsafe {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::{_mm_getcsr, _mm_setcsr};
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        #[allow(deprecated)]
        let csr = _mm_getcsr();
        let was = csr & 0x8040 != 0;
        #[allow(deprecated)]
        _mm_setcsr(if on { csr | 0x8040 } else { csr & !0x8040 });
        was
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64", target_arch = "x86")))]
    {
        let _ = on;
        false
    }
}
