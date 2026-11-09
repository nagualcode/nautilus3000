//! Offline Riverrun.
//!
//! ```text
//! riverrun-render input.wav output.wav [options]
//! riverrun-render --scenario scenario.json out.f32     # for the differential test
//! ```
//!
//! The input's first 4.65 s go into the input screen, as if loaded with the
//! Load button; the engine runs at the input's sample rate.

use riverrun::audio;
use riverrun::engine::tables::{ENVELOPE_NAMES, GRID_NAMES};
use riverrun::engine::{BeatSync, Controls, Engine, HostTime};
use std::io::Write;
use std::path::Path;

const HELP: &str = "riverrun-render INPUT OUTPUT [options]      (INPUT = voice: Audio Ease's demo voice; sung: synthetic)

  --seconds S        length to render (default 30)
  --block N          grain clock / buffer size (default 512)
  --pitch P          0…1000     (500 = original pitch)
  --random R         0…100      pitch random
  --grid G           1…23       pitch grid (1 = seamless; see --list)
  --fine F           0.666…1.5  fine tune ratio
  --speed S          1…286      grain speed
  --speed-random S   0…286
  --length L         1…286      grain length
  --envelope E       1…4        Triangle, Square, Sharp attack, Cosine
  --walk W           -5…5
  --glass X          0…472      Grain Glass start
  --needles L,R      0…472      red and yellow needles
  --mono-stereo M    0…93
  --rhythmic         Rhythmic mode (with --bpm, default 120)
  --tight            Rhythmic grains on the beat grid (not the original timing)
  --format F         16 | 24 | 32f (default 24)
  --preset NAME      a preset from the Riverrun Demo song (options after it override)
  --list             print the envelope, pitch-grid and preset names
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--scenario") {
        return scenario(&args[1], &args[2]);
    }
    if args.iter().any(|a| a == "--list") {
        for (i, n) in ENVELOPE_NAMES.iter().enumerate() {
            println!("envelope {}: {n}", i + 1);
        }
        for (i, n) in GRID_NAMES.iter().enumerate() {
            println!("grid {:2}: {n}", i + 1);
        }
        for p in riverrun::presets::PRESETS.iter() {
            println!("preset \"{}\": {}", p.name, p.about);
        }
        return;
    }
    if args.len() < 2 || args.iter().any(|a| a == "-h" || a == "--help") {
        eprint!("{HELP}");
        std::process::exit(if args.len() < 2 { 2 } else { 0 });
    }
    if let Err(e) = render(&args) {
        eprintln!("riverrun-render: {e}");
        std::process::exit(1);
    }
}

fn render(args: &[String]) -> Result<(), String> {
    let input = Path::new(&args[0]);
    let output = Path::new(&args[1]);
    let mut c = Controls::default();
    let (mut seconds, mut block, mut bpm, mut tight) = (30.0f64, 512usize, 120.0f64, false);
    let mut glass = 150.0f32;
    let mut fmt = audio::SampleFormat::Int24;
    let mut i = 2;
    let num = |i: usize| -> Result<f64, String> {
        args.get(i).and_then(|v| v.parse().ok()).ok_or(format!("{} needs a number", args[i - 1]))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--seconds" => seconds = num(i + 1)?,
            "--block" => block = num(i + 1)? as usize,
            "--pitch" => c.pitch = num(i + 1)? as f32,
            "--random" => c.random = num(i + 1)? as f32,
            "--grid" => c.grid = num(i + 1)? as i32,
            "--fine" => c.fine = num(i + 1)? as f32,
            "--speed" => c.speed = num(i + 1)? as f32,
            "--speed-random" => c.speed_random = num(i + 1)? as f32,
            "--length" => c.length = num(i + 1)? as f32,
            "--envelope" => c.envelope = num(i + 1)? as i32,
            "--walk" => c.walk = num(i + 1)? as f32,
            "--glass" => glass = num(i + 1)? as f32,
            "--mono-stereo" => c.mono_stereo = num(i + 1)? as f32,
            "--bpm" => bpm = num(i + 1)?,
            "--preset" => {
                let name = args.get(i + 1).ok_or("--preset NAME")?;
                let p = riverrun::presets::find(name).ok_or(format!("no preset {name:?} (see --list)"))?;
                c.pitch = p.pitch;
                c.speed = p.speed;
                c.length = p.length;
                c.random = p.random;
                c.speed_random = p.speed_random;
                c.walk = p.walk;
                c.envelope = p.envelope as i32;
                c.grid = p.grid as i32;
                c.rhythmic = p.rhythmic;
                c.mono_stereo = p.mono_stereo;
                c.needle_l = p.needle_l;
                c.needle_r = p.needle_r;
                c.fine = p.fine;
                glass = p.glass;
            }
            "--needles" => {
                let v = args.get(i + 1).ok_or("--needles L,R")?;
                let (l, r) = v.split_once(',').ok_or("--needles L,R")?;
                c.needle_l = l.parse().map_err(|_| "--needles L,R")?;
                c.needle_r = r.parse().map_err(|_| "--needles L,R")?;
            }
            "--format" => {
                fmt = match args.get(i + 1).map(String::as_str) {
                    Some("16") => audio::SampleFormat::Int16,
                    Some("24") => audio::SampleFormat::Int24,
                    Some("32f") => audio::SampleFormat::Float32,
                    _ => return Err("--format 16|24|32f".into()),
                }
            }
            "--rhythmic" => {
                c.rhythmic = true;
                i += 1;
                continue;
            }
            "--tight" => {
                tight = true;
                i += 1;
                continue;
            }
            o => return Err(format!("unknown option {o}")),
        }
        i += 2;
    }
    let src = match args[0].as_str() {
        "demo" | "sung" => {
            let d = riverrun::demo::snippet(48000.0);
            audio::Decoded { name: d.name, sample_rate: d.sample_rate, samples: d.samples }
        }
        "voice-streamed" => {
            let d = riverrun::demo::voice_streamed();
            audio::Decoded { name: d.name, sample_rate: d.sample_rate, samples: d.samples }
        }
        "voice" => {
            let d = riverrun::demo::voice();
            audio::Decoded { name: d.name, sample_rate: d.sample_rate, samples: d.samples }
        }
        _ => audio::load(input)?,
    };
    let sr = src.sample_rate;
    let mut e = Engine::new(sr);
    e.beat_sync = if tight { BeatSync::Tight } else { BeatSync::Original };
    e.load_snippet(&src.samples);
    e.set_glass(glass);
    let total = (seconds * sr) as usize;
    let spq = sr * 60.0 / bpm;
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let zero = vec![0.0f32; block];
    let mut out = Vec::with_capacity(total * 2 + block * 2);
    // Skip the 300-sample look-ahead so the file starts in time.
    let mut skip = riverrun::LATENCY as usize;
    let mut pos = 0usize;
    while out.len() < total * 2 {
        let host = HostTime {
            playing: true,
            bpm,
            ppq: pos as f64 / spq,
            sample_pos: pos as i64,
            transport_changed: pos == 0,
        };
        e.process_block(&c, &host, [&zero, &zero], false, &mut l, &mut r);
        for k in 0..block {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            out.push(l[k]);
            out.push(r[k]);
        }
        pos += block;
    }
    out.truncate(total * 2);
    audio::save(output, sr, 2, &out, fmt)?;
    let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    println!(
        "{} → {}: {:.1} s at {} Hz, {} grains, peak {:.1} dBFS",
        src.name,
        output.display(),
        seconds,
        sr,
        e.grain_count,
        20.0 * peak.max(1e-9).log10()
    );
    Ok(())
}

