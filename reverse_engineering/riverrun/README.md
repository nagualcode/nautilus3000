# Reverse engineering Riverrun

How the granular engine of **Riverrun** (Audio Ease, Nautilus Bundle, 2000–2013;
interface by Captain Video; "thank you Mr. Barry Truax") was recovered from the
original machine code, and how the Rust engine in `plugins/riverrun/src/engine/`
was proven **bit-identical** to it.

Every behaviour described here can be traced to an address in the i386 slice
of `Riverrun.vst` 2.6.4 (`original_binaries/Riverrun-2.6.4-VST-i386`).

| Path | What it is |
|---|---|
| `original_binaries/Riverrun-2.6.4-VST-i386` | i386 slice of the VST from `Nautilus2.6.4.mpkg` (sha256 `d899656e…be0d`) — the reference |
| `original_binaries/Riverrun-2.6.4-VST-ppc` | PowerPC slice of the same universal binary (sha256 `7d198fa9…0f20`) |
| `original_binaries/Riverrun.rsrc` | Resource file: menus, help texts, the planet layout (`MC…` strings) |
| `decompiled/Riverrun-2.6.4-i386.c` | Ghidra 12.2 decompilation of every function (44 838 lines) |
| `decompiled/symbols.txt` | `nm -n` symbol table, demangled |
| `decompiled/import_stubs.txt` | `__IMPORT,__jump_table` stub → imported function |
| `tools/DecompileAll.java` | Ghidra headless post-script that wrote the `.c` file |
| `tools/macho.py` | Minimal Mach-O reader: read constants and tables at virtual addresses |
| `tools/extract_function.py` | Print functions from the decompilation by address or name |
| `tools/binhex.py` | BinHex 4.0 decoder (for `Nautilus_Bundle.hqx`) |
| `tools/pef_unpack_data.py` | Expands the pattern-initialised data section of a PEF (OS 9 build) |
| `original_binaries/Riverrun-1.0-MAS-OS9.pef` | The Mac OS 9 MAS plug-in, Nautilus Bundle 1.0 (§15) |
| `tools/pef_unpack_data.py` | Expands the pattern-initialised data section of a PEF (OS 9 build) |
| `original_binaries/Riverrun-1.0-MAS-OS9.pef` | The Mac OS 9 MAS plug-in, Nautilus Bundle 1.0 (§15) |
| `tools/emulate_riverrun.py` | **Runs the original Riverrun DSP under Unicorn** (see §11) |
| `tools/differential_test.py` | 57 scenarios: original (emulated) vs Rust, compared bit for bit |
| `ui/` | The original panel, about box and assets, for reference |

---

## 1. Sources

All material came from archived installers and forum finds (`reference/`): `Nautilus2.6.4.mpkg` and `Nautilus Demo-2.6.4.mpkg` (Mac OS X, January
2013, the last release), the Mac OS 9 `Nautilus_Bundle.hqx`, manual 2.1.0, the
*Sound on Sound* review (June 2002) and the Audio Ease web pages from the
Wayback Machine.

The OS X packages are old-style bundles whose payload is `Archive.pax.gz`. Every
format (AU, VST, RTAS, MAS) ships the same engine in a universal (ppc + i386)
Mach-O; the i386 VST slice was chosen because it is the smallest and x86 SSE
code decompiles far more faithfully than PowerPC. The demo and the full VST
binaries differ only in the authorisation code.

**Mac OS 9 version.** `Nautilus_Bundle.hqx` decodes (`tools/binhex.py`) to a
StuffIt 5 archive holding *Nautilus Bundle™ Installer*, an **Installer VISE**
(MindVision, `SVCT`) package that no public tool unpacks. The installed OS 9
files were later recovered separately (`reference/macos9/`); see §15 and §16.

## 2. The binary

* `__TEXT` at 0x0, `__DATA` at 0x3e000, `__IMPORT` at 0x45000. Bundles load
  at their link address, so file addresses are runtime addresses.
