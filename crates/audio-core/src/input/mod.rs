//! Audio inputs: the live voice source that the engine mixes into the virtual
//! mic alongside the soundboard voices (see docs/ARCHITECTURE.md).
//!
//! This is what lets one Discord input carry *your voice + the sounds* without
//! any external `pw-link` wiring.

use anyhow::Result;

pub mod cpal_in;
pub use cpal_in::CpalInput;

#[cfg(target_os = "linux")]
pub mod pw_cli;
#[cfg(target_os = "linux")]
pub mod pw_native;

#[cfg(target_os = "linux")]
pub use pw_cli::PipeWireCliInput;
#[cfg(target_os = "linux")]
pub use pw_native::PipeWireInput;

/// A source of interleaved f32 PCM at the bus format.
///
/// `read` is **non-blocking**: it returns as soon as it has some data, or `0`
/// when none is available yet. `Err` means the stream ended (the engine then
/// clears the input). This keeps the reader thread cancellable.
pub trait AudioInput: Send {
    fn start(&mut self) -> Result<()>;
    fn read(&mut self, out: &mut [f32]) -> Result<usize>;
    fn stop(&mut self);
}

/// Produces no audio. Used on platforms without an input backend yet, and in
/// tests.
pub struct NullInput;

impl AudioInput for NullInput {
    fn start(&mut self) -> Result<()> {
        Ok(())
    }
    fn read(&mut self, _out: &mut [f32]) -> Result<usize> {
        Ok(0)
    }
    fn stop(&mut self) {}
}
