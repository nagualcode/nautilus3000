//! Offline PeriScope.
//!
//! ```text
//! periscope-render in.wav out.wav --faders 135,135,…  --low 40 --high 300
//! periscope-render --scenario scenario.json out.f32     # for the differential test
//! ```

use periscope::engine::{Controls, Periscope, BANDS};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--scenario") {
        return scenario(&args[1], &args[2]);
    }
    eprintln!("usage: periscope-render --scenario scenario.json out.f32");
    std::process::exit(2);
}

fn apply(c: &mut Controls, m: &serde_json::Map<String, serde_json::Value>) {
    for (k, v) in m {
        match k.as_str() {
            "low" => c.low = v.as_f64().unwrap() as f32,
            "high" => c.high = v.as_f64().unwrap() as f32,
            "faders" => {
                let a = v.as_array().expect("faders");
                assert_eq!(a.len(), BANDS);
                for (i, f) in a.iter().enumerate() {
                    c.faders[i] = f.as_i64().unwrap() as i32;
                }
            }
            _ => panic!("unknown control {k}"),
        }
    }
}

fn scenario(path: &str, out_path: &str) {
    let sc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("scenario")).expect("json");
    let sr = sc["sample_rate"].as_f64().unwrap();
    let n = sc["block"].as_u64().unwrap() as usize;
    let buffer = sc["buffer"].as_u64().map(|b| b as usize).unwrap_or(n);
    let blocks = sc["blocks"].as_u64().unwrap();
    let ch = sc["channels"].as_u64().unwrap_or(2) as usize;
    let mut c = Controls::default();
    if let Some(p) = sc["params"].as_object() {
        apply(&mut c, p);
    }
    let raw = std::fs::read(sc["input"].as_str().unwrap()).expect("input");
    let sig: Vec<f32> = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let left: Vec<f32> = sig.iter().step_by(2).copied().collect();
    let right: Vec<f32> = sig.iter().skip(1).step_by(2).copied().collect();
    let mut p = Periscope::new(sr, ch, buffer);
    let changes = sc["changes"].as_object().cloned().unwrap_or_default();
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path).expect("output"));
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let _ftz = mission::denormal::DenormalFlush::on();
    for b in 0..blocks {
        if let Some(m) = changes.get(&b.to_string()).and_then(|v| v.as_object()) {
            apply(&mut c, m);
        }
        let o = (b as usize * n) % (left.len() - n);
        l.copy_from_slice(&left[o..o + n]);
        r.copy_from_slice(&right[o..o + n]);
        if ch == 1 {
            p.process(&mut c, &mut [&mut l[..]]);
        } else {
            p.process(&mut c, &mut [&mut l[..], &mut r[..]]);
        }
        for k in 0..n {
            out.write_all(&l[k].to_le_bytes()).unwrap();
            if ch == 2 {
                out.write_all(&r[k].to_le_bytes()).unwrap();
            }
        }
    }
    println!("needles {:.6} {:.6} freqs {:?}", c.low, c.high, &p.filter.freqs[..3]);
}
