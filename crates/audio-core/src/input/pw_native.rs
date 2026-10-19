//! Native PipeWire capture backend (Linux, ADR-4 option C): reads a source
//! (your mic / Easy Effects Source) straight into a ring, no `pw-record`.

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

use super::AudioInput;
use crate::pw_common;

/// Capture slack (~1s). Drained promptly by `read`, so it doesn't add latency.
const RING_FRAMES: usize = 48_000;

struct InData {
    producer: rtrb::Producer<f32>,
}

pub struct PipeWireInput {
    target: String,
    rate: u32,
    channels: u16,
    consumer: Option<rtrb::Consumer<f32>>,
    quit: Option<pw::channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PipeWireInput {
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

impl AudioInput for PipeWireInput {
    fn start(&mut self) -> Result<()> {
        if self.consumer.is_some() {
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
            .name("pw-in".into())
            .spawn(move || {
                let data = InData { producer };
                if let Err(e) = run_input(target, rate, channels, data, rx, &ready_tx) {
                    eprintln!("[pw-in] {e}");
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            })?;

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!(e)),
            Err(e) => return Err(anyhow!("pipewire input: start timeout: {e}")),
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

impl Drop for PipeWireInput {
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

fn run_input(
    target: String,
    rate: u32,
    channels: u16,
    data: InData,
    rx: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), pw::Error> {
    pw_common::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::NODE_NAME => "soundboard-voice-in",
        *pw::keys::AUDIO_CHANNELS => "2",
    };
    if !target.is_empty() {
        props.insert(*pw::keys::TARGET_OBJECT, target.as_str());
    }

    let stream = pw::stream::StreamBox::new(&core, "soundboard-in", props)?;

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
            let size = buf.chunk().size() as usize / 4;
            if let Some(slice) = buf.data() {
                for i in 0..size {
                    let off = i * 4;
                    let sample = f32::from_le_bytes([
                        slice[off],
                        slice[off + 1],
                        slice[off + 2],
                        slice[off + 3],
                    ]);
                    let _ = data.producer.push(sample);
                }
            }
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

    // Pin the buffer to ~1 graph quantum (see output/pw_native.rs).
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
        spa::utils::Direction::Input,
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