* **Class names are obfuscated** (a 31-letter random name per class — copy
  protection), but **method names survived**, which is what made the map
  possible:

| Obfuscated class | Role | Key methods |
|---|---|---|
| `gyaywgjoxfgohfmpwutfuydpbiggdyv` | the Riverrun process | `Process` @ 0x204c6, `InitBuffers` @ 0x2013c, `GetPitch`, `GetGrainLength`, … `ReceiveMidi` @ 0x1fbf2 |
| `ptoqcayexipwcepyhzpwknuemzcseph` | the granular engine | `Init` @ 0x1c020, `InitEnvelopes` @ 0x1a616, `CreateScales` @ 0x1b55e, `FindClosestPitch` |
| `pdrabnexyxetetrmuacqttvvpynpown` | the grain generator | `Run(unsigned long)` @ 0x1adda |
| `hgiovhogovemzqirnqmvniotxfltptf` | snippet / output bus (ring + overview) | `AddGrain` @ 0x1c4aa, `LoadSnippet`, `SaveSnippet`, `PutInOverView` |
| `ekmdvrkalxdgqoeajnnmnqbvjhvlvgw` | circular buffer | `Put`, `Get`, `GetPitched` @ 0x3166, `GetAndClear` @ 0x34ee |
| `yruwgvpjbwnzjpmoiefxsqrddxiyfug` | process base: AGC + demo beep | `AGC` @ 0x21db0, `InitAGC`, `Blackout`, `InitBlackout` |
| `adodghoqgyduuxhvcqjtqxnskmkxbjn` | host tempo context | `SetTempo` @ 0xdf9a, `GetQuarterPosFraction`, … |
| `tgxemushnzhoebnxxkyhbjbmeyoybjy` | plug-in description | `GetParameter` @ 0x1d618 (ids, ranges, defaults) |
| `vltuteeygzsxaejmwlboltsiambnbus` | parameter store | `GetParamFloat` @ 0xe94c, `GetParamLong`, `SetParamLong` |

The UI classes kept their names (`RiverrunWaveform`, `RiverrunSliderGrainSpeed`,
`RiverrunPitchGrid`, `RiverrunEnvelopChoice`, …).

## 3. Method, and a warning about decompilers

1. `analyzeHeadless` + `tools/DecompileAll.java` → `decompiled/…c`.
2. Constants and static tables were read from the file at their virtual
   address (`tools/macho.py`). Position-independent code addresses them as
   `ebx + k` after a `call/pop` thunk, so each needs the right `ebx`.
3. **Every hot path was re-read in the disassembly** (`otool -tvV`), because
   Ghidra misrepresents some conversions. Example: the per-grain envelope index
   in `Run` reads as a single-precision product in the C, but the code is
   `cvtdq2pd; mulsd; cvttsd2si` — double precision. Getting such details right
   is the difference between "sounds the same" and "is the same".
4. Finally, the original was **executed** (§11) and compared to the port.

## 4. Parameters

From `GetParameter` @ 0x1d618 and `ReceiveMidi` @ 0x1fbf2:

| Id | Name | Range | Default | MIDI CC |
|---|---|---|---|---|
| 0 | Pitch | 0 … 1000 | 500 | 15 |
| 1 | Grain speed | 1 … 286 | 125 | 14 |
| 2 | Record | off / on | off | 19 |
| 3 | Grain length | 1 … 286 | 100 | 18 |
| 4 | Pitch Random | 0 … 100 | 0 | 16 |
| 5 | Grain speed random | 0 … 286 | 50 | 17 |
| 6 | Walk | −5 … 5 | 0.5 | 13 |
| 7 | Envelope | 1 … 4 | 2 (Square) | program change |
| 8 | Pitch grid | 1 … 23 | 1 (seamless) | |
| 9 | Rhythmic or Flowing | | Flowing | |
| 10 | Mono-stereo | 0 … 93 | 93 | 1 |
| 11 | Grain Glass Position | 0 … 472 | 150 | note on |
| 12 | GrainWindowRight | (display only) | | |
| 13 | Left (red) Needle | 0 … 472 | 20 | 11 |
| 14 | Right (yellow) Needle | 0 … 472 | 270 | 12 |
| 15 | Fine-tune | 0.666666 … 1.5 | 1.0 | |
| 16/17 | snippet file / snippet data | | | |

