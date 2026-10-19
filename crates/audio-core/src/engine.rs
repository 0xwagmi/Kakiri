//! The engine owns the whole mix: voices, an optional live input (your voice),
//! a mic output and an optional monitor output — plus the threads that move it.
//!
//! Because the mic capture happens *here*, "talk + play sounds at once" needs no
//! external `pw-link` wiring: the engine sums them and emits one stream.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;

use crate::bus::{BusFormat, Pcm};
use crate::input::AudioInput;
use crate::mixer::{self, Mixer, Route, VoiceId, VoiceInfo};
use crate::output::AudioOutput;

const BLOCK_FRAMES: usize = 512;
const INPUT_RING_SECS: usize = 1;
const INPUT_POLL: Duration = Duration::from_millis(2);

/// A ring of captured input samples awaiting the next render block. Keeps the
/// live input decoupled from the output clock (small, bounded, drops oldest).
struct InputRing {
    buf: VecDeque<f32>,
    cap: usize,
}

impl InputRing {
    fn new(channels: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap: (48_000 * channels * INPUT_RING_SECS).max(1),
        }
    }
    fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples.iter().copied());
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
    }
    fn mix_into(&mut self, out: &mut [f32], gain: f32) {
        let n = out.len().min(self.buf.len());
        for i in 0..n {
            out[i] += self.buf[i] * gain;
        }
        self.buf.drain(0..n);
    }
    fn clear(&mut self) {
        self.buf.clear();
    }
}

struct InputSlot {
    input: Option<Box<dyn AudioInput>>,
}

pub struct Engine {
    format: BusFormat,
    mixer: Arc<Mutex<Mixer>>,
    mic: Arc<Mutex<Box<dyn AudioOutput>>>,
    monitor: Arc<Mutex<Option<Box<dyn AudioOutput>>>>,
    input: Arc<Mutex<InputSlot>>,
    input_ring: Arc<Mutex<InputRing>>,
    input_gain: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    next_id: AtomicU64,
    writer: Mutex<Option<JoinHandle<()>>>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(format: BusFormat, mut mic: Box<dyn AudioOutput>) -> Arc<Self> {
        if let Err(e) = mic.start() {
            eprintln!("[engine] mic output start failed: {e}");
        }
        let engine = Arc::new(Self {
            format,
            mixer: Arc::new(Mutex::new(Mixer::new(format.channels))),
            mic: Arc::new(Mutex::new(mic)),
            monitor: Arc::new(Mutex::new(None)),
            input: Arc::new(Mutex::new(InputSlot { input: None })),
            input_ring: Arc::new(Mutex::new(InputRing::new(format.channels as usize))),
            input_gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            running: Arc::new(AtomicBool::new(true)),
            next_id: AtomicU64::new(1),
            writer: Mutex::new(None),
            reader: Mutex::new(None),
        });
        engine.clone().spawn_writer();
        engine.clone().spawn_reader();
        engine
    }

    pub fn format(&self) -> BusFormat {
        self.format
    }

