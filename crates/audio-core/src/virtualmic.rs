//! Virtual microphone management.
//!
//! On Linux the virtual mic is a PipeWire loopback made of two linked nodes:
//!
//! * `Soundboard-Sink`   (`Audio/Sink`)   — the engine mixes its output here
//! * `Soundboard-Source` (`Audio/Source`) — "Soundboard Mic", what Discord records
//!
//! It can exist in two forms:
//!
//! * **ephemeral** — a `pw-loopback` child of this app. Simple, but it dies
//!   with the app, so Discord loses its input device until it's re-created.
//! * **persistent** — a drop-in config for the PipeWire daemon
//!   (`$XDG_CONFIG_HOME/pipewire/pipewire.conf.d/90-kakiri-soundboard.conf`)
//!   that loads `libpipewire-module-loopback`. The daemon creates the nodes
//!   itself, so they survive app exit *and* reboots, and can never be orphaned.
//!
//! *Verified*: a `context.modules` drop-in is merged with the base config (it
//! does not replace it), and the drop-in is read from the user config dir.
//!
//! Windows/macOS have no equivalent — the user installs a virtual cable
//! (VB-CABLE / BlackHole) and we detect it.

use std::path::PathBuf;

/// The `Audio/Sink` the engine plays into.
pub const SINK: &str = "Soundboard-Sink";
/// The `Audio/Source` Discord should record from.
pub const SOURCE: &str = "Soundboard-Source";
/// Human-readable name shown by Discord/`pavucontrol`.
pub const DESCRIPTION: &str = "Soundboard Mic";

/// File name of the PipeWire drop-in (also how we recognise our own file).
const DROPIN_NAME: &str = "90-kakiri-soundboard.conf";

/// The drop-in we install. Kept byte-for-byte as validated against the daemon.
pub const CONFIG: &str = r#"# Kakiri Soundboard — virtual microphone (managed by the app).
#
# Creates two linked nodes:
#   Soundboard-Sink   (Audio/Sink)   — the soundboard mixes its output here
#   Soundboard-Source (Audio/Source) — "Soundboard Mic", selected in Discord
#
# Deleting this file (Settings → Virtual microphone → Remove) undoes it.
context.modules = [
    { name = libpipewire-module-loopback
      args = {
          capture.props = {
              node.name        = "Soundboard-Sink"
              node.description = "Soundboard Sink"
              media.class      = "Audio/Sink"
              audio.position   = [ FL FR ]
          }
          playback.props = {
              node.name        = "Soundboard-Source"
              node.description = "Soundboard Mic"
              media.class      = "Audio/Source"
              audio.position   = [ FL FR ]
          }
      }
    }
]
"#;

#[derive(Debug, Clone, serde::Serialize)]
pub struct VirtualMic {
    /// A loopback (or a virtual cable, on Windows/macOS) is available.
    pub present: bool,
    /// Human-readable name of the input device, exactly as Discord/pavucontrol
    /// show it (Linux: the node *description*; Windows/macOS: the device name).
    pub source: String,
    /// The output the engine plays into — a node/device *name* used for routing.
    pub sink: String,
    /// We installed a PipeWire drop-in, so it survives reboots.
    pub persistent: bool,
    /// This build can create/manage the device itself.
    pub can_manage: bool,
}

/// Where our drop-in lives, or `None` if there's no config directory.
pub fn dropin_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("pipewire").join("pipewire.conf.d").join(DROPIN_NAME))
}