Most ranges are **slider pixels** (472 = the input screen, 285 = the speed and
length sliders). Every getter truncates the value to `short` before using it, so
the controls have exactly one step per original pixel. The port keeps these
units.

* CC: `value = min + (cc · (max − min)) / 128` — with `/128`, CC 127 never
  reaches the maximum.
* Note on: Grain Glass = `((note − 36) mod 60) / 60` of the screen.
* Program change: envelope `(program & 3) + 1`.
* The needles push each other (`SetFloatValue` of params 13/14).

## 5. Control curves

`raprsrnunraeegybghbdzrvqypyotzm(short x, float* table, short n)` @ 0x4786 is a
piecewise-linear lookup over `(x, y)` pairs. Each getter has its own table
(`engine/tables.rs`):

| Control | Breakpoints (x → y) | Unit of y |
|---|---|---|
| Pitch | 0→1/8, 200→1/4, 350→1/2, 500→1, 750→2, 1000→8 | playback ratio |
| Pitch Random | 0→0, 20→0.1, 50→1, 90→4, 100→16 | ratio band, **added** to the ratio |
| Grain length | 0→50, 50→700, 125→3000, 200→7000, 285→15000 | samples at 44.1 kHz (×sr/44100) |
| Grain speed (Flowing) | 0→1, 50→10, 125→200, 200→800, 285→2000 | grains per second |
| Grain speed random | 0→1, 50→100, 150→2000, 285→30000 | onset jitter, samples at 44.1 kHz |

In Rhythmic mode the grain speed picks one of 14 divisions,
`index = round(speed / 21.142857)` (`Process::C_180`): 1/1, dotted 1/2, 1/2,
1/2T, 1/4, 1/4T, 1/8, 1/8T, 1/16, 1/16T, 1/32, 1/32T, 1/64, 1/128.

## 6. Buffers

`InitBuffers` sizes everything from the sample rate:

* snippet (the input screen) = `sr × 4.65` samples — "about four seconds";
* longest grain = envelope table = `sr` samples;
* two output buses (L, R) of a fixed **100 000** samples (quad: 4, 5.0: 5).

## 7. One block: `Process` @ 0x204c6 → `Run` @ 0x1adda

`Process` turns parameters into engine fields, moves the Grain Glass, calls
`Run`, reads the buses, applies the AGC and the mono/stereo mix.

**The Grain Glass** lives in sample units counted from the oldest sample of the
snippet ring. Each block:
`pos = (snap/472)·glass + (int)(n · walk)`; below the red needle it jumps to the
yellow one and beyond the yellow one to the red one; if a grain would run past
the end of the screen it is moved back (or, walking forwards, to the red needle)
and, if necessary, shortened. Walk is therefore in "× real time": 0.01 is the
manual's microscope at a hundredth of normal speed.

**`Run` makes at most one grain per block** — this is the heart of Riverrun's
texture. If the next onset `t` falls inside the block:

1. LCG step `x = x·1664525 + 1013904223` (Numerical Recipes);
2. read position = put position + glass position + `x mod 1–2` (sub-sample jitter);
3. ratio = `pitch + (x / 2³²) · random`, then snapped up to the pitch grid,
   times fine tune; length = `grain length / ratio`, at most one second;
4. `GetPitched`: linear interpolation with the read position accumulated in
   **single precision** — far into the 4.65 s ring the float step loses bits and
   the pitch wobbles slightly; kept;
5. the envelope is resampled from the one-second master table and applied.