    fn spawn_writer(self: Arc<Self>) {
        let block = BLOCK_FRAMES * self.format.channels as usize;
        let mixer = self.mixer.clone();
        let mic = self.mic.clone();
        let monitor = self.monitor.clone();
        let ring = self.input_ring.clone();
        let gain = self.input_gain.clone();
        let running = self.running.clone();

        let handle = thread::Builder::new()
            .name("audio-writer".into())
            .spawn(move || {
                let mut mic_buf = vec![0f32; block];
                let mut mon_buf = vec![0f32; block];
                while running.load(Ordering::Relaxed) {
                    let mut mon_guard = monitor.lock().unwrap();
                    let has_mon = mon_guard.is_some();
                    {
                        let mut mx = mixer.lock().unwrap();
                        let mon_slice = if has_mon {
                            Some(mon_buf.as_mut_slice())
                        } else {
                            None
                        };
                        mx.render(&mut mic_buf, mon_slice);
                    }
                    // the live input goes to the mic bus only (not the monitor,
                    // which would echo your own voice back at you)
                    {
                        let g = f32::from_bits(gain.load(Ordering::Relaxed));
                        ring.lock().unwrap().mix_into(&mut mic_buf, g);
                    }
                    mixer::soft_limit(&mut mic_buf);

                    let mic_failed = {
                        let mut out = mic.lock().unwrap();
                        out.push(&mic_buf).is_err()
                    };
                    if let Some(m) = mon_guard.as_mut() {
                        let _ = m.push(&mon_buf);
                    }
                    drop(mon_guard);
                    if mic_failed {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            })
            .expect("spawn audio writer");
        *self.writer.lock().unwrap() = Some(handle);
    }

    fn spawn_reader(self: Arc<Self>) {
        let block = BLOCK_FRAMES * self.format.channels as usize;
        let input = self.input.clone();
        let ring = self.input_ring.clone();
        let running = self.running.clone();

        let handle = thread::Builder::new()
            .name("audio-input".into())
            .spawn(move || {
                let mut buf = vec![0f32; block];
                while running.load(Ordering::Relaxed) {
                    let n = {
                        let mut slot = input.lock().unwrap();
                        match slot.input.as_mut() {
                            Some(inp) => match inp.read(&mut buf) {
                                Ok(n) => n,
                                Err(e) => {
                                    eprintln!("[engine] input ended: {e}");
                                    slot.input = None; // stop trying; avoids a spin
                                    0
                                }
                            },
                            None => 0,
                        }
                    };
                    if n > 0 {
                        ring.lock().unwrap().push(&buf[..n]);
                    } else {
                        thread::sleep(INPUT_POLL);
                    }
                }
            })
            .expect("spawn audio reader");
        *self.reader.lock().unwrap() = Some(handle);
    }

    /// Swap the virtual-mic output. Starts the new one, then drops (stops) the old.
    pub fn set_output(&self, mut output: Box<dyn AudioOutput>) -> Result<()> {
        output.start()?;
        let old = {
            let mut guard = self.mic.lock().unwrap();
            std::mem::replace(&mut *guard, output)
        };
        drop(old);
        Ok(())
    }

    /// Enable/replace (`Some`) or disable (`None`) the local monitor output.
    pub fn set_monitor(&self, mut output: Option<Box<dyn AudioOutput>>) -> Result<()> {
        if let Some(o) = output.as_mut() {
            o.start()?;
        }
        let old = {
            let mut guard = self.monitor.lock().unwrap();
            std::mem::replace(&mut *guard, output)
        };
        drop(old);
        Ok(())
    }

    pub fn monitor_enabled(&self) -> bool {
        self.monitor.lock().unwrap().is_some()
    }

    /// Enable/replace (`Some`) or disable (`None`) the live voice input.
    pub fn set_input(&self, mut input: Option<Box<dyn AudioInput>>) -> Result<()> {
        if let Some(i) = input.as_mut() {
            i.start()?;
        }
        let old = {
            let mut slot = self.input.lock().unwrap();
            std::mem::replace(&mut slot.input, input)
        };
        if let Some(mut o) = old {
            o.stop();
        }
        self.input_ring.lock().unwrap().clear();
        Ok(())
    }

    pub fn input_active(&self) -> bool {
        self.input.lock().unwrap().input.is_some()
    }

    pub fn set_input_gain(&self, gain: f32) {
        self.input_gain
            .store(gain.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play(
        &self,
        key: impl Into<String>,
        label: impl Into<String>,
        pcm: Arc<Pcm>,
        gain: f32,
        looped: bool,
        route: Route,
    ) -> VoiceId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.mixer
            .lock()
            .unwrap()
            .add(id, key.into(), label.into(), pcm, gain, looped, route);
        id
    }

    /// Stop every voice started with this key (for choke/toggle retrigger).
    pub fn stop_key(&self, key: &str) {
        self.mixer.lock().unwrap().stop_key(key);
    }

    pub fn is_key_active(&self, key: &str) -> bool {
        self.mixer.lock().unwrap().is_key_active(key)
    }

    pub fn stop(&self, id: VoiceId) {
        self.mixer.lock().unwrap().stop(id);
    }

    pub fn stop_all(&self) {
        self.mixer.lock().unwrap().stop_all();
    }

    pub fn set_gain(&self, id: VoiceId, gain: f32) {
        self.mixer.lock().unwrap().set_gain(id, gain);
    }

    pub fn seek(&self, id: VoiceId, fraction: f32) {
        self.mixer.lock().unwrap().seek(id, fraction);
    }

    /// Set every playing voice's gain (master volume).
    pub fn set_all_gain(&self, gain: f32) {
        self.mixer.lock().unwrap().set_all_gain(gain);
    }

    pub fn active(&self) -> Vec<VoiceInfo> {
        self.mixer.lock().unwrap().active()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(mut input) = self.input.lock().unwrap().input.take() {
            input.stop();
        }
        if let Some(h) = self.writer.lock().unwrap().take() {
            let _ = h.join();
        }
        if let Some(h) = self.reader.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_ring_mixes_with_gain_and_drains() {
        let mut ring = InputRing::new(2);
        ring.push(&[0.5, 0.5, 0.5, 0.5]);
        let mut out = vec![0.1f32; 4];
        ring.mix_into(&mut out, 0.5);
        assert!((out[0] - 0.35).abs() < 1e-6);
        assert_eq!(ring.buf.len(), 0);
    }

    #[test]
    fn input_ring_drops_oldest_when_full() {
        let mut ring = InputRing::new(2);
        ring.cap = 4;
        ring.push(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
        assert_eq!(ring.buf.len(), 4);
        // oldest (1.0) dropped → front is 2.0
        assert_eq!(ring.buf[0], 2.0);
    }
}
