//! Native PipeWire playback backend (Linux, ADR-4 option C).
//!
//! Our mix bus is fed straight into a PipeWire stream that targets the virtual
//! mic sink — no `pw-play` subprocess, no pipe. The engine writer thread pushes
//! samples into a small ring; the stream's real-time `process` callback drains
//! it (so `push` blocks when the ring is full, which paces us to realtime).

#![cfg(target_os = "linux")]

use std::io::Cursor;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;

use super::AudioOutput;
use crate::pw_common;

/// A few graph periods of slack. Small = low latency, but too small underruns.
const RING_FRAMES: usize = 2048;

struct OutData {
    consumer: rtrb::Consumer<f32>,
    channels: usize,
}

pub struct PipeWireOutput {
    target: String,
    rate: u32,
    channels: u16,
    producer: Option<rtrb::Producer<f32>>,
    quit: Option<pw::channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PipeWireOutput {
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

impl AudioOutput for PipeWireOutput {
    fn start(&mut self) -> Result<()> {
        if self.producer.is_some() {
            return Ok(());
        }
        let cap = RING_FRAMES * self.channels as usize;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cap);
        let (tx, rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        let target = self.target.clone();
        let rate = self.rate;
        let channels = self.channels;
        let handle = thread::Builder::new()
            .name("pw-out".into())
            .spawn(move || {
                let data = OutData {
                    consumer,
                    channels: channels as usize,
                };
                if let Err(e) = run_output(target, rate, channels, data, rx, &ready_tx) {
                    eprintln!("[pw-out] {e}");
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
                // Ring full → the stream hasn't drained yet; wait (paces to realtime).
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

impl Drop for PipeWireOutput {
    fn drop(&mut self) {
        self.stop();
    }
}

fn to_pod(obj: pw::spa::pod::Object) -> Result<Vec<u8>, pw::Error> {
    let v: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .map_err(|_| pw::Error::CreationFailed)?
    .0
    .into_inner();
    Ok(v)
}

fn run_output(
    target: String,
    rate: u32,
    channels: u16,
    data: OutData,
    rx: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), pw::Error> {
    pw_common::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::AUDIO_CHANNELS => "2",
        *pw::keys::NODE_LATENCY => "1024/48000",
        *pw::keys::NODE_MAX_LATENCY => "1024/48000",
    };
    // Distinct, descriptive node names for the two playback streams.
    let node_name = if target.is_empty() {
        "soundboard-monitor-out"
    } else {
        "soundboard-mic-out"
    };
    props.insert(*pw::keys::NODE_NAME, node_name);
    if !target.is_empty() {
        props.insert(*pw::keys::TARGET_OBJECT, target.as_str());
    }

    let stream = pw::stream::StreamBox::new(&core, "soundboard-out", props)?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let buf = &mut datas[0];
            let stride = std::mem::size_of::<f32>() * data.channels;
            let max_frames = buf.data().as_ref().map(|s| s.len() / stride).unwrap_or(0);
            let requested = buf.chunk().size() as usize / stride.max(1);
            let frames = {
                let Some(slice) = buf.data() else { return };
                // Only produce what the graph asked for; the mapped buffer can be
                // much larger (its maxsize), and over-producing drains our ring.
                let frames = if requested > 0 && requested <= max_frames {
                    requested
                } else {
                    max_frames
                };
                for i in 0..frames * data.channels {
                    let sample = data.consumer.pop().unwrap_or(0.0);
                    let off = i * 4;
                    slice[off..off + 4].copy_from_slice(&sample.to_le_bytes());
                }
                frames
            };
            let chunk = buf.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = (stride * frames) as u32;
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(rate);
    audio_info.set_channels(channels as u32);

    let format_obj = pw::spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };

    // Pin the buffer to ~1 graph quantum. Without this, pw_stream defaults to a
    // huge buffer (16384 frames = 341ms here), which both lags and starves our
    // ring (the callback only refills every 341ms).
    let stride = std::mem::size_of::<f32>() as u32 * channels as u32;
    let quantum_frames: u32 = 1024;
    let buffers_obj = pw::spa::pod::Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_BUFFERS_buffers,
                pw::spa::pod::Value::Int(2),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_BUFFERS_blocks,
                pw::spa::pod::Value::Int(1),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_BUFFERS_size,
                pw::spa::pod::Value::Int((quantum_frames * stride) as i32),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_BUFFERS_stride,
                pw::spa::pod::Value::Int(stride as i32),
            ),
        ],
    };

    let format_vals = to_pod(format_obj)?;
    let format_pod = Pod::from_bytes(&format_vals).ok_or(pw::Error::CreationFailed)?;
    let buffers_vals = to_pod(buffers_obj)?;
    let buffers_pod = Pod::from_bytes(&buffers_vals).ok_or(pw::Error::CreationFailed)?;
    let mut params = [format_pod, buffers_pod];

    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    let _recv = rx.attach(mainloop.loop_(), {
        let ml = mainloop.clone();
        move |_| ml.quit()
    });

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}
