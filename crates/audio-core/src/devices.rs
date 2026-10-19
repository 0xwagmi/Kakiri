//! Device / node enumeration.
//!
//! - `cpal_outputs()` — OS-level output devices (WASAPI / CoreAudio / ALSA).
//!   Used by the device picker and by the monitor path.
//! - `pipewire_nodes()` — Linux only: the PipeWire nodes we can target by name
//!   with `pw-play --target` (ADR-4). This is how we find the virtual mic.

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub index: usize,
    pub name: String,
    pub channels: u16,
    pub rate: u32,
}

/// Enumerate output devices via cpal.
pub fn cpal_outputs() -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    for (index, dev) in host.output_devices()?.enumerate() {
        let name = dev.to_string();
        let (channels, rate) = match dev.default_output_config() {
            Ok(cfg) => (cfg.channels(), cfg.sample_rate()),
            Err(_) => (0, 0),
        };
        out.push(DeviceInfo {
            index,
            name,
            channels,
            rate,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub class: String,
    pub name: String,
    pub description: String,
    pub serial: u64,
}

impl NodeInfo {
    pub fn is_sink(&self) -> bool {
        self.class == "Audio/Sink"
    }

    pub fn is_source(&self) -> bool {
        self.class.starts_with("Audio/Source")
    }

    /// Heuristic: does this look like a virtual microphone (a sink whose monitor
    /// is exposed, or a loopback source)?
    pub fn looks_like_virtual_mic(&self) -> bool {
        let d = self.description.to_lowercase();
        let n = self.name.to_lowercase();
        d.contains("mic")
            || d.contains("blackhole")
            || d.contains("cable")
            || d.contains("loopback")
            || n.contains("mic")
            || n.contains("loopback")
    }
}

/// Linux: list PipeWire nodes that are audio sinks or sources.
#[cfg(target_os = "linux")]
pub fn pipewire_nodes() -> Result<Vec<NodeInfo>> {
    let out = std::process::Command::new("pw-dump").output()?;
    let data: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut nodes = Vec::new();
    let Some(arr) = data.as_array() else {
        return Ok(nodes);
    };
    for obj in arr {
        if obj.get("type").and_then(|t| t.as_str()) != Some("PipeWire:Interface:Node") {
            continue;
        }
        let props = &obj["info"]["props"];
        let class = props["media.class"].as_str().unwrap_or_default();
        if !(class == "Audio/Sink" || class.starts_with("Audio/Source")) {
            continue;
        }
        nodes.push(NodeInfo {
            class: class.to_string(),
            name: props["node.name"].as_str().unwrap_or_default().to_string(),
            description: props["node.description"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            serial: props["object.serial"].as_u64().unwrap_or(0),
        });
    }
    Ok(nodes)
}

#[cfg(not(target_os = "linux"))]
pub fn pipewire_nodes() -> Result<Vec<NodeInfo>> {
    Ok(Vec::new())
}
