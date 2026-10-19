//! Linux input: capture a PipeWire source (raw mic, Easy Effects Source, …)
//! via `pw-record`, read non-blocking so the reader thread stays cancellable.

#![cfg(target_os = "linux")]

use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};

use anyhow::{anyhow, Result};

use super::AudioInput;

pub struct PipeWireCliInput {
    target: String,
    rate: u32,
    channels: u16,
    child: Option<Child>,
    bytes: Vec<u8>,
}

impl PipeWireCliInput {
    pub fn new(target: impl Into<String>, rate: u32, channels: u16) -> Self {
        Self {
            target: target.into(),
            rate,
            channels,
            child: None,
            bytes: Vec::new(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

fn set_nonblocking(fd: i32) {
    // SAFETY: plain fcntl on a valid fd we own; return values checked.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

impl AudioInput for PipeWireCliInput {
    fn start(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new("pw-record");
        if !self.target.is_empty() {
            cmd.args(["--target", &self.target]);
        }
        cmd.args([
            // pw-cat defaults to a 100ms node latency; lower it for snappier
            // voice passthrough.
            "--latency",
            "40ms",
            "--rate",
            &self.rate.to_string(),
            "--channels",
            &self.channels.to_string(),
            "--format",
            "f32",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn pw-record (is pipewire-bin installed?): {e}"))?;
        if let Some(out) = child.stdout.as_ref() {
            set_nonblocking(out.as_raw_fd());
        }
        self.child = Some(child);
        Ok(())
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let child = self.child.as_mut().ok_or_else(|| anyhow!("input not started"))?;
        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow!("pw-record has no stdout"))?;

        let want = out.len() * 4;
        self.bytes.clear();
        while self.bytes.len() < want {
            let mut chunk = [0u8; 8192];
            let cap = (want - self.bytes.len()).min(chunk.len());
            match stdout.read(&mut chunk[..cap]) {
                Ok(0) => {
                    // EOF — pw-record exited.
                    if self.bytes.is_empty() {
                        return Err(anyhow!("input stream ended"));
                    }
                    break;
                }
                Ok(n) => self.bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if self.bytes.is_empty() {
                        return Ok(0); // nothing available right now
                    }
                    break;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        let samples = self.bytes.len() / 4;
        for i in 0..samples {
            out[i] = f32::from_le_bytes([
                self.bytes[i * 4],
                self.bytes[i * 4 + 1],
                self.bytes[i * 4 + 2],
                self.bytes[i * 4 + 3],
            ]);
        }
        Ok(samples)
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for PipeWireCliInput {
    fn drop(&mut self) {
        self.stop();
    }
}
