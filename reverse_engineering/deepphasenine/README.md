# Reverse engineering Deep Phase Nine

How the phaser of **Deep Phase Nine** (Audio Ease, Nautilus Bundle, 2000–2013;
interface by Captain Video) was recovered from the original machine code, and
how `plugins/deepphasenine/src/engine.rs` was proven **bit-identical** to it.
Method and tools are those of [Riverrun](../riverrun/README.md); the shared
tools live in [`../tools`](../tools).

| Path | What it is |
|---|---|
| `original_binaries/DeepPhaseNine-2.6.4-VST-i386` | i386 slice of `DeepPhaseNine.vst` 2.6.4 — the reference |
| `original_binaries/DeepPhaseNine-2.6.4-VST-ppc` | its PowerPC slice |
| `original_binaries/DeepPhaseNine.rsrc` | resources (menus, help texts) |
| `tools/emulate_deepphasenine.py` | runs the original `InitBuffers` / `Process` under Unicorn |
| `tools/differential_test.py` | 39 scenarios, original vs Rust, bit for bit |
| `ui/` | the original panel, about box and balloon-help images |

## 1. Map

Same obfuscated "Mission" framework as Riverrun — the shared classes even keep
the same random names. Specific to Deep Phase Nine:

| Class | Role |
|---|---|
| `jwtrfajrcvfykpiwlfqjsktwwtletfm` | the process: `Process` @ 0x1c784, `InitBuffers` @ 0x1e104, getters, `ReceiveMidi` |
| `bqoibkkycavvbtrbgpvkncpnvmxives` | the phaser: `UpdateOneBuffer` @ 0x1aed0 (mono), `UpdateTwoBuffers` @ 0x1b616 (stereo), `lfoShape`, `Rate`, `Range`, … |
| `orjogkbvlnlbjeosxawstygikcennhy` | description: `GetParameter` @ 0x19e56, `GetPrivateParameter` @ 0x1a60c, 12 variants |
| `yruwg…` (shared) | the AGC, used here as the look-ahead peak limiter |

## 2. Parameters

| Id | Name | Range | Default | MIDI |
|---|---|---|---|---|
| 0 | Notch count | 1…24 (long) | 3 | CC 18, note → `note mod 24 + 1` |
| 1 | Feedback | −0.95…0.95 | 0.8 | CC 1 |
| 2 | Low | 0.01…2000 Hz | 100 | CC 11 |
| 3 | High | 200…20000 Hz | 4000 | CC 12 |
| 4 | Spread | 0…100 | 35 | CC 13 |
| 5 | WetDry | 0…1 | 1 | CC 14 |
| 6 | LFO Choice | 1…5 | 1 | CC 17 |
| 7 | LFO Speed | 0…70 (long) | 21 | CC 15 |
| 8 | One shot end | 0…1 | 0.5 | |
| 9 | LFO Direction (of a one-shot) | −1…1 | −1 | |
| 10 | Sync to beats | off/on | off | |
| 11 | Stereo offset | −180…180° | 0 | CC 16 |

Private parameters, through which the phaser talks to itself and to the editor:
`0x0c` *LFO Position* (float 0…1000), `0x0d` *UIF Position* (the dragged
ball), `0x0e…0x11` coefficient pointers for the display, `0x12` *LFO
Direction*, `0x13` *UIF Direction*, `0x14` *User update* (shape changed),
`0x15` *At end* (one-shot arrived), `0x16` *Hold* (ball held).

The phaser **writes its LFO value into "LFO Position" and reads it back** to
compute the left channel's filter, so the store's rules are part of the sound:
a float only changes when it is inside its range and different, a long only
when different, and `GetParamLong` on a float parameter returns 0. The
emulator therefore uses the binary's own parameter objects (§6).

## 3. Process

Per host block: tempo from the host; `spread = max(Spread, 2) / 200`;
`lfoShape(choice)`; LFO speed

* free: `2^(speed/5) / 256` Hz — 1/256 to 16 Hz, as the manual says (×8 for
  Sample & Hold);
* sync: `sr / (samples-per-quarter · T[0x54 − speed])`, `T` = 85 beat lengths
  from 1/32 to 128 quarters with dotted and triplet values (`Process::C_149`);
  Sample & Hold uses `T[0x3f − speed]`, which runs off the table above speed
  63 (clamped in the port). Only Sample & Hold restarts its phase from the
  next beat on a transport change or speed change.

