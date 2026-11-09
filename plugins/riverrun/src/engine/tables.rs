//! Every constant table of the Riverrun engine, read from the i386 slice of
//! `Riverrun.vst` 2.6.4. Addresses refer to that slice (see
//! `reverse_engineering/riverrun/README.md`).

/// The breakpoint interpolator `raprsrnunraeegybghbdzrvqypyotzm(short, float*, short)`
/// @ 0x4786. `table` holds `(x, y)` pairs; `x` is the control position, already
/// truncated to an integer (the original passes a `short`), so every control has
/// the resolution of one pixel of its 1999 slider.
pub fn breakpoint(x: i16, table: &[f32]) -> f32 {
    let n = table.len();
    let f = x as i32 as f32;
    if table[0] < f {
        if f < table[n - 2] {
            let mut s = 0;
            while s < n && table[s] <= f {
                s += 2;
            }
            ((f - table[s - 2]) / (table[s] - table[s - 2])) * (table[s + 1] - table[s - 1])
                + table[s - 1]
        } else {
            table[n - 1]
        }
    } else {
        table[1]
    }
}

/// The original truncates a float parameter to `short` before every lookup
/// (`(short)(int)value`).
#[inline]
pub fn to_short(v: f32) -> i16 {
    (v as i32) as i16
}

/// `GetPitch()` @ 0x1f992: Pitch 0…1000 → playback ratio 1/8 … 8 (six octaves).
pub const PITCH_MAP: [f32; 12] = [0.0, 0.125, 200.0, 0.25, 350.0, 0.5, 500.0, 1.0, 750.0, 2.0, 1000.0, 8.0];

/// `GetRandom()` @ 0x1fa2a: Pitch Random 0…100 → width of the random band,
/// *added to the ratio* (not to semitones).
pub const RANDOM_MAP: [f32; 10] = [0.0, 0.0, 20.0, 0.1, 50.0, 1.0, 90.0, 4.0, 100.0, 16.0];

/// `GetGrainLength()` @ 0x1f8f8: Grain length 1…286 → samples at 44.1 kHz.
pub const LENGTH_MAP: [f32; 10] = [0.0, 50.0, 50.0, 700.0, 125.0, 3000.0, 200.0, 7000.0, 285.0, 15000.0];

/// `GetTimingRandom()` @ 0x1faae: Grain speed random 0…286 → onset jitter in
/// samples at 44.1 kHz.
pub const TIMING_MAP: [f32; 8] = [0.0, 1.0, 50.0, 100.0, 150.0, 2000.0, 285.0, 30000.0];

/// `Process` @ 0x204c6, Flowing mode: Grain speed 1…286 → grains per second.
pub const FLOWING_MAP: [f32; 10] = [0.0, 1.0, 50.0, 10.0, 125.0, 200.0, 200.0, 800.0, 285.0, 2000.0];

/// `Process::C_180`: Rhythmic mode grain spacing, as `(num, den)` with the
/// spacing in quarter notes = `den / num`. Slider index = `round(speed / 21.142857)`.
pub const RHYTHM_DIVISIONS: [(f64, f64); 14] = [
    (1.0, 4.0),  // whole
    (2.0, 6.0),  // dotted half (3 quarters)
    (1.0, 2.0),  // half
    (3.0, 4.0),  // half triplet
    (1.0, 1.0),  // quarter
    (3.0, 2.0),  // quarter triplet
    (2.0, 1.0),  // eighth
    (3.0, 1.0),  // eighth triplet
    (4.0, 1.0),  // 16th
    (6.0, 1.0),  // 16th triplet
    (8.0, 1.0),  // 32nd
    (12.0, 1.0), // 32nd triplet
    (16.0, 1.0), // 64th
    (32.0, 1.0), // 128th
];

pub const RHYTHM_LABELS: [&str; 14] = [
    "1/1", "1/2.", "1/2", "1/2T", "1/4", "1/4T", "1/8", "1/8T", "1/16", "1/16T", "1/32", "1/32T",
    "1/64", "1/128",
];