Then *every* onset in the block gets a **copy of that same grain**: two more LCG
steps choose the bus (the sign bit, for stereo) and an onset jitter
(`x mod jitter`), and `t += 1 / grain speed`. With 2000 grains/s and a 512
buffer, ~23 identical grains share a block, which is why the original sounds
different at different host buffer sizes (see §10, "grain clock").

**Mono/stereo**: every copy lands hard left or hard right; the output is
`L' = L + k·R`, `R' = R + k·L` with `k = 1 − ms/93` (mono = both buses on both
sides). Quad and 5.0 spread the copies over four or five buses.

## 8. Pitch grids — `CreateScales` @ 0x1b55e

Twenty-three grids, stored as `(double)(float)pow(1.0594630943592953, note−60)`:
seamless; minor 7th, major 7th, dominant 7th (`C_138…C_140`); min/maj 5 and
min/maj 4 chords "deluxe"; min/maj 9th (with a doubled 62); octaves and fifths;
diatonic major; chromatic −36…+48; and equal steps of 2…12 semitones from −36.
Full note lists in `engine/tables.rs`.

Snapping (`FindClosestPitch` @ 0x1ac76, inlined in `Run`) takes the first
entry ≥ the random ratio, searching up to index `count` — **one past the last
written entry** for most chord grids. When pitch + random exceed the top of the
grid, the original reads an unwritten slot of fresh heap memory (0.0) and plays
the grain at ratio 0: one input sample held for one second under the envelope,
a soft DC "thump". Reproduced.

## 9. Envelopes — `InitEnvelopes` @ 0x1a616

| Menu | Name | Shape code | Attack / decay (fraction of grain) |
|---|---|---|---|
| 1 | Triangle | 0, linear | 0.5 / 0.5 |
| 2 | Square | 1, linear | 0.05 / 0.05 (a trapezoid) |
| 3 | Sharp attack | 3 | 0.05 / 0.3 |
| 4 | Cosine | 2, raised cosine | 0.5 / 0.5 |

Quirks kept bit for bit:

* the cosine uses π = **3.1415926528** (a typo for …26535…);
* Sharp attack rises over 40 % of the attack, falls by `0x3fe9999998000000`
  (0.79999999702 — neither the double nor the float nearest to 0.8) to ≈0.2,
  holds 0.2 and falls by 0.2 over the decay — continuing from wherever the float
  ramp actually ended;
* for shapes 0–2 the decay loop starts one sample past the grain and stops one
  sample early.

## 10. AGC — `AGC` @ 0x21db0

`InitAGC(1.0, 0.3, 10^(−12/(10·sr)), 10^(3/(10·sr)), 0.15, 300)`: a 300-sample
look-ahead (the plug-in latency, `GetLatency` @ 0x20022). Per sample: if
`gain · peak > 0.15` the gain falls (−24 dB/s); if `gain < 0.3` it rises
(+6 dB/s). So after the first loud moment it hovers around 0.3 and never goes
back to unity; leaving Rhythmic mode multiplies it by 0.2. This is what keeps
"1000 layers" from exploding.

`Blackout`/`InitBlackout` is the demo beep of an unauthorised copy (a ~1 kHz
blip every 12 s); it is not part of the instrument and is not reproduced.

## 11. Rhythmic mode and the host's time

`VSTMissionHost::GetTimeInfo` @ 0x26998 passes the host tempo, position, play
state and `ppq − (int)ppq` (the part of the quarter already played).
`Process` raises a resync on: switching to Rhythmic, a tempo change, a transport
change, a grain-speed change, or a position jump — including "no movement",
i.e. every block while the transport is stopped. `Run` honours it only in a
block that starts less than one block after a quarter note (or when stopped),
placing the first grain at `block start + elapsed part of the beat`: up to two
buffers late. The port reproduces this (*Beat sync: Original*) and offers
*Tight*, which puts grains on the host's grid.

## 12. Verification: running the original

