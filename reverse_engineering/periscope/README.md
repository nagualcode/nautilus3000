# Reverse engineering PeriScope

How the equaliser of **PeriScope** (Audio Ease, Nautilus Bundle, 2000–2013;
interface by Captain Video) was recovered from the original machine code, and
how `plugins/periscope/src/engine.rs` was proven **bit-identical** to it.
Method and tools are those of [Riverrun](../riverrun/README.md) and
[Deep Phase Nine](../deepphasenine/README.md); the shared tools live in
[`../tools`](../tools).

| Path | What it is |
|---|---|
| `original_binaries/Periscope-2.6.4-VST-i386` | i386 slice of `Periscope.vst` 2.6.4 — the reference |
| `original_binaries/Periscope-2.6.4-VST-ppc` | its PowerPC slice |
| `decompiled/periscope_i386.c`, `symbols.txt` | Ghidra output and symbol table of the i386 slice |
| `tools/emulate_periscope.py` | runs the original `InitBuffers` / `Process` under Unicorn |
| `tools/differential_test.py` | 28 scenarios, original vs Rust, bit for bit |
| `ui/` | the original panel (PICT 2000) and about box |

## 1. Map

The obfuscated "Mission" framework again, with the shared classes under the
same random names as in the other two plug-ins. PeriScope's own:

| Symbol | Role |
|---|---|
| `rzivejahtvmzyhvlytgfbtensfyssyn` | the process: ctor @ 0x1a198 (0x2ac bytes), `InitBuffers` @ 0x1a83e, `UpdateFreqs` @ 0x1ac9a, `Process` @ 0x1b674, `ReceiveMidi` @ 0x1a4e4, `GetLatency` @ 0x1a4da |
| `wdncxlttwddkppttcetairushhjjywf` | description: `GetParameter` @ 0x1c208, `GetPrivateParameter` @ 0x1c68c, `MakeProcess` @ 0x1cbbe |
| `thumbs2fftcoefsHerm` @ 0x16bb4 | fader gains → spectrum (a staircase) |
| `halfShiftAndHanPad` @ 0x198ee, `hanning` @ 0x177b8 | impulse → centred, windowed FIR |
| `pxyvz…` @ 0x16cc4 | packed complex multiply (the convolution) |
| `CVecLibFFT` @ 0x1d7b0… | vecLib's `create_fftsetup` / `ctoz` / `fft_zrip` / `ztoc` |
| `raprsrn…` @ 0x4556 | the framework's breakpoint interpolator |
| `PeriscopeControl` | the fader screen and overview (`ClickInFaders` @ 0x16d50, `Draw` @ 0x1783e) |
| `PeriscopeEditMode` | the five edit modes and their modifier keys (`Idle` @ 0x19d40) |

## 2. Parameters

| Id | Name | Range | Default | MIDI |
|---|---|---|---|---|
| 0 | Curve Latency (display release) | 0…0.999 | 0.9 | |
| 1 | Gooeyness (magnetism) | 1…32 | 3 | |
| 2 | loCutOff (red needle) | 5…408 | 40 | CC 64 (`value/128` normalised) |
| 3 | hiCutOff (yellow needle) | 175…412 | 300 | CC 65 |
| 4 | Fader Edit Mode | 0…4 | 1 | |
| 5 | Brightness | 0…1 | 1 | |
| 6…37 | Fader 1…32 | 0…265 | 135 (0 dB) | CC 32…63; notes (below) |

Private ("planet") parameters 0x26…0x31: *Compare Thumbs* (the fader values the
kernel was built from, initialised to −1), *freqs* (33 floats, the band
edges), *Conv Data* (0x4000 bytes: the kernel spectrum), *DrawBuf* (the
spectroscope ring), *Drawing / Redraw flag*, *Sauzig* and *PPM multiplier*,
*Altivec flag* (`Gestalt('ppcf') & 0x10`), *Old lo / hi freq*, *draw fft size*
(4096, 32…16384).

A needle position `p` means `f = nyq · b^(p − 418)` with
`b = (nyq/18)^(1/418)`: 418 steps from 18 Hz to Nyquist (the 1/418 is the
double `0x3f63991c2c187f63`). The faders go through `gThumbBreakPoints`, a
36-float table built by a **static initialiser** at load time
(`__GLOBAL__I_gThumbBreakPoints` @ 0x156a, `(float)pow(10, dB/20)`):

| position | 0 | 3 | 15 | 35 | 55 | 75 | 95 | 115 | 135 | 150 | 165 | 180 | 195 | 210 | 225 | 240 | 255 | 263 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| dB | −∞ | −96 | −48 | −40 | −32 | −24 | −16 | −8 | 0 | +4 | +8 | +12 | +16 | +20 | +24 | +28 | +32 | +34 |

linearly interpolated *in gain*. The screen's faders travel 263 pixels
(0x107), so 264 and 265 are reachable by MIDI only — and sound like 263.

## 3. UpdateFreqs: from faders to a kernel

Called at the top of every `Process`; returns at once if neither needle nor
fader moved. Otherwise:

1. **Needles → frequencies** (`powf`, single precision).
2. **Needles pushed apart.** One FFT bin is `2·nyq/4096` Hz (10.77 Hz at
   44.1 kHz). Below 352 Hz bands are spaced linearly, above it
   logarithmically, and 30 bands of at least one bin each must fit between
   the needles. The lowest the high needle may go (`lo_min_hi`, by `exp`) and
   the highest the low needle may go (searched down in steps of ×0.95, then
   ×0.99, and if under 352 Hz snapped to whole bins with `ceil`) are
   computed; the needle that moved is pushed back. The pushed position goes
   out through `SetParamFloat(2/3)` — which, for an *automatable* parameter,
   only **sends it to the host** as automation; the host writes it back.
