//! Tauri backend for discord-soundboard.
//!
//! The audio engine lives in the `audio-core` crate; this file is the IPC layer
//! the frontend talks to. See docs/ARCHITECTURE.md §5 for the contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use audio_core::bus::BusFormat;
use audio_core::engine::Engine;
use audio_core::input::AudioInput;
use audio_core::library::Folder;
use audio_core::mixer::{Retrigger, Route, VoiceInfo};
use audio_core::output::AudioOutput;
use audio_core::timeline::{Clip, TimelineStatus};
use audio_core::virtualmic::{self, VirtualMic as MicStatus};

#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    /// Cover-art data-URL cache, keyed by absolute path.
    art_cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

/// The virtual-mic output. Native PipeWire by default on Linux; set
/// `SOUNDBOARD_PW_BACKEND=cli` to fall back to the `pw-play` subprocess.
fn make_output(target: String, fmt: BusFormat) -> Box<dyn AudioOutput> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("SOUNDBOARD_PW_BACKEND").as_deref() == Ok("cli") {
            Box::new(audio_core::output::PipeWireCliOutput::new(
                target,
                fmt.rate,
                fmt.channels,
            ))
        } else {
            Box::new(audio_core::output::PipeWireOutput::new(
                target,
                fmt.rate,
                fmt.channels,
            ))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(audio_core::output::CpalOutput::new(
            target,
            fmt.rate,
            fmt.channels,
        ))
    }
}

/// The live voice input. Native PipeWire by default on Linux; set
/// `SOUNDBOARD_PW_BACKEND=cli` to use the `pw-record` subprocess.
fn make_input(target: String, fmt: BusFormat) -> Box<dyn AudioInput> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("SOUNDBOARD_PW_BACKEND").as_deref() == Ok("cli") {
            Box::new(audio_core::input::PipeWireCliInput::new(
                target,
                fmt.rate,
                fmt.channels,
            ))
        } else {
            Box::new(audio_core::input::PipeWireInput::new(
                target,
                fmt.rate,
                fmt.channels,
            ))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(audio_core::input::CpalInput::new(
            target,
            fmt.rate,
            fmt.channels,
        ))
    }
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Device {
    index: usize,
    name: String,
    channels: u16,
    rate: u32,
}

#[derive(Serialize)]
struct Node {
    class: String,
    name: String,
    description: String,
    virtual_mic: bool,
}

#[derive(Serialize)]
struct Devices {
    outputs: Vec<Device>,
    inputs: Vec<Device>,
    nodes: Vec<Node>,
}

#[tauri::command]
async fn list_devices() -> Result<Devices, String> {
    // Device enumeration (ALSA/cpal + pw-dump) can take a while — keep it off
    // the main thread so the window stays responsive at startup.
    tauri::async_runtime::spawn_blocking(|| {
    let outputs = audio_core::devices::cpal_outputs()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| Device {
            index: d.index,
            name: d.name,
            channels: d.channels,
            rate: d.rate,
        })
        .collect();
    let inputs = audio_core::devices::cpal_inputs()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| Device {
            index: d.index,
            name: d.name,
            channels: d.channels,
            rate: d.rate,
        })
        .collect();
    let nodes = audio_core::devices::pipewire_nodes()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|n| Node {
            virtual_mic: n.looks_like_virtual_mic(),
            class: n.class,
            name: n.name,
            description: n.description,
        })
        .collect();
        Ok::<Devices, String>(Devices { outputs, inputs, nodes })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn virtual_mic_status() -> Result<MicStatus, String> {
    tauri::async_runtime::spawn_blocking(virtualmic::status)
        .await
        .map_err(|e| e.to_string())?
}

/// Create the virtual mic for this session (an ephemeral `pw-loopback` on Linux).
#[tauri::command]
async fn create_virtual_mic() -> Result<MicStatus, String> {
    tauri::async_runtime::spawn_blocking(virtualmic::ensure)
        .await
        .map_err(|e| e.to_string())?
}

/// Install a PipeWire drop-in so the virtual mic is created by the daemon from
/// boot on, and survives Kakiri quitting. (Windows/macOS: not supported.)
#[tauri::command]
async fn install_virtual_mic() -> Result<MicStatus, String> {
    tauri::async_runtime::spawn_blocking(virtualmic::install_persistent)
        .await
        .map_err(|e| e.to_string())?
}

