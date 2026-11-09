//! Presets recovered from Audio Ease's own "Riverrun Demo" song for Digital
//! Performer (Nautilus Bundle 1.0, Mac OS 9). The song stores each Riverrun
//! setting as a list of MAS parameter records `[0][id u32][len u32][value]`;
//! see `reverse_engineering/riverrun/README.md` §15.
//!
//! The song also references the frozen screens ("RiverRun Temp Snap 51876",
//! "…61716"), which were not on the disc; every one of them was taken from the
//! song's only audio file, the Voice, so applying a preset loads the Voice.
//! In the song Record was sometimes left on (the Voice track was streaming
//! in); presets here always leave Record off.

/// Values in the original units (see `engine::Controls`).
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub about: &'static str,
    pub pitch: f32,
    pub speed: f32,
    pub length: f32,
    pub random: f32,
    pub speed_random: f32,
    pub walk: f32,
    /// 1…4
    pub envelope: u32,
    /// 1…23
    pub grid: u32,
    pub rhythmic: bool,
    pub mono_stereo: f32,
    pub glass: f32,
    pub needle_l: f32,
    pub needle_r: f32,
    pub fine: f32,
}

pub const PRESETS: [Preset; 4] = [
    Preset {
        name: "Riverrun Demo (track)",
        about: "the state of the plug-in on the voice track of the demo song",
        pitch: 500.0,
        speed: 152.40625,
        length: 143.5,
        random: 0.0,
        speed_random: 71.5,
        walk: 0.0,
        envelope: 2,
        grid: 1,
        rhythmic: false,
        mono_stereo: 0.0,
        glass: 470.0,
        needle_l: 0.0,
        needle_r: 472.0,
        fine: 1.0,
    },
    Preset {
        name: "probeer de klant",
        about: "\"try the client\" — Audio Ease's own preset name, in Dutch",
        pitch: 513.333_3,
        speed: 176.0,
        length: 152.0,
        random: 6.666_667,
        speed_random: 0.0,
        walk: 0.0,
        envelope: 2,
        grid: 1,
        rhythmic: false,
        mono_stereo: 92.0,
        glass: 453.0,
        needle_l: 0.0,
        needle_r: 472.0,
        fine: 1.0,
    },
    Preset {
        name: "probeer de klant 2",
        about: "walking forwards between the needles",
        pitch: 477.678_56,
        speed: 152.40625,
        length: 143.5,
        random: 6.666_667,
        speed_random: 54.0,
        walk: 1.047_297_5,
        envelope: 2,
        grid: 1,
        rhythmic: false,
        mono_stereo: 93.0,
        glass: 470.0,
        needle_l: 145.0,
        needle_r: 328.0,
        fine: 1.0,
    },
    Preset {
        name: "drumloop 3",
        about: "sparse sharp-attack grains on the minor-sixths grid",
        pitch: 386.666_66,
        speed: 23.0,
        length: 166.0,
        random: 44.0,
        speed_random: 1.0,
        walk: 0.500_892,
        envelope: 3,
        grid: 18,
        rhythmic: false,
        mono_stereo: 93.0,
        glass: 156.0,
        needle_l: 207.0,
        needle_r: 255.0,
        fine: 1.0,
    },
];

pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}
