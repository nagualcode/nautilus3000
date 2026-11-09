//! The input screen as persisted plug-in state.
//!
//! The original wrote the screen to an SD II file in
//! `/Library/Application Support/Riverrun Scratch ƒ/Riverrun Snippets` when a
//! song closed, and stored that file's path in the song. Here the 4.65 s of
//! audio are embedded in the project itself (about 0.9 MB at 48 kHz).
//!
//! Two directions:
//! * `request` — a snippet the engine should load (from a file, the demo, or a
//!   restored project), picked up by the audio thread via `generation()`;
//! * `mirror` — what the engine currently holds, copied by the audio thread
//!   while recording and after every load. This is what gets saved and what the
//!   editor draws.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use truce::core::custom_state::{PersistField, StateCursor};

const FORMAT_VERSION: u8 = 1;

#[derive(Clone, Default)]
pub struct SnippetData {
    pub name: String,
    pub sample_rate: f64,
    /// Mono, oldest sample first.
    pub samples: Vec<f32>,
}

#[derive(Default)]
pub struct SnippetSlot {
    request: RwLock<Option<Arc<SnippetData>>>,
    request_gen: AtomicU64,
    pub mirror: Mutex<SnippetData>,
    mirror_gen: AtomicU64,
}

impl SnippetSlot {
    /// Ask the engine to load `data` (UI or host thread).
    pub fn load(&self, data: SnippetData) {
        if let Ok(mut m) = self.mirror.lock() {
            m.name = data.name.clone();
        }
        if let Ok(mut g) = self.request.write() {
            *g = Some(Arc::new(data));
        }
        self.request_gen.fetch_add(1, Ordering::Release);
    }

    pub fn generation(&self) -> u64 {
        self.request_gen.load(Ordering::Acquire)
    }

    /// Non-blocking read for the audio thread (an `Arc` clone).
    pub fn try_request(&self) -> Option<Option<Arc<SnippetData>>> {
        self.request.try_read().ok().map(|g| g.clone())
    }

    pub fn mirror_generation(&self) -> u64 {
        self.mirror_gen.load(Ordering::Acquire)
    }

    pub fn bump_mirror(&self) {
        self.mirror_gen.fetch_add(1, Ordering::Release);
    }

    pub fn name(&self) -> String {
        self.mirror.lock().map(|m| m.name.clone()).unwrap_or_default()
    }

    /// A copy of the screen for saving to a file.
    pub fn snapshot(&self) -> Option<SnippetData> {
        let m = self.mirror.lock().ok()?;
        (!m.samples.is_empty()).then(|| m.clone())
    }
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn get_u32(c: &mut StateCursor) -> Option<u32> {
    Some(u32::from_le_bytes(c.read_bytes(4)?.try_into().ok()?))
}

fn get_str(c: &mut StateCursor) -> Option<String> {
    let n = get_u32(c)? as usize;
    String::from_utf8(c.read_bytes(n)?.to_vec()).ok()
}

impl PersistField for SnippetSlot {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        buf.push(FORMAT_VERSION);
        let Ok(m) = self.mirror.lock() else {
            buf.push(0);
            return;
        };
        if m.samples.iter().all(|&s| s == 0.0) {
            buf.push(0);
            return;
        }
        buf.push(1);
        put_str(buf, &m.name);
        buf.extend_from_slice(&m.sample_rate.to_le_bytes());
        buf.extend_from_slice(&(m.samples.len() as u32).to_le_bytes());
        buf.reserve(m.samples.len() * 4);
        for s in &m.samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
    }

    fn persist_read(&self, c: &mut StateCursor) {
        let Some(v) = c.read_bytes(1) else { return };
        if v[0] != FORMAT_VERSION {
            return;
        }
        let Some(has) = c.read_bytes(1) else { return };
        if has[0] == 0 {
            return;
        }
        let Some(name) = get_str(c) else { return };
        let Some(sr) = c.read_bytes(8).and_then(|b| b.try_into().ok()).map(f64::from_le_bytes) else {
            return;
        };
        let Some(n) = get_u32(c) else { return };
        let Some(raw) = c.read_bytes(n as usize * 4) else { return };
        let samples = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        self.load(SnippetData { name, sample_rate: sr, samples });
    }
}
