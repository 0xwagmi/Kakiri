//! Cross-platform output via `cpal` (WASAPI on Windows, CoreAudio on macOS).
//!
//! Mirrors the PipeWire backend: the engine writer pushes interleaved f32 into
//! a ring; a cpal output stream (built on its own thread, so we don't rely on
//! `cpal::Stream: Send`) drains it. The device is chosen **by name** (e.g.
//! `CABLE Input` on Windows, `BlackHole 2ch` on macOS).

use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::AudioOutput;

/// Ring slack. Small = low latency; too small underruns.
const RING_FRAMES: usize = 4096;

pub struct CpalOutput {
    target: String,
    rate: u32,
    channels: u16,
    producer: Option<rtrb::Producer<f32>>,
    quit: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl CpalOutput {
    pub fn new(target: impl Into<String>, rate: u32, channels: u16) -> Self {
        Self {
            target: target.into(),
            rate,
            channels,
            producer: None,
            quit: None,
            thread: None,
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

fn find_output_device(host: &cpal::Host, target: &str) -> Option<cpal::Device> {
    if target.is_empty() {
        return host.default_output_device();
    }
    host.output_devices()
        .ok()?
        .find(|d| d.to_string() == target || d.to_string().contains(target))
}

fn build_stream(
    target: &str,
    channels: u16,
    rate: u32,
    mut consumer: rtrb::Consumer<f32>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = find_output_device(&host, target)
        .ok_or_else(|| anyhow!("output device not found: {target:?}"))?;
    let config = cpal::StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for s in data.iter_mut() {
                *s = consumer.pop().unwrap_or(0.0);
            }
        },
        |e| eprintln!("[cpal-out] {e}"),
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

impl AudioOutput for CpalOutput {
    fn start(&mut self) -> Result<()> {
        if self.producer.is_some() {
            return Ok(());
        }
        let cap = RING_FRAMES * self.channels as usize;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cap);
        let (tx, rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        let target = self.target.clone();
        let rate = self.rate;
        let channels = self.channels;
        let handle = thread::Builder::new()
            .name("cpal-out".into())
            .spawn(move || match build_stream(&target, channels, rate, consumer) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = rx.recv(); // keep the stream alive until asked to stop
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })?;

        match ready_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!(e)),
            Err(_) => {} // not ready yet — it comes up on its own thread
        }

        self.producer = Some(producer);
        self.quit = Some(tx);
        self.thread = Some(handle);
        Ok(())
    }

    fn push(&mut self, interleaved: &[f32]) -> Result<()> {
        let producer = self
            .producer
            .as_mut()
            .ok_or_else(|| anyhow!("output not started"))?;
        let mut rest = interleaved;
        while !rest.is_empty() {
            let free = producer.slots();
            if free == 0 {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            let n = free.min(rest.len());
            match producer.push_entire_slice(&rest[..n]) {
                Ok(()) => rest = &rest[n..],
                Err(_) => thread::sleep(Duration::from_millis(1)),
            }
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.producer = None;
        if let Some(q) = self.quit.take() {
            let _ = q.send(());
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }

    fn latency_hint_ms(&self) -> u32 {
        (RING_FRAMES as u32 * 1000) / self.rate.max(1)
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.stop();
    }
}
