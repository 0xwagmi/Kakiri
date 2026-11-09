//! Streaming playback voice: a per-voice decoder thread feeds a small SPSC ring
//! which the mixer consumes. Memory is bounded (≈2 s) regardless of file length,
//! and playback starts as soon as the first packet is decoded.
//!
//! Seeking is generation-based: the mixer discards buffered audio while the
//! decoder performs the seek, so no stale audio is ever played.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::bus::BusFormat;
use crate::decode::StreamDecoder;

const RING_SECONDS: usize = 2;

pub struct StreamControl {
    stop: Arc<AtomicBool>,
    seek: AtomicI64, // requested target frame, -1 = none
    generation: AtomicU64,
    applied: AtomicU64,
    looped: AtomicBool,
    played: AtomicU64, // frames consumed by the mixer
    total: AtomicU64,  // total source frames (0 = unknown)
    eof: AtomicBool,
    /// set once the first audio is in the ring, so a network source can prebuffer
    ready: AtomicBool,
    /// paused voices keep their buffer and position, but hand the mixer silence
    paused: AtomicBool,
}

impl StreamControl {
    /// The stop flag, for background work that should end with this voice.
    pub(crate) fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
}

pub struct StreamVoice {
    consumer: Consumer<f32>,
    control: Arc<StreamControl>,
    channels: usize,
}

