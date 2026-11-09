//! Offline Deep Phase Nine.
//!
//! ```text
//! dpn-render --scenario scenario.json out.f32     # for the differential test
//! ```

use deepphasenine::engine::{Controls, DeepPhaseNine, HostTime, Layout};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--scenario") {
        return scenario(&args[1], &args[2], args.iter().any(|a| a == "--trace"));
    }
    eprintln!("usage: dpn-render --scenario scenario.json out.f32");
    std::process::exit(2);
}

fn apply(c: &mut Controls, m: &serde_json::Map<String, serde_json::Value>) {
    for (k, v) in m {
        let f = v.as_f64().expect("number");
        match k.as_str() {
            "notches" => c.notches = f as i32,
            "feedback" => c.feedback = f as f32,
            "low" => c.low = f as f32,
            "high" => c.high = f as f32,
            "spread" => c.spread = f as f32,
            "wet_dry" => c.wet_dry = f as f32,
            "lfo_choice" => c.lfo_choice = f as i32,
            "lfo_speed" => c.lfo_speed = f as i32,
            "oneshot_end" => c.oneshot_end = f as f32,
            "oneshot_dir" => c.oneshot_dir = f as i32,
            "sync" => c.sync = f != 0.0,
            "stereo_offset" => c.stereo_offset = f as i32,
            _ => panic!("unknown control {k}"),
        }
    }
}

fn scenario(path: &str, out_path: &str, trace: bool) {
    let sc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("scenario")).expect("json");
    let sr = sc["sample_rate"].as_f64().unwrap();
    let n = sc["block"].as_u64().unwrap() as usize;
    let blocks = sc["blocks"].as_u64().unwrap();
    let layout = match sc["layout"].as_str().unwrap_or("stereo") {
        "mono" => Layout::Mono,
        "mono_to_stereo" => Layout::MonoToStereo,
        _ => Layout::Stereo,
    };
    let mut c = Controls::default();
    if let Some(p) = sc["params"].as_object() {
        apply(&mut c, p);
    }
    let raw = std::fs::read(sc["input"].as_str().unwrap()).expect("input");
    let sig: Vec<f32> = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let left: Vec<f32> = sig.iter().step_by(2).copied().collect();
    let right: Vec<f32> = sig.iter().skip(1).step_by(2).copied().collect();
    let mut d = DeepPhaseNine::new(sr, layout);
    let changes = sc["changes"].as_object().cloned().unwrap_or_default();
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path).expect("output"));
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    deepphasenine::engine::flush_denormals(true);
    for b in 0..blocks {
        if let Some(m) = changes.get(&b.to_string()).and_then(|v| v.as_object()) {
            apply(&mut c, m);
        }
        let host = match sc["transport"].as_object() {
            Some(t) => {
                let bpm = t["bpm"].as_f64().unwrap();
                let pos = b as f64 * n as f64;
                let ppq = t.get("start_ppq").and_then(|v| v.as_f64()).unwrap_or(0.0) + pos / (sr * 60.0 / bpm);
                HostTime { playing: t["playing"].as_bool().unwrap(), bpm, ppq, transport_changed: b == 0 }
            }
            None => HostTime::default(),
        };
        let o = (b as usize * n) % (left.len() - n);
        l.copy_from_slice(&left[o..o + n]);
        match layout {
            Layout::Stereo => r.copy_from_slice(&right[o..o + n]),
            _ => r.copy_from_slice(&left[o..o + n]),
        }
        d.process(&c, &host, &mut l, &mut r);
        if trace {
            println!("{b:4} {}", d.trace());
        }
        for k in 0..n {
            out.write_all(&l[k].to_le_bytes()).unwrap();
            if layout != Layout::Mono {
                out.write_all(&r[k].to_le_bytes()).unwrap();
            }
        }
    }
}
