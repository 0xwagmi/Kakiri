//! Cross-platform input via `cpal` (the microphone / any capture device).

use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::AudioInput;

const RING_FRAMES: usize = 48_000; // ~1s slack; drained promptly by `read`

pub struct CpalInput {
    target: String,
    rate: u32,
    channels: u16,
    consumer: Option<rtrb::Consumer<f32>>,
    quit: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl CpalInput {
    pub fn new(target: impl Into<String>, rate: u32, channels: u16) -> Self {
        Self {
            target: target.into(),
            rate,
            channels,
            consumer: None,
            quit: None,
            thread: None,
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

fn find_input_device(host: &cpal::Host, target: &str) -> Option<cpal::Device> {
    if target.is_empty() {
        return host.default_input_device();
    }
    host.input_devices()
        .ok()?
        .find(|d| d.to_string() == target || d.to_string().contains(target))
}

fn build_stream(
    target: &str,
    channels: u16,
    rate: u32,
    mut producer: rtrb::Producer<f32>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = find_input_device(&host, target)
        .ok_or_else(|| anyhow!("input device not found: {target:?}"))?;
    let config = cpal::StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &s in data {
                let _ = producer.push(s);
            }
        },
        |e| eprintln!("[cpal-in] {e}"),
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

impl AudioInput for CpalInput {
    fn start(&mut self) -> Result<()> {
        if self.consumer.is_some() {
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
            .name("cpal-in".into())
            .spawn(move || match build_stream(&target, channels, rate, producer) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })?;

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!(e)),
            Err(e) => return Err(anyhow!("cpal input: start timeout: {e}")),
        }

        self.consumer = Some(consumer);
        self.quit = Some(tx);
        self.thread = Some(handle);
        Ok(())
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let consumer = self
            .consumer
            .as_mut()
            .ok_or_else(|| anyhow!("input not started"))?;
        let mut n = 0;
        while n < out.len() {
            match consumer.pop() {
                Ok(s) => {
                    out[n] = s;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        Ok(n)
    }

    fn stop(&mut self) {
        self.consumer = None;
        if let Some(q) = self.quit.take() {
            let _ = q.send(());
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for CpalInput {
    fn drop(&mut self) {
        self.stop();
    }
}