/// Envelope menu (param 7, values 1…4), names from `ppixlhex…::GetValueString`.
pub const ENVELOPE_NAMES: [&str; 4] = ["Triangle", "Square", "Sharp attack", "Cosine"];

/// Pitch Grid menu (param 8, values 1…23), from the `Riverrun.rsrc` menu.
pub const GRID_NAMES: [&str; 23] = [
    "seamless (no grid)",
    "minor 7th chord",
    "major 7th chord",
    "dominant 7th chord",
    "min 5 chord 'deluxe'",
    "maj 5 chord 'deluxe'",
    "min 4 chord 'deluxe'",
    "maj 4 chord 'deluxe'",
    "min/maj 9th chord",
    "octaves and fifths",
    "diatonic major",
    "chromatic",
    "whole note",
    "minor thirds",
    "major thirds",
    "fourths",
    "diminished fifths",
    "fifths",
    "minor sixths",
    "major sixths",
    "minor sevenths",
    "major sevenths",
    "octaves",
];

/// The semitone constant of `pow(1.0594630943592953, n)` used by `CreateScales`.
pub const SEMITONE: f64 = 1.059_463_094_359_295_3;

pub const GRIDS: usize = 23;
/// Slots per grid in the engine object (`this + 0x80 + (grid*100 + i)*8`).
pub const GRID_SLOTS: usize = 100;

/// MIDI notes (60 = unison) of the chord grids, `CreateScales` @ 0x1b55e.
/// The static arrays `C_138…C_143` and the inline ones are reproduced verbatim,
/// including the doubled 62 of the 9th chord.
const CHORDS: [&[i16]; 10] = [
    // 1 minor 7th (C_138)
    &[24, 27, 31, 34, 36, 39, 43, 46, 48, 51, 55, 58, 60, 63, 67, 70, 72, 75, 79, 82, 84, 87, 91, 94, 96, 99, 103, 106],
    // 2 major 7th (C_139)
    &[24, 28, 31, 35, 36, 40, 43, 47, 48, 52, 55, 59, 60, 64, 67, 71, 72, 76, 79, 83, 84, 88, 91, 95, 96, 100, 103, 107],
    // 3 dominant 7th (C_140)
    &[24, 28, 31, 34, 36, 40, 43, 46, 48, 52, 55, 58, 60, 64, 67, 70, 72, 76, 79, 82, 84, 88, 91, 94, 96, 100, 103, 106],
    // 4 min 5 'deluxe'
    &[24, 36, 43, 50, 51, 58, 65, 70, 77, 82, 89],
    // 5 maj 5 'deluxe'
    &[24, 36, 43, 50, 52, 59, 66, 67, 74, 76, 83, 90],
    // 6 min 4 'deluxe' (C_141)
    &[24, 36, 39, 41, 46, 50, 51, 53, 58, 62, 63, 65, 70, 74, 75, 77, 82, 86],
    // 7 maj 4 'deluxe'
    &[24, 36, 40, 45, 50, 54, 59, 64, 69, 74, 78],
    // 8 min/maj 9th (C_142)
    &[24, 36, 39, 43, 47, 50, 51, 55, 59, 62, 62, 67, 71, 74, 75, 79, 83, 86, 87, 91, 95],
    // 9 octaves and fifths
    &[24, 31, 36, 43, 48, 55, 60, 67, 72, 79, 84, 91, 96, 103],
    // 10 diatonic major (C_143)
    &[24, 26, 28, 29, 31, 33, 35, 36, 38, 40, 41, 43, 45, 47, 48, 50, 52, 53, 55, 57, 59, 60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84, 86, 88, 89, 91, 93, 95, 96, 98, 100, 101, 103, 105, 107],
];

/// The search bound the original stores for each grid (`this + 0x5e40 + grid*2`).
///
/// It is used as `do { if p <= t[i] break; } while (i++ < count)`, so the search
/// can reach index `count`. For most chord grids that slot was never written:
/// see [`Scales`].
pub const GRID_COUNTS: [i16; GRIDS] = [
    0, 28, 28, 28, 11, 12, 18, 11, 21, 14, 49, 85, 42, 28, 21, 17, 14, 12, 10, 9, 8, 7, 7,
];

