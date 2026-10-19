//! Shared PipeWire initialization (must happen once per process).

#![cfg(target_os = "linux")]

use std::sync::Once;

static INIT: Once = Once::new();

pub fn init() {
    INIT.call_once(|| pipewire::init());
}