`tools/emulate_riverrun.py` maps the i386 binary into
[Unicorn](https://www.unicorn-engine.org/) and runs the **original constructors,
`InitBuffers` and `Process`** — every instruction of the DSP path is Audio
Ease's. Only the environment is replaced:

* the 428 import stubs: a bump allocator for `operator new`/`NewPtrClear`/
  `calloc`, libm for `cos`/`pow`/…, no-ops for logging; functions that return a
  float on i386 do so on the x87 stack, so those hooks jump to a
  `fld [scratch]; ret` trampoline;
* the parameter store (`GetParamFloat`/`GetParamLong`/`SetParamLong`/
  `GetParameter`), answered from a Python dict;
* the host tempo context, filled like `GetTimeInfo` does;
* `IsBypassed` → 0, `Blackout`/`InitBlackout` skipped.

`tools/differential_test.py` then renders the same blocks with the Rust engine
(`riverrun-render --scenario`) and compares the float32 output **bit for bit**:

```
$ python differential_test.py original_binaries/Riverrun-2.6.4-VST-i386 \
      ../../target/release/riverrun-render /tmp/rr
defaults                     153600 frames  IDENTICAL
envelope-1 … envelope-4                     IDENTICAL
grid-1 … grid-23 (pitch 700, random 95)     IDENTICAL
pitch-low, pitch-high, fine-tune            IDENTICAL
walk-back, walk-stop, walk-fast-narrow      IDENTICAL
long-grains-at-end, long-grains-backwards   IDENTICAL
dense, sparse, jitter-max, mono, half-stereo IDENTICAL
automation (8 parameter changes mid-run)     IDENTICAL
rhythmic-div-0/4/7/13, 93 bpm + jitter,
  stopped transport, rhythmic→flowing,
  speed changes                              IDENTICAL
record-live, record-then-freeze              IDENTICAL
host blocks of 64 and 1024                   IDENTICAL
long-run-bus-wrap          1280000 frames    IDENTICAL
sr-48000, sr-96000                           IDENTICAL

57 identical, 0 different
```

`emulate_riverrun.py --trace` prints the engine's internal state per block in
the same format as `Engine::trace()`, which is how the one bug found on the way
(in the harness itself) was located.

Requirements: `pip install unicorn capstone`, and macOS `otool` (stub table).

## 13. What the port changes on purpose

| Original | Port | Why |
|---|---|---|
| One grain per **host** buffer: the sound depends on the host's buffer size | A fixed **grain clock** (default 512, selectable 64–1024); identical output for any host buffer | reproducible projects; 512 is a common host setting |
| Rhythmic grains up to two buffers off the beat | *Beat sync: Original* (default) or *Tight* | musical option |
| 100 000-sample output buses; above 48 kHz long grains plus jitter could overrun them (fixed in 2.5.9) | buses grow with the sample rate; identical output whenever the original did not overrun | robustness |
| Grain speed random 0 below 44.1 kHz → division by zero | jitter ≥ 1 sample | robustness |
| Loaded files used as is | resampled to the session rate | modern convenience |
| Snippet saved as an SD II file in `Riverrun Scratch ƒ` | embedded in the project | portability |
| SD II / Wave / AIFF in, AIFF out | WAV / AIFF / AIFC in, WAV / AIFF out | SD II is gone |
| Mono, stereo, quad, 5.0 outputs | stereo (and mono) | modern hosts; the quad/5.0 bus choice is ported (`choose_bus`) but the extra buses are not wired |
| No dry/wet | Mix, Output (defaults: 100 %, 0 dB = original) | modern options |
| Demo beep | — | not part of the instrument |

## 14. Reproducing

```bash
# decompile
analyzeHeadless /tmp/proj rr -import original_binaries/Riverrun-2.6.4-VST-i386 \
    -scriptPath tools -postScript DecompileAll.java out.c
python3 tools/extract_function.py out.c 0001adda          # Run
otool -tvV original_binaries/Riverrun-2.6.4-VST-i386 | less

# prove the port
cargo build --release -p riverrun --bin riverrun-render
python3 -m venv .venv && .venv/bin/pip install unicorn capstone
.venv/bin/python tools/differential_test.py original_binaries/Riverrun-2.6.4-VST-i386 \
    ../../target/release/riverrun-render /tmp/rr
```

## 15. The Mac OS 9 version (Nautilus Bundle 1.0, MAS)

`original_binaries/Riverrun-1.0-MAS-OS9.pef` (sha256 `841b394c…8213`, from the OS 9 disc
image the user recovered) is a PowerPC PEF for MOTU's MAS 2.1 / Digital
Performer 2.7. Unlike 2.6.4 it is **not obfuscated**: method names such as
`GrainEngine`, `RiverRunEffectProc`, `GetPitch`, `GetGrainLength`,
`RiverRunCyclicBuffer` sit in its (pattern-initialised) data section —
`tools/pef_unpack_data.py` expands it. Ghidra 12 loads it with
`-processor PowerPC:BE:32:default` (it guesses VLE otherwise).

Searching the unpacked data for every table and constant of 2.6.4:

| | v1.0 (OS 9) | 2.6.4 (OS X) |
|---|---|---|
| Pitch, random, length, jitter and flowing-speed curves | identical | |
| Rhythmic divisions, 21.142857, 480 | identical | |
| semitone `1.0594630943592953`, π typo `3.1415926528` | identical (the typo dates from 2000) | |
| AGC constants (0.3, 0.15, −12, 3, 300, 10) | present | |
| **minor 7th chord grid** | top six notes a semitone high: …84, 87, **92, 95, 97, 100, 104, 107** | …84, 87, 91, 94, 96, 99, 103, 106 (fixed) |
| other chord / scale grids | identical | |
| input screen | **204 800 samples**, fixed (433.9 per pixel) | `sr × 4.65` |
| longest grain / envelope table | 20 000 / 5 000, fixed | `sr` / `sr` |
| Sharp-attack "0.8" | ordinary float `0x3f4ccccd` | `0x3fe9999998000000` (≈1e-9 apart) |
| stereo-input mix `×0.707`, mono/stereo `/−93` | absent (the expressions differ) | present |

So the 2000 engine is the same design with a few tables changed; the most
audible difference is the out-of-tune top of the minor-7th grid. The code
itself has not been executed yet: a PowerPC/CFM version of the Unicorn harness
(§12) is the next step to make v1.0 a verified variant.

## 16. The Riverrun Demo song

The OS 9 disc carried Audio Ease's own demo: a Digital Performer project
(`reference/macos9/Riverrun Demo Song`) and one audio file, `Audio Files/Voice`.

* **Voice** is the data fork of a Sound Designer II file — 16-bit big-endian
  mono, header lost with the resource fork. 205 800 samples and a spectrum that
  rolls off ~55 dB toward 22 kHz place it at 44.1 kHz: one full input screen.
  It ships in the port (`plugins/riverrun/assets/voice.sd2`) as the demo sound.
* The project stores Riverrun settings as MAS parameter records
  `[0][id u32][len u32][value]` (floats, with Record, Envelope, Pitch grid,
  Rhythmic, Grain Glass and the needles as integers). Four were found: the voice
  track's own state and three named presets — **"probeer de klant"** (Dutch:
  "try the client"), **"probeer de klant 2"** and **"drumloop 3"** — now in
  `src/presets.rs`, in the editor's *demo & presets* menu and in
  `riverrun-render --preset`.
* The frozen screens they used ("RiverRun Temp Snap 51876 / 61716") were not
  on the disc. The track had Record on — the voice streamed in and the Grain
  Glass sat at the newest end — so presets load the Voice right-aligned on its
  last syllable (`demo::voice_streamed`). That alignment is an inference.
* The song's automation of 13 of the 14 parameters (*A Professional Guide to
  Audio Plug-ins*, p. 340) is in the project's event lists and has not been
  decoded yet.