Then `Range(low, high)` (normalised to Nyquist), feedback, `NumStages =
2·notches`, depth; one phaser per channel pair (variants: mono, mono→stereo,
stereo, and quad / 5.0 / 5.1 with LFOs a third apart); the AGC with
`InitAGC(1.0, 1.0, 10^−0.003, 10^(18/(10·sr)), 0.5, 300)` — a peak limiter
at −6 dBFS, −0.06 dB per sample of attack, +36 dB/s release, 300 samples of
look-ahead (the latency). The whole `Process` runs inside `DenormalFlush`
(MXCSR flush-to-zero + DAZ); the port sets the same mode.

## 4. The phaser

Every **123 samples** (control rate) the LFO advances and the coefficients
are recomputed:

* the phase bounces between 0 and 1 (`phase += dir · rate`, folded at the
  ends), `rate = (Hz / sr) · 246`;
* shapes: **Sine** publishes `(1 − cos πφ)/2`, **Triangle** φ,
  **Sample & Hold** a new LCG value (`x·1664525 + 1013904223`, seed
  `0x23db21`, value `x / 4294967295.0`, direction from bit 4) each time the
  phase bounces at 0; **OneShot Sine/Triangle** stop at the socket
  (`One shot end`, via `acos(−(2e−1))/π` for the sine) and set *At end*;
* the left chain's centre is `w = lfo · (high − low) + low`; the right chain
  uses the phase shifted by `direction · offset/180`, folded the same way;
* each section is a second-order **all-pass**: poles at radius
  `r = 1 − 0.95·spread` and angle `πw`, zeros at the reciprocal radius,
  normalised by `g = (1 + a1 + a2) / (1 + b1 + b2)` — computed with `cosf` /
  `sinf` in single precision, `r` in double.

Per sample: `u = x + feedback · s`, `y = A^N(u)` through `notches` identical
sections, output `x + depth · y`. The feedback state `s` of the **left**
channel is smoothed, `s = 0.1·y + 0.9·s` in double — but not while the
**Option key** is held (`OptionKeyDown()` is called from the DSP!); the right
channel and the mono version use `s = y` unsmoothed. Kept bit for bit, minus
the Option key.

Two more quirks kept: the mono version reads the dragged ball's position with
`GetParamLong` (always 0), so holding the ball in mono with a sine snaps the
phase to 0; and both versions read "LFO Position" as a long when folding a
negative phase (always 0).

## 5. Verification

`tools/emulate_deepphasenine.py` executes the original constructor,
`InitBuffers` and `Process`. Every parameter object is built by the binary's
own `GetParameter` / `GetPrivateParameter`; only the `std::map` lookup
`GetParamFromMap` is answered from Python, plus a fake host object for
`GetSettings`, `GetSampleRate`, and the usual imports (`acos`, `cosf`, `sinf`
are answered through double precision, exactly like the port). The harness
also binds the bundle's non-lazy pointers to its own weak symbols (a C++
vtable is reached that way), which dyld would do at load time.

```
$ python tools/differential_test.py original_binaries/DeepPhaseNine-2.6.4-VST-i386 \
      ../../target/release/dpn-render /tmp/dpn
defaults, lfo-1…5, notches-1/2/6/12/24, feedback ±0.95, spread-max,
range-extreme, range-inverted, wet-dry, offset ±, fastest LFO and S&H,
one-shots up/down/stopped, automation (shape changes mid-run),
sync for sine/triangle/S&H, S&H speed changes, 97 bpm,
mono, mono-feedback, mono→stereo, blocks of 64 and 1024,
1 024 000-frame run, 48 and 96 kHz                         IDENTICAL

39 identical, 0 different
```

## 6. Mac OS 9 (v1.0)

`reference/macos9/Deep Phase Nine™` shares the parameter ranges and defaults
and the limiter constants (−0.003, 18, 300), but its sync is built
differently (no 85-entry table; a 25…12800 scale instead, probably MAS
ticks), and neither the `2^(speed/5)` curve nor the `/4294967295` of the
Sample & Hold appear. The 2000 phaser is therefore not the same code as
2.6.4's; executing it (PowerPC/CFM emulation) is an open item.

## 7. What the port changes

| Original | Port |
|---|---|
| The Option key disabled the left feedback smoothing | not reproduced (a keyboard state is not a parameter) |
| S&H sync index runs off its table above speed 63 | clamped to the first entry |
| Quad / 5.0 / 5.1 variants | mono, mono→stereo, stereo |
| "WetDry" | called Mix; same law (`x + depth·y`) |
| — | Output gain (0 dB = original) |