/// Whether our drop-in is installed (regardless of whether it's live yet).
pub fn persistent_installed() -> bool {
    dropin_path().map(|p| p.is_file()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Linux / PipeWire
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    fn is_present() -> Result<bool, String> {
        let nodes = crate::devices::pipewire_nodes().map_err(|e| e.to_string())?;
        Ok(nodes.iter().any(|n| n.name == SINK))
    }

    pub fn status() -> Result<VirtualMic, String> {
        Ok(VirtualMic {
            present: is_present()?,
            // Discord lists the node *description*, not the node name.
            source: DESCRIPTION.to_string(),
            sink: SINK.to_string(),
            persistent: persistent_installed(),
            can_manage: true,
        })
    }

    /// Start a `pw-loopback` child. Used when there's no drop-in yet.
    fn spawn_ephemeral() -> Result<(), String> {
        Command::new("pw-loopback")
            .args([
                format!("--capture-props=node.name={SINK} media.class=Audio/Sink"),
                format!(
                    "--playback-props=node.name={SOURCE} media.class=Audio/Source \
                     node.description=\"{DESCRIPTION}\""
                ),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start pw-loopback: {e}"))?;
        Ok(())
    }

    fn kill_ephemeral() {
        let _ = Command::new("pkill")
            .args(["-9", "-f", "pw-loopback.*Soundboard-Sink"])
            .stderr(Stdio::null())
            .status();
    }

    /// Make sure the virtual mic exists, creating an ephemeral one if needed.
    pub fn ensure() -> Result<VirtualMic, String> {
        if is_present()? {
            return status();
        }
        spawn_ephemeral()?;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            if is_present()? {
                break;
            }
        }
        status()
    }

    /// Start the loopback at launch if it's missing — without waiting for it,
    /// so app startup is never blocked.
    pub fn autostart() {
        if !is_present().unwrap_or(false) {
            let _ = spawn_ephemeral();
        }
    }

    /// Install the drop-in and bring the device up *now*.
    ///
    /// The drop-in takes effect from the next PipeWire start (login/reboot);
    /// `ensure()` covers the current session in the meantime.
    pub fn install_persistent() -> Result<VirtualMic, String> {
        let path = dropin_path().ok_or("cannot determine XDG config directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        std::fs::write(&path, CONFIG).map_err(|e| format!("write {}: {e}", path.display()))?;
        ensure()
    }

    /// Remove the drop-in and stop the ephemeral loopback.
    ///
    /// A loopback created *by the daemon* only goes away when PipeWire restarts,
    /// so the returned status may still report `present`.
    pub fn remove_persistent() -> Result<VirtualMic, String> {
        if let Some(path) = dropin_path() {
            if path.is_file() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("remove {}: {e}", path.display()))?;
            }
            // Tidy up the directory too, but only if we left it empty.
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        kill_ephemeral();
        std::thread::sleep(Duration::from_millis(300));
        status()
    }
}

// ---------------------------------------------------------------------------
// Windows / macOS — detect a user-installed virtual cable
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    fn is_virtual_cable(name: &str) -> bool {
        let n = name.to_lowercase();
        ["cable", "voicemeeter", "blackhole", "loopback audio", "soundflower", "virtual audio"]
            .iter()
            .any(|k| n.contains(k))
    }

    fn defaults() -> (&'static str, &'static str) {
        #[cfg(target_os = "windows")]
        {
            ("CABLE Output", "CABLE Input")
        }
        #[cfg(not(target_os = "windows"))]
        {
            ("BlackHole 2ch", "BlackHole 2ch")
        }
    }

    pub fn status() -> Result<VirtualMic, String> {
        let outputs = crate::devices::cpal_outputs().unwrap_or_default();
        let inputs = crate::devices::cpal_inputs().unwrap_or_default();
        let sink = outputs
            .iter()
            .find(|d| is_virtual_cable(&d.name))
            .map(|d| d.name.clone());
        let source = inputs
            .iter()
            .find(|d| is_virtual_cable(&d.name))
            .map(|d| d.name.clone());
        let (def_source, def_sink) = defaults();
        Ok(VirtualMic {
            present: sink.is_some() || source.is_some(),
            source: source.or_else(|| sink.clone()).unwrap_or_else(|| def_source.to_string()),
            sink: sink.unwrap_or_else(|| def_sink.to_string()),
            persistent: false,
            can_manage: false,
        })
    }

    pub fn ensure() -> Result<VirtualMic, String> {
        status()
    }

    pub fn autostart() {}

    pub fn install_persistent() -> Result<VirtualMic, String> {
        Err("install VB-CABLE (Windows) or BlackHole (macOS) to get a virtual microphone".into())
    }

    pub fn remove_persistent() -> Result<VirtualMic, String> {
        Err("the virtual cable is a separate driver; uninstall it the way you installed it".into())
    }
}

pub use imp::{autostart, ensure, install_persistent, remove_persistent, status};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropin_path_is_under_pipewire_conf_d() {
        let p = dropin_path().expect("home dir");
        assert!(p.ends_with(format!("pipewire/pipewire.conf.d/{DROPIN_NAME}")));
    }

    #[test]
    fn config_declares_both_nodes() {
        assert!(CONFIG.contains("libpipewire-module-loopback"));
        assert!(CONFIG.contains(&format!("node.name        = \"{SINK}\"")));
        assert!(CONFIG.contains(&format!("node.name        = \"{SOURCE}\"")));
        assert!(CONFIG.contains("media.class      = \"Audio/Sink\""));
        assert!(CONFIG.contains("media.class      = \"Audio/Source\""));
        assert!(CONFIG.contains(&format!("node.description = \"{DESCRIPTION}\"")));
        // a stray quote would make PipeWire fail to start → check balance
        assert_eq!(CONFIG.matches('"').count() % 2, 0);
    }
}
