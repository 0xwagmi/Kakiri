//! audio-core — the real-time audio engine for discord-soundboard.
//!
//! Design notes live in `docs/ARCHITECTURE.md` and `docs/DECISIONS.md`.
//! The Linux output path is ADR-4: our own mix bus drained into a specific
//! PipeWire node via `pw-play --target <node>`.

pub mod bus;
pub mod decode;
pub mod devices;
pub mod engine;
pub mod input;
pub mod library;
pub mod mixer;
pub mod output;
#[cfg(target_os = "linux")]
pub mod pw_common;

pub use bus::BusFormat;