impl StreamVoice {
    /// Read up to `out.len()` interleaved samples into `out`. Returns the number
    /// written (0 when nothing is available yet, or while seeking).
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        if self.control.paused.load(Ordering::Relaxed) {
            // silence, and `eof` stays false, so the mixer keeps the voice alive
            return 0;
        }
        let gen = self.control.generation.load(Ordering::Relaxed);
        if self.control.applied.load(Ordering::Relaxed) != gen {
            while self.consumer.pop().is_ok() {} // discard stale audio during seek
            return 0;
        }
        let mut n = 0;
        while n < out.len() {
            match self.consumer.pop() {
                Ok(s) => {
                    out[n] = s;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        if n > 0 {
            self.control
                .played
                .fetch_add((n / self.channels) as u64, Ordering::Relaxed);
        }
        n
    }

    pub fn eof(&self) -> bool {
        self.control.eof.load(Ordering::Relaxed)
    }

    pub fn total_frames(&self) -> u64 {
        self.control.total.load(Ordering::Relaxed)
    }

    /// Frames consumed so far since the voice started (source frames).
    pub fn played_frames(&self) -> u64 {
        self.control.played.load(Ordering::Relaxed)
    }

    pub fn progress(&self) -> f32 {
        let total = self.control.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let played = self.control.played.load(Ordering::Relaxed);
        let pos = if self.control.looped.load(Ordering::Relaxed) {
            played % total
        } else {
            played.min(total)
        };
        (pos as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// Pause or resume. Nothing is consumed while paused, so the position and the
    /// buffered audio are exactly where they were — just silence to the mixer.
    pub fn set_paused(&self, paused: bool) {
        self.control.paused.store(paused, Ordering::Relaxed);
    }

    pub fn paused(&self) -> bool {
        self.control.paused.load(Ordering::Relaxed)
    }

    pub fn set_looped(&self, on: bool) {
        self.control.looped.store(on, Ordering::Relaxed);
    }

    pub fn request_seek(&self, frame: u64) {
        self.control.seek.store(frame as i64, Ordering::Relaxed);
        self.control.generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for StreamVoice {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
    }
}

/// Play a direct stream URL -- typically what `yt-dlp -g` resolves a link to.
///
/// `url` is a short-lived CDN URL; the caller resolves it per playback and
/// re-resolves when it fails. Waits for the first packet to be buffered (up to
/// `prebuffer`) so playback starts with sound rather than silence, and returns
/// how long that took, so the UI can show an honest "loading" state.
pub fn spawn_url(
    url: String,
    fmt: BusFormat,
    looped: bool,
    prebuffer: Duration,
    cache: Option<std::path::PathBuf>,
) -> Result<(StreamVoice, Duration)> {
    let started = std::time::Instant::now();

    // streamed through once before? then this is a local file read, no network
    let mut prefetcher = None;
    let decoder = match cache.as_ref().filter(|p| crate::http::is_complete(p)) {
        Some(path) => StreamDecoder::open(path, fmt)?,
        None => {
            let (source, handle) = crate::http::HttpSource::open_cached(&url, cache)?;
            let mut hint = symphonia::core::formats::probe::Hint::new();
            if let Some(ext) = source.extension() {
                hint.with_extension(ext);
            }
            prefetcher = handle;
            StreamDecoder::open_source(Box::new(source), &hint, fmt)?
        }
    };
    let total = decoder.total_frames();
    let (producer, consumer) = RingBuffer::<f32>::new(RING_SECONDS * fmt.rate as usize * fmt.channels as usize);
    let control = Arc::new(StreamControl {
        stop: Arc::new(AtomicBool::new(false)),
        seek: AtomicI64::new(-1),
        generation: AtomicU64::new(0),
        applied: AtomicU64::new(0),
        looped: AtomicBool::new(looped),
        played: AtomicU64::new(0),
        total: AtomicU64::new(total),
        eof: AtomicBool::new(false),
        ready: AtomicBool::new(false),
        paused: AtomicBool::new(false),
    });
    // keep caching the rest of the track in the background: it survives pausing,
    // and stops when this voice is stopped
    if let Some(handle) = prefetcher {
        let url2 = url.clone();
        let stop2 = control.clone();
        thread::Builder::new()
            .name("voice-prefetch".into())
            .spawn(move || handle.prefetch(url2, stop2.stop_flag()))
            .ok();
    }
    let ctl = control.clone();
    thread::Builder::new()
        .name("voice-decoder-net".into())
        .spawn(move || decode_loop(decoder, producer, ctl))
        .expect("spawn net voice decoder");

    // wait for the first audio to land, but never longer than `prebuffer`
    let deadline = std::time::Instant::now() + prebuffer;
    while !control.ready.load(Ordering::Relaxed)
        && !control.eof.load(Ordering::Relaxed)
        && std::time::Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    Ok((
        StreamVoice { consumer, control, channels: fmt.channels as usize },
        started.elapsed(),
    ))
}

/// Open `path`, start a decoder thread, and return a voice to mix.
pub fn spawn(path: PathBuf, fmt: BusFormat, looped: bool) -> Result<StreamVoice> {
    spawn_from(path, fmt, looped, 0.0)
}

/// Like [`spawn`], but begins `skip_secs` into the file.
///
/// The seek happens *before* the decoder thread starts producing, so playback
/// begins exactly at the requested position with no audible jump. Used by the
/// timeline when you scrub into the middle of a clip.
pub fn spawn_from(
    path: PathBuf,
    fmt: BusFormat,
    looped: bool,
    skip_secs: f64,
) -> Result<StreamVoice> {
    let mut decoder = StreamDecoder::open(&path, fmt)?;
    let total = decoder.total_frames();
    let mut played = 0u64;
    if skip_secs > 0.0 && total > 0 {
        let dur = decoder.duration_secs() as f64;
        let target = ((skip_secs.min(dur) / dur.max(1e-9)) * total as f64) as u64;
        if target > 0 && target < total {
            if decoder.seek(target).is_ok() {
                played = target;
            }
        }
    }
    let (producer, consumer) = RingBuffer::<f32>::new(RING_SECONDS * fmt.rate as usize * fmt.channels as usize);
    let control = Arc::new(StreamControl {
        stop: Arc::new(AtomicBool::new(false)),
        seek: AtomicI64::new(-1),
        generation: AtomicU64::new(0),
        applied: AtomicU64::new(0),
        looped: AtomicBool::new(looped),
        played: AtomicU64::new(played),
        total: AtomicU64::new(total),
        eof: AtomicBool::new(false),
        ready: AtomicBool::new(true),
        paused: AtomicBool::new(false),
    });
    let ctl = control.clone();
    thread::Builder::new()
        .name("voice-decoder".into())
        .spawn(move || decode_loop(decoder, producer, ctl))
        .expect("spawn voice decoder");

    Ok(StreamVoice {
        consumer,
        control,
        channels: fmt.channels as usize,
    })
}

fn decode_loop(mut decoder: StreamDecoder, mut producer: Producer<f32>, ctl: Arc<StreamControl>) {
    let mut chunk: Vec<f32> = Vec::with_capacity(8192);
    let mut last_gen = ctl.generation.load(Ordering::Relaxed);

    loop {
        if ctl.stop.load(Ordering::Relaxed) {
            return;
        }
        // paused: don't keep pulling from the network into a ring nobody reads
        if ctl.paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        // Apply a pending seek before producing more audio.
        let gen = ctl.generation.load(Ordering::Relaxed);
        if gen != last_gen {
            let target = ctl.seek.load(Ordering::Relaxed).max(0) as u64;
            if decoder.seek(target).is_err() {
                // seek unsupported → restart from the top as a fallback
                let _ = decoder.seek(0);
            }
            ctl.played.store(target, Ordering::Relaxed);
            last_gen = gen;
            ctl.applied.store(gen, Ordering::Relaxed);
        }

        chunk.clear();
        match decoder.next_chunk(&mut chunk) {
            Ok(0) => {
                if ctl.looped.load(Ordering::Relaxed) {
                    let _ = decoder.seek(0);
                    ctl.played.store(0, Ordering::Relaxed);
                    continue;
                }
                ctl.eof.store(true, Ordering::Relaxed);
                return;
            }
            Ok(_) => {
                let mut i = 0;
                while i < chunk.len() {
                    if ctl.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let free = producer.slots();
                    if free == 0 {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    let n = free.min(chunk.len() - i);
                    if producer.push_entire_slice(&chunk[i..i + n]).is_err() {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    i += n;
                }
            }
            Err(_) => {
                ctl.eof.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusFormat;

    /// A tiny mono 48 kHz WAV, so the decoder has something real to read.
    fn wav(path: &std::path::Path, frames: usize) {
        let mut out = Vec::new();
        let data_len = (frames * 2) as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // pcm
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&48_000u32.to_le_bytes());
        out.extend_from_slice(&96_000u32.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn pause_hands_the_mixer_silence_and_keeps_the_place() {
        let dir = std::env::temp_dir().join("kakiri-pause-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        wav(&path, 48_000 * 10); // long enough that the 2 s ring stays unfilled

        let fmt = BusFormat { rate: 48_000, channels: 1 };
        let mut voice = spawn(path, fmt, false).unwrap();

        // let the decoder thread fill the ring
        std::thread::sleep(Duration::from_millis(120));
        let mut buf = vec![0.0f32; 512];
        let first = voice.read(&mut buf);
        assert!(first > 0, "nothing came out of the decoder");
        let played_after_first = voice.played_frames();

        let eof_before = voice.eof();
        voice.set_paused(true);
        assert!(voice.paused());
        std::thread::sleep(Duration::from_millis(60));
        let n = voice.read(&mut buf);
        assert_eq!(n, 0, "a paused voice must not hand out audio");
        assert_eq!(voice.played_frames(), played_after_first, "position moved while paused");
        assert_eq!(voice.eof(), eof_before, "pausing must not touch the eof flag");

        voice.set_paused(false);
        std::thread::sleep(Duration::from_millis(60));
        let resumed = voice.read(&mut buf);
        assert!(resumed > 0, "resume should continue from the same place");
        assert_eq!(voice.played_frames(), played_after_first + resumed as u64);
    }
}
