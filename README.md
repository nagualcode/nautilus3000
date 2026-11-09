# Nautilus Bundle

```
  _  _   _  _   _ _____ ___ _   _   _ ___   ___ _   _ _  _ ___  _    ___
 | \| | /_\| | | |_   _|_ _| | | | | / __| | _ ) | | | \| |   \| |  | __|
 | .` |/ _ \ |_| | | |  | || |_| |_| \__ \ | _ \ |_| | .` | |) | |__| _|
 |_|\_/_/ \_\___/  |_| |___|____\___/|___/ |___/\___/|_|\_|___/|____|___|

  AUDIO EASE 2000–2013 • REBUILT FROM THE ORIGINAL BINARIES • CLAP / VST3 / AU
```

A modern recreation of **Audio Ease's Nautilus Bundle** — Riverrun, Deep Phase
Nine and PeriScope — the three "watery" plug-ins that started life as MAS
plug-ins for Digital Performer on Mac OS 9 and were sold until 2013.
Each one is rebuilt from the original machine code, in Rust, with the
[truce](https://github.com/truce-audio/truce) framework.

| Plug-in | What it was | Status |
|---|---|---|
| **Riverrun** | real-time granular synthesizer, "by the maker of Thonk" | ✅ engine bit-identical to the original; plug-in, UI, CLI |
| Deep Phase Nine | 24-notch phaser with look-ahead limiter | planned |
| PeriScope | 32-band phase-correct zoomable EQ with spectroscope | planned |

---

## Riverrun

![Riverrun, modern theme](docs/riverrun.png)

> *Audio from the sequencer's tracks or directly from a disk file can be
> streamed into and frozen in Riverrun. Tiny particles of the audio, called
> grains, are pushed through an envelope generator, pitched and scattered across
> the output channels according to the surface controls.* — Audio Ease

Press **●** while the track plays and the audio streams into the input screen,
from right to left; press it again and the screen freezes. (Or **load** a file,
drop one on the screen, or pick from **demo & presets**: the original spoken
**Voice** from Audio Ease's 2000 demo song, and the presets recovered from that
song — *probeer de klant*, *probeer de klant 2*, *drumloop 3* and the demo
track's own setting.) From then on Riverrun picks
grains from under the **Grain Glass** and throws them at you:

| Control | What it does |
|---|---|
| **Grain Glass** | where grains come from. Drag it; it **walks** between the red and yellow **needles** (drag them at the top edge) |
| **Walk** | speed of the glass in × real time, −5…+5. `0` halts it, 0.01 is the manual's "microscope at a hundredth of normal speed" |
| **Shape** | grain envelope: Triangle, Square (default), Sharp attack, Cosine |
| **Mono / stereo** | every grain goes hard left or hard right; mono folds both sides together |
| **Flowing / Rhythmic** | free texture, or grains locked to the host tempo (1/1 … 1/128, black fields = even, grey = triplets) |
| **Grain speed / random / length** | density (1–2000 grains/s), onset scatter, grain size (1–340 ms) |
| **Pitch grid** | 23 grids — seamless, 7th / 9th / "deluxe" chords, diatonic, chromatic, equal intervals |
| **Pitch / Random / Fine tune** | the dice: each grain gets a pitch from the blue band on the keyboard, snapped up to the grid's red keys |

Not in the original, defaults = original behaviour: **Mix**, **Output**,
**Grain clock** (see below), **Beat sync** (Original / Tight). The input screen
is saved inside the project. Classic theme:

![Riverrun, classic theme](docs/riverrun-classic.png)

### MIDI

As in version 2.0.3: CC 1 mono/stereo, CC 11/12 red/yellow needle, CC 13 walk,
CC 14 grain speed, CC 15 pitch, CC 16 pitch random, CC 17 grain speed random,
CC 18 grain length; **notes** place the Grain Glass (MIDI note 36 = far left, five
octaves across, wrapping); **program change** picks the envelope.

### How faithful is it?

**Bit for bit.** The engine is a translation of the i386 code of
`Riverrun.vst` 2.6.4 (class names were obfuscated by the copy protection; the
method names were not). To prove it, the original machine code is *executed* —
constructors, `InitBuffers`, `Process`, the grain generator, the AGC — under the
Unicorn CPU emulator, and its output is compared with the Rust engine's:

```
57 scenarios — every envelope, all 23 pitch grids at extreme settings, walking
in both directions, needles, long grains at the end of the screen, automation,
Rhythmic mode at several tempos and divisions, live recording, 64- to 1024-sample
blocks, 1.28 M-sample runs, 44.1 / 48 / 96 kHz:  57 identical, 0 different
```

That includes the original's quirks, which are part of its sound: one grain
per buffer copied to every onset in that buffer, a read position accumulated in
single precision, a mistyped π (3.1415926528) in the Cosine envelope, an AGC
that settles at −10 dB and never comes back, and the occasional one-second
"thump" when pitch + random overshoot a chord grid. All documented, with
addresses, in **[reverse_engineering/riverrun](reverse_engineering/riverrun/README.md)**,
which also compares the Mac OS 9 build (v1.0, 2000): same curves, same π typo,
but a minor-7th grid whose top notes were a semitone sharp and a fixed
204 800-sample screen.

**Grain clock.** The original made one grain per *host* buffer, so the same
song sounded different at different buffer sizes. Here that buffer is a
parameter (default 512); the sound no longer depends on the host.

### Command line

```bash
riverrun-render input.wav out.wav --seconds 30 --grid 2 --pitch 380 --random 45 \
    --envelope 4 --length 200 --speed 180 --walk 0.08     # a monk choir
riverrun-render voice out.wav --preset "probeer de klant 2"   # Audio Ease's demo + preset
riverrun-render sung out.wav --rhythmic --bpm 112 --speed 190 --grid 11
riverrun-render --list                                    # envelopes, grids, presets
```

About 1000× real time; the heaviest setting of all (2000 grains/s of one-second
grains, ~1000 layers — "can easily crash a G3 or G4") runs at 60×.

## Build

Requirements: Rust 1.85+, macOS 12+ (Linux and Windows builds are untested so
far), and [`cargo-truce`](https://github.com/truce-audio/truce).

```bash
cargo test --release                                  # unit + plug-in tests
cargo run --release -p riverrun --bin riverrun-standalone
cargo truce install --clap --vst3 --au2 --user        # into ~/Library/Audio/Plug-Ins
auval -v aumf RvRn JdSn                               # passes
```

## Repository

```
nautilusbundle/
├── plugins/riverrun/src/
│   ├── engine/            # the translated engine: mod.rs (Process + Run), ring.rs,
│   │                      #   envelope.rs, agc.rs, tables.rs
│   ├── lib.rs             # plug-in glue: grain clock, MIDI, snippet, dry/wet
│   ├── params.rs, ui.rs   # parameters (original units) and the egui editor
│   ├── snippet.rs         # the input screen, saved with the project
│   ├── audio.rs, demo.rs  # WAV/AIFF I/O, the demo voice
│   └── bin/riverrun_render.rs
├── reference/             # the original material: macos9/, macosX/, review, web pages
├── reverse_engineering/riverrun/
│   ├── README.md          # the write-up
│   ├── original_binaries/ # i386 + ppc slices of Riverrun.vst 2.6.4, resources
│   ├── decompiled/        # Ghidra output, symbols, import stubs
│   ├── tools/             # emulator, differential test, Mach-O / BinHex helpers
│   └── ui/                # the original panel
└── docs/                  # screenshots
```

The archive this started from — installers (`reference/macosX`), the Mac OS 9
plug-ins, manual and demo song (`reference/macos9`), the *Sound on Sound* review
and Audio Ease's web pages — is in `reference/`.

## License and credits

- Modern implementation: MIT © 2026 nagualcode / Jardim Sonoro.
- Riverrun, Deep Phase Nine, PeriScope: © 1995–2013 **Audio Ease B.V.**;
  Riverrun by the maker of Thonk (Arjen van der Schoot); interface by Captain
  Video. "Thank you Mr. Barry Truax" — from Riverrun's about box.
- Audio plug-in framework: [truce](https://github.com/truce-audio/truce).