3. **Band edges** (`freqs`): if the low needle is under 352 Hz,
   `floor((352 − lo)/step)` linear bands, then a log ramp to the high needle;
   otherwise 31 bands of a constant ratio `(hi/lo)^(1/30)`. The 32nd edge is
   Nyquist.
4. **Staircase spectrum** (`thumbs2fftcoefsHerm`, despite the name): every
   bin up to a band edge gets that band's gain, as a *real* value, in vecLib's
   packed layout. Fader 1 is therefore a low pass (0 Hz…edge 1), fader 32 a
   high pass (edge 31…Nyquist) — the red and yellow faders.
5. **Inverse FFT** → a zero-phase impulse, then `halfShiftAndHanPad`: the
   two `memmove`s are given a size **in bytes** equal to `n`, i.e. `n/4`
   floats, so the last quarter of the impulse goes in front of the first
   quarter — a 2048-tap kernel centred at 1024 — which is then Hann-windowed
   and zero-padded to 4096. Linear phase: every band is delayed alike, the
   "phase correct" of the manual.
6. **Forward FFT** → the kernel spectrum in *Conv Data*.

`hanning` computes its step from the double **6.2831853056** (2π mistyped:
…3056 for …307179586) and the window in double precision; the magnetic fader
mode uses **3.1415926528** — the same mistyped π as Riverrun's Cosine
envelope. Same programmer, same typo, in both directions.

## 4. Process: overlap-add

Per channel: the input goes into a ring; whenever 2048 samples are pending,
they are taken, zero-padded to 4096, transformed, multiplied by the kernel,
transformed back; the first 2048 results are **added** into the output ring
(the previous block's tail), the last 2048 **put** after them. vecLib's
forward transform is 2·DFT and the inverse is scaled by 0.5/N, so the round
trip is exact.

`InitBuffers` staggers the channels: channel *c* starts `(B·c) mod 2048`
samples into its block (B = the host buffer), so the blocks of different
channels complete in different host buffers and the CPU load spreads out.
Input and output rings are offset by the same amount, so all channels keep
the same delay.

**Latency.** `GetLatency` returns 3072 (0xc00: a block plus the kernel's
centre). That is the truth for host buffers up to 2048. The output ring is
started `max(2048, B)` behind, so with larger buffers the signal arrives
`B − 2048` samples later than reported (4096-sample buffers: 5120). The
port reports the real figure; the audio is unchanged.

The spectroscope (*DrawBuf*): the outputs summed into one more ring and
scaled by 1/√channels; the editor reads it.

## 5. Verification

`tools/emulate_periscope.py` runs the binary's own constructor,
`InitBuffers` and `Process` under Unicorn, with the binary's own parameter
objects. Three things had to be right:

* **The static initialiser.** Without `__GLOBAL__I_gThumbBreakPoints` the
  fader table is all zeros except the two literal breakpoints (0 dB, +20 dB)
  — and a flat EQ still renders perfectly, which hides the problem until a
  fader moves.
* **The host.** Pushed needles reach the parameters only through the host
  (vtable +0x3c of the settings' listener). The harness records them and
  writes them back after `Process`, as a VST host's `setParameter` would.
* **The FFT.** `fft_zrip` is forwarded through ctypes to the *real*
  Accelerate framework — the very routine the Rust port calls. vDSP picks
  its kernel by **buffer alignment**, and the kernels round differently:
  64-byte-aligned split buffers give one result, 16-byte-aligned another.
  Both sides therefore use 64-byte-aligned buffers (`mission::fft`).

```
28 scenarios — flat, mono, random / extreme / breakpoint fader sets, widest and
narrowest needles, the log-only region, needles crossed and pushed in both
directions, automation, host buffers 64…4096 including non-powers of two,
a buffer smaller than the declared one, 614 400-sample runs,
22.05 / 44.1 / 48 / 96 kHz:  28 identical, 0 different
```

What "identical" covers: the Rust engine against the 2.6.4 machine code, both
running today's vecLib. On a 2006 Mac the FFT itself (and so the last bits)
came from that era's vecLib; everything around it is the original's
arithmetic, operation for operation.

## 6. Mac OS 9 (v1.0)

`reference/macos9/Periscope™` (PEF) shares the fader table (the same
`pow(10, −4.8 … 1.7)` constants), the 4096-point FFT and 2048 blocks, the
30-band / 352 Hz layout, the ×0.95 / ×0.99 needle search, the 263-pixel
faders — and **both mistyped π constants** (6.2831853056 and 3.1415926528). It
differs in the needles: there is no 1/418, but a 1/390 (`0.0025641…`), i.e. 390
needle steps instead of 418, and its band arithmetic is in double precision
(`0.0333…` as a double, where 2.6.4 has the float `0.033333335`). Same design,
not the same numbers; running it is the open PowerPC/CFM item shared with the
other two plug-ins.

## 7. What the port changes

| | Original 2.6.4 | Port |
|---|---|---|
| Engine output | — | bit-identical (§5) |
| Reported latency | 3072 always | 3072 + max(0, buffer − 2048), the real delay |
| Pushed needles | sent to the host as automation | held by the engine, written to the parameters by the editor |
| MIDI | notes, CC 32–65, pitch bend | same; values hold until the parameter moves |
| Fader edit mode default | 1 (Flat) | Free draw (the mode the panel lists first, ⌘) |
| Display | QuickDraw, 4096-point FFT | egui; same FFT size, peak + average graphs, plus the kernel's real response as a thin line |
| Channels | mono … 5.1 | mono, stereo |
| Bypass | input copied, rings kept primed | host bypass |
