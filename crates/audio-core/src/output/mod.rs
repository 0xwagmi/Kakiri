//! Audio outputs: where the mixed bus is drained to.
//!
//! ADR-4: one `AudioOutput` trait; on Linux we target a specific PipeWire node
//! with `pw-play --target`, on Windows/macOS we would use `CpalOutput`.

use anyhow::{anyhow, Result};

#[cfg(target_os = "linux")]
pub mod pw_cli;
#[cfg(target_os = "linux")]
pub mod pw_native;

#[cfg(target_os = "linux")]
pub use pw_cli::PipeWireCliOutput;
#[cfg(target_os = "linux")]
pub use pw_native::PipeWireOutput;

/// A sink for the interleaved f32 mix bus.
pub trait AudioOutput: Send {
    /// Open the device / spawn the helper. Idempotent.
    fn start(&mut self) -> Result<()>;
    /// Write one block of interleaved f32 samples. May block briefly.
    fn push(&mut self, interleaved: &[f32]) -> Result<()>;
    /// Stop and release resources. Idempotent.
    fn stop(&mut self);
    /// Rough output latency, for UI/telemetry.
    fn latency_hint_ms(&self) -> u32;
}

/// Discards everything that is written. Useful for tests and for a
/// "muted to mic" state that keeps the pipeline running.
pub struct NullOutput;

impl AudioOutput for NullOutput {
    fn start(&mut self) -> Result<()> {
        Ok(())
    }
    fn push(&mut self, _interleaved: &[f32]) -> Result<()> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn latency_hint_ms(&self) -> u32 {
        0
    }
}

/// Convenience for callers that need to check the output is live.
pub(crate) fn not_started() -> anyhow::Error {
    anyhow!("audio output is not started")
}
