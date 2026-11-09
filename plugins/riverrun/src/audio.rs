//! Audio files: WAV and AIFF/AIFC in, WAV and AIFF out.
//!
//! Riverrun 2.5.2 loaded SD II, WAV and AIFF and saved AIFF. SD II files carry
//! their audio in the resource fork and are not supported here.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A decoded file, mixed to mono.
pub struct Decoded {
    pub name: String,
    pub sample_rate: f64,
    pub samples: Vec<f32>,
}

pub fn load(path: &Path) -> Result<Decoded, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let (sample_rate, channels, interleaved) = if matches!(ext.as_str(), "aif" | "aiff" | "aifc") {
        read_aiff(path)?
    } else {
        read_wav(path).or_else(|_| read_aiff(path))?
    };
    if !(sample_rate > 0.0 && sample_rate.is_finite()) {
        return Err("invalid sample rate".into());
    }
    let ch = channels.max(1);
    let samples: Vec<f32> = if ch == 1 {
        interleaved
    } else {
        interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    };
    if samples.is_empty() {
        return Err("the file contains no audio".into());
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
    Ok(Decoded { name, sample_rate, samples })
}

fn read_wav(path: &Path) -> Result<(f64, usize, Vec<f32>), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("WAV: {e}"))?;
    let spec = r.spec();
    let data = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 / scale).collect()
        }
    };
    Ok((spec.sample_rate as f64, spec.channels as usize, data))
}