/// Runs a scenario of `reverse_engineering/riverrun/tools/differential_test.py`
/// and writes interleaved float32.
fn scenario(path: &str, out_path: &str) {
    let sc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("scenario")).expect("json");
    let sr = sc["sample_rate"].as_f64().unwrap();
    let n = sc["block"].as_u64().unwrap() as usize;
    let blocks = sc["blocks"].as_u64().unwrap();
    let mut e = Engine::new(sr);
    let mut c = Controls::default();
    let mut glass = 150.0f32;
    let apply = |c: &mut Controls, glass: &mut f32, m: &serde_json::Map<String, serde_json::Value>| {
        for (k, v) in m {
            let f = v.as_f64().unwrap() as f32;
            match k.as_str() {
                "pitch" => c.pitch = f,
                "speed" => c.speed = f,
                "record" => c.record = f != 0.0,
                "length" => c.length = f,
                "random" => c.random = f,
                "speed_random" => c.speed_random = f,
                "walk" => c.walk = f,
                "envelope" => c.envelope = f as i32,
                "grid" => c.grid = f as i32,
                "rhythmic" => c.rhythmic = f != 0.0,
                "mono_stereo" => c.mono_stereo = f,
                "glass" => *glass = f,
                "needle_l" => c.needle_l = f,
                "needle_r" => c.needle_r = f,
                "fine" => c.fine = f,
                _ => panic!("unknown control {k}"),
            }
        }
    };
    if let Some(p) = sc["params"].as_object() {
        apply(&mut c, &mut glass, p);
    }
    e.set_glass(glass);
    let mut snip: Vec<f32> = vec![];
    if let Some(p) = sc["snippet"].as_str() {
        let raw = std::fs::read(p).expect("snippet");
        snip = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        e.load_snippet(&snip);
    }
    let live = sc["live_input"].as_bool().unwrap_or(false);
    let changes = sc["changes"].as_object().cloned().unwrap_or_default();
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let zero = vec![0.0f32; n];
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path).expect("output"));
    for b in 0..blocks {
        if let Some(m) = changes.get(&b.to_string()).and_then(|v| v.as_object()) {
            apply(&mut c, &mut glass, m);
        }
        let inp: &[f32] = if live {
            let off = (b as usize * n) % (snip.len() - n);
            &snip[off..off + n]
        } else {
            &zero
        };
        let host = match sc["transport"].as_object() {
            Some(t) => {
                let bpm = t["bpm"].as_f64().unwrap();
                let pos = b as i64 * n as i64;
                let ppq = t.get("start_ppq").and_then(|v| v.as_f64()).unwrap_or(0.0) + pos as f64 / (sr * 60.0 / bpm);
                HostTime { playing: t["playing"].as_bool().unwrap(), bpm, ppq, sample_pos: pos, transport_changed: b == 0 }
            }
            None => HostTime::default(),
        };
        e.process_block(&c, &host, [inp, &zero], false, &mut l, &mut r);
        for k in 0..n {
            out.write_all(&l[k].to_le_bytes()).unwrap();
            out.write_all(&r[k].to_le_bytes()).unwrap();
        }
    }
}
