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
    stop: AtomicBool,
    seek: AtomicI64, // requested target frame, -1 = none
    generation: AtomicU64,
    applied: AtomicU64,
    looped: AtomicBool,
    played: AtomicU64, // frames consumed by the mixer
    total: AtomicU64,  // total source frames (0 = unknown)
    eof: AtomicBool,
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

/// Open `path`, start a decoder thread, and return a voice to mix.
pub fn spawn(path: PathBuf, fmt: BusFormat, looped: bool) -> Result<StreamVoice> {
    let decoder = StreamDecoder::open(&path, fmt)?;
    let total = decoder.total_frames();
    let (producer, consumer) = RingBuffer::<f32>::new(RING_SECONDS * fmt.rate as usize * fmt.channels as usize);
    let control = Arc::new(StreamControl {
        stop: AtomicBool::new(false),
        seek: AtomicI64::new(-1),
        generation: AtomicU64::new(0),
        applied: AtomicU64::new(0),
        looped: AtomicBool::new(looped),
        played: AtomicU64::new(0),
        total: AtomicU64::new(total),
        eof: AtomicBool::new(false),
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