fn read_aiff(path: &Path) -> Result<(f64, usize, Vec<f32>), String> {
    let mut r = BufReader::new(File::open(path).map_err(|e| format!("AIFF: {e}"))?);
    let mut id = [0u8; 4];
    let mut sz = [0u8; 4];
    r.read_exact(&mut id).map_err(|e| e.to_string())?;
    r.read_exact(&mut sz).map_err(|e| e.to_string())?;
    let mut form = [0u8; 4];
    r.read_exact(&mut form).map_err(|e| e.to_string())?;
    if &id != b"FORM" || !(&form == b"AIFF" || &form == b"AIFC") {
        return Err("not an AIFF file".into());
    }
    let (mut channels, mut bits, mut sr) = (1usize, 16usize, 44100.0);
    let mut float = false;
    let mut pcm: Option<Vec<u8>> = None;
    while r.read_exact(&mut id).is_ok() {
        if r.read_exact(&mut sz).is_err() {
            break;
        }
        let size = u32::from_be_bytes(sz) as usize;
        match &id {
            b"COMM" => {
                let mut c = vec![0u8; size];
                r.read_exact(&mut c).map_err(|e| e.to_string())?;
                channels = u16::from_be_bytes([c[0], c[1]]) as usize;
                bits = u16::from_be_bytes([c[6], c[7]]) as usize;
                sr = decode_extended_80(c[8..18].try_into().unwrap());
                if size >= 22 {
                    let comp = &c[18..22];
                    float = comp == b"fl32" || comp == b"FL32";
                    if !(float || comp == b"NONE" || comp == b"twos") {
                        return Err(format!("unsupported AIFC compression {}", String::from_utf8_lossy(comp)));
                    }
                }
                if size % 2 == 1 {
                    r.seek(SeekFrom::Current(1)).map_err(|e| e.to_string())?;
                }
            }
            b"SSND" => {
                let mut h = [0u8; 8];
                r.read_exact(&mut h).map_err(|e| e.to_string())?;
                let offset = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
                r.seek(SeekFrom::Current(offset as i64)).map_err(|e| e.to_string())?;
                let mut d = vec![0u8; size.saturating_sub(8 + offset)];
                let got = r.read(&mut d).map_err(|e| e.to_string())?;
                d.truncate(got);
                pcm = Some(d);
                break;
            }
            _ => {
                r.seek(SeekFrom::Current((size + size % 2) as i64)).map_err(|e| e.to_string())?;
            }
        }
    }
    let pcm = pcm.ok_or("AIFF without sound data")?;
    let data: Vec<f32> = match (float, bits) {
        (true, _) => pcm.chunks_exact(4).map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]])).collect(),
        (false, 8) => pcm.iter().map(|&b| b as i8 as f32 / 128.0).collect(),
        (false, 16) => pcm.chunks_exact(2).map(|b| i16::from_be_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
        (false, 24) => pcm
            .chunks_exact(3)
            .map(|b| (i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (false, 32) => pcm
            .chunks_exact(4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        _ => return Err(format!("unsupported bit depth {bits}")),
    };
    Ok((sr, channels, data))
}

/// Linear-interpolation resampler (the snippet is converted once, on load).
pub fn resample(src: &[f32], from: f64, to: f64) -> Vec<f32> {
    if (from - to).abs() < 1e-9 || src.is_empty() {
        return src.to_vec();
    }
    let step = from / to;
    let n = ((src.len() as f64) / step).floor() as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * step;
            let k = p as usize;
            let f = (p - k as f64) as f32;
            let a = src[k];
            let b = *src.get(k + 1).unwrap_or(&a);
            a + (b - a) * f
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Int16,
    Int24,
    Float32,
}

impl SampleFormat {
    fn bits(self) -> u16 {
        match self {
            SampleFormat::Int16 => 16,
            SampleFormat::Int24 => 24,
            SampleFormat::Float32 => 32,
        }
    }
}

/// Writes interleaved `channels`-channel audio: AIFF (AIFC for float) for
/// `.aif`/`.aiff`, WAV otherwise.
pub fn save(path: &Path, sample_rate: f64, channels: u16, interleaved: &[f32], fmt: SampleFormat) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "aif" | "aiff" | "aifc") {
        write_aiff(path, sample_rate, channels, interleaved, fmt)
    } else {
        let spec = hound::WavSpec {
            channels,
            sample_rate: sample_rate.round() as u32,
            bits_per_sample: fmt.bits(),
            sample_format: if fmt == SampleFormat::Float32 { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
        };
        let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        for &x in interleaved {
            match fmt {
                SampleFormat::Int16 => w.write_sample(to_int(x, 32768.0) as i16),
                SampleFormat::Int24 => w.write_sample(to_int(x, 8_388_608.0)),
                SampleFormat::Float32 => w.write_sample(x),
            }
            .map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())
    }
}

fn write_aiff(path: &Path, sr: f64, channels: u16, x: &[f32], fmt: SampleFormat) -> Result<(), String> {
    let float = fmt == SampleFormat::Float32;
    let frames = (x.len() / channels as usize) as u32;
    let data_bytes = x.len() as u32 * (fmt.bits() as u32 / 8);
    let comm_len: u32 = if float { 18 + 4 + 14 } else { 18 };
    let fver_len: u32 = if float { 12 } else { 0 };
    let mut out = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let mut h = Vec::with_capacity(80);
    h.extend_from_slice(b"FORM");
    h.extend_from_slice(&(4 + fver_len + 8 + comm_len + 16 + data_bytes).to_be_bytes());
    h.extend_from_slice(if float { b"AIFC" } else { b"AIFF" });
    if float {
        h.extend_from_slice(b"FVER");
        h.extend_from_slice(&4u32.to_be_bytes());
        h.extend_from_slice(&0xA280_5140u32.to_be_bytes());
    }
    h.extend_from_slice(b"COMM");
    h.extend_from_slice(&comm_len.to_be_bytes());
    h.extend_from_slice(&channels.to_be_bytes());
    h.extend_from_slice(&frames.to_be_bytes());
    h.extend_from_slice(&fmt.bits().to_be_bytes());
    h.extend_from_slice(&encode_extended_80(sr));
    if float {
        h.extend_from_slice(b"fl32");
        h.push(12);
        h.extend_from_slice(b"32-bit float");
        h.push(0);
    }
    h.extend_from_slice(b"SSND");
    h.extend_from_slice(&(8 + data_bytes).to_be_bytes());
    h.extend_from_slice(&[0u8; 8]);
    out.write_all(&h).map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(x.len() * 4);
    for &s in x {
        match fmt {
            SampleFormat::Int16 => buf.extend_from_slice(&(to_int(s, 32768.0) as i16).to_be_bytes()),
            SampleFormat::Int24 => buf.extend_from_slice(&to_int(s, 8_388_608.0).to_be_bytes()[1..]),
            SampleFormat::Float32 => buf.extend_from_slice(&s.to_be_bytes()),
        }
    }
    out.write_all(&buf).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

#[inline]
fn to_int(x: f32, scale: f32) -> i32 {
    (x * scale).round().clamp(-scale, scale - 1.0) as i32
}

fn decode_extended_80(b: &[u8; 10]) -> f64 {
    let sign = b[0] & 0x80 != 0;
    let exp = u16::from_be_bytes([b[0] & 0x7f, b[1]]) as i32;
    let mant = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exp == 0 && mant == 0 {
        return 0.0;
    }
    let v = (mant as f64) / (1u64 << 63) as f64 * 2f64.powi(exp - 16383);
    if sign { -v } else { v }
}

fn encode_extended_80(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if v <= 0.0 || !v.is_finite() {
        return out;
    }
    let exp = v.log2().floor() as i32;
    let mant = (v / 2f64.powi(exp) * (1u64 << 63) as f64) as u64;
    out[..2].copy_from_slice(&((exp + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aiff_and_wav_round_trip() {
        let dir = std::env::temp_dir();
        let x: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        for (name, fmt) in [("rr_t.aif", SampleFormat::Int24), ("rr_t.aiff", SampleFormat::Float32), ("rr_t.wav", SampleFormat::Int16)] {
            let p = dir.join(name);
            save(&p, 48000.0, 1, &x, fmt).unwrap();
            let d = load(&p).unwrap();
            assert_eq!(d.sample_rate, 48000.0);
            assert_eq!(d.samples.len(), 1000);
            assert!((d.samples[500] - x[500]).abs() < 1e-4);
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn extended_80() {
        for sr in [22050.0, 44100.0, 48000.0, 96000.0] {
            assert_eq!(decode_extended_80(&encode_extended_80(sr)), sr);
        }
    }
}