/// The pitch grids as playback ratios, exactly as `CreateScales` stores them:
/// `(double)(float)pow(1.0594630943592953, note - 60)`.
///
/// Slots the original never writes are 0.0 here. The engine object is fresh
/// heap memory, so a grain whose random pitch lands above the top of a chord
/// grid reads that slot and plays at ratio 0: a one-second DC "thump" built from
/// a single input sample. That is what the original does on a clean heap and it
/// is reproduced (see the reverse-engineering notes, §7).
#[derive(Clone)]
pub struct Scales {
    pub ratio: Vec<[f64; GRID_SLOTS]>,
}

impl Default for Scales {
    fn default() -> Self {
        Self::new()
    }
}

fn ratio(semitones: f64) -> f64 {
    SEMITONE.powf(semitones) as f32 as f64
}

impl Scales {
    pub fn new() -> Self {
        let mut ratio = vec![[0.0f64; GRID_SLOTS]; GRIDS];
        for (g, notes) in CHORDS.iter().enumerate() {
            for (i, &n) in notes.iter().enumerate() {
                ratio[g + 1][i] = self::ratio((n - 60) as f64);
            }
        }
        // 11: chromatic, -36 … +48.
        for (i, st) in (-36..=48).enumerate() {
            ratio[11][i] = self::ratio(st as f64);
        }
        // 12…22: equal intervals of 2…12 semitones from -36, as far as the
        // original loop bound goes (0x56, 0x57, 0x58, 0x55, 0x5a, 0x5b, 0x58,
        // 0x5a, 0x5a, 0x58, 0x60 for steps 2…12).
        const ENDS: [i32; 11] = [0x56, 0x57, 0x58, 0x55, 0x5a, 0x5b, 0x58, 0x5a, 0x5a, 0x58, 0x60];
        for (k, &end) in ENDS.iter().enumerate() {
            let step = k as i32 + 2;
            let mut i = 0;
            while i != end {
                ratio[12 + k][(i / step) as usize] = self::ratio((i - 36) as f64);
                i += step;
            }
        }
        Self { ratio }
    }

    /// The snapping search inlined in `Run` (and `FindClosestPitch` @ 0x1ac76):
    /// the first grid ratio that is `>= p`, searching up to index `count`.
    #[inline]
    pub fn snap(&self, grid: i16, p: f32) -> f32 {
        let g = grid as usize;
        let count = GRID_COUNTS[g];
        let pd = p as f64;
        let mut i: i16 = 0;
        loop {
            if pd <= self.ratio[g][i as usize] {
                break;
            }
            let more = i < count;
            i += 1;
            if !more {
                // The do/while exits with i == count + 1, but the original
                // reads the slot computed before the increment.
                i -= 1;
                break;
            }
        }
        self.ratio[g][i as usize] as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints() {
        assert_eq!(breakpoint(500, &PITCH_MAP), 1.0);
        assert_eq!(breakpoint(0, &PITCH_MAP), 0.125);
        assert_eq!(breakpoint(1000, &PITCH_MAP), 8.0);
        assert_eq!(breakpoint(625, &PITCH_MAP), 1.5);
        assert_eq!(breakpoint(286, &FLOWING_MAP), 2000.0);
        assert_eq!(breakpoint(100, &LENGTH_MAP), 700.0 + 50.0 / 75.0 * 2300.0);
    }

    #[test]
    fn grids() {
        let s = Scales::new();
        // minor 7th: the lowest entry is three octaves down.
        assert_eq!(s.ratio[1][0], 0.125);
        assert_eq!(s.snap(1, 0.9), 1.0);
        // Above the top of a chord grid the original reads an unwritten slot.
        assert_eq!(s.snap(1, 20.0), 0.0);
        // Whole-note grid: index 42 is the last written slot (+48 semitones).
        assert_eq!(s.snap(12, 20.0), 16.0);
        assert!((s.snap(11, 1.03) as f64 - SEMITONE).abs() < 1e-6);
    }
}
