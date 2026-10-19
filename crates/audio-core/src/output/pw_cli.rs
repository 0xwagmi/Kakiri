//! Linux output: drain the mix bus into a named PipeWire node via `pw-play`.
//!
//! This is the proven path from the Python project's `speak.py`. It sidesteps
//! the cpal-on-ALSA problem of not being able to select a PipeWire node by name
//! dynamically (ADR-4). An empty target means "play to the default sink".

#![cfg(target_os = "linux")]

use std::io::Write;
use std::process::{Child, Command, Stdio};

use anyhow::{anyhow, Result};

use super::{not_started, AudioOutput};

pub struct PipeWireCliOutput {
    target: String,
    rate: u32,
    channels: u16,
    child: Option<Child>,
}

impl PipeWireCliOutput {
    pub fn new(target: impl Into<String>, rate: u32, channels: u16) -> Self {
        Self {
            target: target.into(),
            rate,
            channels,
            child: None,
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

impl AudioOutput for PipeWireCliOutput {
    fn start(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new("pw-play");
        if !self.target.is_empty() {
            cmd.args(["--target", &self.target]);
        }
        cmd.args([
            // pw-cat defaults to a 100ms node latency; ask for less so the
            // soundboard round-trips close to the PipeWire graph period.
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
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn pw-play (is pipewire-bin installed?): {e}"))?;
        self.child = Some(child);
        Ok(())
    }

    fn push(&mut self, interleaved: &[f32]) -> Result<()> {
        let child = self.child.as_mut().ok_or_else(not_started)?;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("pw-play has no stdin"))?;
        stdin
            .write_all(bytemuck::cast_slice(interleaved))
            .map_err(|e| anyhow!("pw-play write failed: {e}"))?;
        stdin.flush()?;
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            drop(child.stdin.take()); // EOF → pw-play drains and exits
            let _ = child.wait();
        }
    }

    fn latency_hint_ms(&self) -> u32 {
        80
    }
}

impl Drop for PipeWireCliOutput {
    fn drop(&mut self) {
        self.stop();
    }
}