/// Remove the drop-in and stop any ephemeral loopback.
#[tauri::command]
async fn remove_virtual_mic() -> Result<MicStatus, String> {
    tauri::async_runtime::spawn_blocking(virtualmic::remove_persistent)
        .await
        .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Output routing
// ---------------------------------------------------------------------------

/// Point the mix bus at a specific PipeWire node (`None` = default sink).
#[tauri::command]
fn set_output_target(state: tauri::State<'_, AppState>, target: Option<String>) -> Result<(), String> {
    let fmt = state.engine.format();
    let target = target.unwrap_or_default();
    state
        .engine
        .set_output(make_output(target, fmt))
        .map_err(|e| e.to_string())
}

/// Enable/replace the local monitor output, or disable it.
#[tauri::command]
fn set_monitor(
    state: tauri::State<'_, AppState>,
    enabled: bool,
    target: Option<String>,
) -> Result<bool, String> {
    let fmt = state.engine.format();
    let output = if enabled {
        Some(make_output(target.unwrap_or_default(), fmt))
    } else {
        None
    };
    state
        .engine
        .set_monitor(output)
        .map_err(|e| e.to_string())?;
    Ok(state.engine.monitor_enabled())
}

#[tauri::command]
fn monitor_status(state: tauri::State<'_, AppState>) -> bool {
    state.engine.monitor_enabled()
}

// ---------------------------------------------------------------------------
// Live voice input (mic passthrough) — mixes your voice into the virtual mic
// ---------------------------------------------------------------------------

/// Point the live input at a PipeWire source (`None`/empty disables it).
#[tauri::command]
fn set_input(
    state: tauri::State<'_, AppState>,
    source: Option<String>,
    gain: Option<f32>,
) -> Result<bool, String> {
    let fmt = state.engine.format();
    let input = source
        .filter(|s| !s.trim().is_empty())
        .map(|s| make_input(s, fmt));
    state.engine.set_input(input).map_err(|e| e.to_string())?;
    if let Some(g) = gain {
        state.engine.set_input_gain(g);
    }
    Ok(state.engine.input_active())
}

#[tauri::command]
fn set_input_gain(state: tauri::State<'_, AppState>, gain: f32) -> f32 {
    state.engine.set_input_gain(gain);
    gain
}

#[tauri::command]
fn input_status(state: tauri::State<'_, AppState>) -> bool {
    state.engine.input_active()
}

#[tauri::command]
fn play_test_tone(
    state: tauri::State<'_, AppState>,
    freq: Option<f32>,
    ms: Option<u32>,
    route: Option<String>,
) -> Result<u64, String> {
    let fmt = state.engine.format();
    let pcm = Arc::new(audio_core::bus::sine(
        fmt,
        freq.unwrap_or(440.0),
        ms.unwrap_or(700),
        0.25,
    ));
    let route = Route::parse(route.as_deref().unwrap_or("mic"));
    Ok(state.engine.play("test-tone", "test tone", pcm, 1.0, false, route))
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

/// Dev convenience: `<repo>/test_playlist` if it exists next to the app crate.
#[tauri::command]
fn default_roots() -> Vec<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    match Path::new(manifest).parent().map(|r| r.join("test_playlist")) {
        Some(p) if p.is_dir() => vec![p.display().to_string()],
        _ => Vec::new(),
    }
}

#[tauri::command]
async fn library_scan(roots: Vec<String>) -> Result<Vec<Folder>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
        audio_core::library::scan(&paths)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Embedded cover art for a file as a `data:` URL (cached), or `None`.
#[tauri::command]
async fn artwork(state: tauri::State<'_, AppState>, path: String) -> Result<Option<String>, String> {
    let cache = state.art_cache.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cached = { cache.lock().unwrap().get(&path).cloned() };
        if let Some(v) = cached {
            return Ok::<_, String>(v);
        }
        let url = audio_core::tags::cover_data_url(Path::new(&path));
        cache.lock().unwrap().insert(path, url.clone());
        Ok(url)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// Start a streaming voice for a file. Returns the voice id (or `None` if a
/// Toggle retrigger stopped the playing copy).
#[tauri::command]
async fn play_sound(
    state: tauri::State<'_, AppState>,
    path: String,
    gain: Option<f32>,
    looped: Option<bool>,
    route: Option<String>,
    retrigger: Option<String>,
) -> Result<Option<u64>, String> {
    let engine = state.engine.clone();
    let gain = gain.unwrap_or(0.85);
    let looped = looped.unwrap_or(false);
    let route = Route::parse(route.as_deref().unwrap_or("mic"));
    let retrigger = Retrigger::parse(retrigger.as_deref().unwrap_or("layer"));

    tauri::async_runtime::spawn_blocking(move || {
        let label = Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound")
            .to_string();
        let start = |engine: &std::sync::Arc<audio_core::engine::Engine>| {
            engine
                .play_stream(path.clone(), label.clone(), PathBuf::from(&path), gain, looped, route)
                .map(Some)
                .map_err(|e| e.to_string())
        };

        match retrigger {
            // already playing → stop it (play/stop toggle)
            Retrigger::Toggle if engine.is_key_active(&path) => {
                engine.stop_key(&path);
                Ok(None)
            }
            // restart from the top
            Retrigger::Choke => {
                engine.stop_key(&path);
                start(&engine)
            }
            // layer another copy on top (default)
            _ => start(&engine),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_voice(state: tauri::State<'_, AppState>, id: u64) {
    state.engine.stop(id);
}

/// Decode a file into the cache ahead of time.
#[tauri::command]
fn stop_all(state: tauri::State<'_, AppState>) {
    state.engine.stop_all();
}

#[tauri::command]
fn set_voice_gain(state: tauri::State<'_, AppState>, id: u64, gain: f32) {
    state.engine.set_gain(id, gain);
}

#[tauri::command]
fn seek_voice(state: tauri::State<'_, AppState>, id: u64, position: f32) {
    state.engine.seek(id, position);
}

/// Master volume: applies live to every playing voice.
#[tauri::command]
fn set_master_gain(state: tauri::State<'_, AppState>, gain: f32) {
    state.engine.set_all_gain(gain);
}

#[tauri::command]
fn active_voices(state: tauri::State<'_, AppState>) -> Vec<VoiceInfo> {
    state.engine.active()
}

// ---------------------------------------------------------------------------
// Timeline / arranger
// ---------------------------------------------------------------------------

/// Play a set of clips on the timeline. Clips overlapping in time are mixed.
#[tauri::command]
async fn timeline_play(
    state: tauri::State<'_, AppState>,
    clips: Vec<Clip>,
    looped: Option<bool>,
    from: Option<f64>,
) -> Result<TimelineStatus, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .play_timeline(clips, looped.unwrap_or(false), from.unwrap_or(0.0))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn timeline_stop(state: tauri::State<'_, AppState>) {
    state.engine.stop_timeline();
}

#[tauri::command]
fn timeline_status(state: tauri::State<'_, AppState>) -> TimelineStatus {
    state.engine.timeline_status()
}

/// Length of a clip list, so the UI can size the ruler without decoding.
#[tauri::command]
async fn timeline_length(
    state: tauri::State<'_, AppState>,
    clips: Vec<Clip>,
) -> Result<f64, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.timeline_length(&clips))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_cache(state: tauri::State<'_, AppState>) {
    state.art_cache.lock().unwrap().clear();
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Global hotkeys
// ---------------------------------------------------------------------------

/// Replace the set of registered global hotkeys. Returns, for each input, the
/// canonical accelerator string that was registered, or `""` if it failed.
#[tauri::command]
fn set_hotkeys(app: tauri::AppHandle, accelerators: Vec<String>) -> Vec<String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    accelerators
        .into_iter()
        .map(|acc| {
            let acc = acc.trim();
            if acc.is_empty() {
                return String::new();
            }
            match acc.parse::<Shortcut>() {
                Ok(sc) => {
                    let canonical = sc.to_string();
                    if gs.register(sc).is_ok() {
                        canonical
                    } else {
                        String::new()
                    }
                }
                Err(_) => String::new(),
            }
        })
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        use tauri::Emitter;
                        let _ = app.emit("hotkey", shortcut.to_string());
                    }
                })
                .build(),
        )
        .setup(|app| {
            use tauri::Manager;
            let fmt = BusFormat::default();
            // Bring the virtual mic up (the drop-in already did, if installed);
            // never block startup waiting for it.
            virtualmic::autostart();
            let target = virtualmic::status().map(|m| m.sink).unwrap_or_default();
            let engine = Engine::new(fmt, make_output(target, fmt));
            app.manage(AppState {
                engine,
                art_cache: Arc::new(Mutex::new(HashMap::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            virtual_mic_status,
            create_virtual_mic,
            install_virtual_mic,
            remove_virtual_mic,
            set_output_target,
            set_monitor,
            monitor_status,
            set_input,
            set_input_gain,
            input_status,
            play_test_tone,
            default_roots,
            library_scan,
            artwork,
            play_sound,
            stop_voice,
            stop_all,
            set_voice_gain,
            seek_voice,
            set_master_gain,
            active_voices,
            timeline_play,
            timeline_stop,
            timeline_status,
            timeline_length,
            clear_cache,
            set_hotkeys,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
