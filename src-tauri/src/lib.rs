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
pub mod download;
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

/// Where imports go when the library has no folders yet: `~/Music/Kakiri`,
/// created on demand so a fresh install can still take a dropped file.
#[tauri::command]
fn import_default_folder() -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("cannot find your home folder")?;
    let dir = PathBuf::from(home).join("Music").join("Kakiri");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir.display().to_string())
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

/// Master volume: applied at render time, so it also affects sounds that start
/// later (timeline clips, pads).
#[tauri::command]
fn set_master_gain(state: tauri::State<'_, AppState>, gain: f32) -> f32 {
    state.engine.set_master(gain);
    state.engine.master()
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

/// Drop a clip into the timeline that is already playing — no restart, and the
/// sounds that are already sounding keep going.
#[tauri::command]
async fn timeline_add_clip(
    state: tauri::State<'_, AppState>,
    clip: Clip,
) -> Result<TimelineStatus, String> {
    let engine = state.engine.clone();
    // Probing the file duration is I/O; keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || engine.timeline_add_clip(clip))
        .await
        .map_err(|e| e.to_string())?
}

/// Take one clip out of the running timeline, silencing only that clip.
#[tauri::command]
fn timeline_remove_clip(state: tauri::State<'_, AppState>, id: String) -> TimelineStatus {
    state.engine.timeline_remove_clip(&id)
}

/// Loop on/off while playing, without dropping the playhead.
#[tauri::command]
fn timeline_set_loop(state: tauri::State<'_, AppState>, looped: bool) {
    state.engine.timeline_set_loop(looped);
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

/// What the import UI can offer (ffmpeg is only needed for video files).
#[tauri::command]
fn import_tools() -> download::Tools {
    download::tools()
}

/// Stop whatever a job is doing (a stalled downloader, a huge video).
#[tauri::command]
fn import_cancel(job: String) -> bool {
    download::cancel(&job)
}

/// Progress for one import job, mirrored to a bar in the UI.
#[derive(Clone, serde::Serialize)]
struct ImportProgress {
    job: String,
    /// 0..1, negative while the total size is unknown.
    pct: f32,
    stage: String,
}

fn emit_progress(app: &tauri::AppHandle, job: &str, pct: f32, stage: &str) {
    use tauri::Emitter;
    let _ = app.emit(
        "import-progress",
        ImportProgress {
            job: job.to_string(),
            pct,
            stage: stage.to_string(),
        },
    );
}

/// Import a file the user dropped or picked: audio is copied, video has its
/// audio track extracted (ffmpeg).
#[tauri::command]
async fn import_file(
    app: tauri::AppHandle,
    job: String,
    src: String,
    dest: String,
) -> Result<audio_core::import::Imported, String> {
    let dest_dir = download::ensure_dest(&dest)?;
    let src_path = std::path::PathBuf::from(&src);
    let app2 = app.clone();
    let job2 = job.clone();
    tauri::async_runtime::spawn_blocking(move || {
        audio_core::import::import_file(&src_path, &dest_dir, |pct, stage| {
            emit_progress(&app2, &job2, pct, stage);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Re-encode a file that is already in the library but that the mixer can't
/// decode (HE-AAC and friends), in place, and return what it became.
#[tauri::command]
async fn convert_sound(
    app: tauri::AppHandle,
    job: String,
    path: String,
) -> Result<audio_core::import::Imported, String> {
    let src = std::path::PathBuf::from(&path);
    let app2 = app.clone();
    let job2 = job.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (target, converted) =
            audio_core::import::ensure_playable(&src, &mut |pct, stage| {
                emit_progress(&app2, &job2, pct, stage);
            })?;
        Ok::<_, String>(audio_core::import::Imported {
            name: target
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("sound")
                .to_string(),
            duration_secs: audio_core::import::duration_of(&target),
            path: target.to_string_lossy().to_string(),
            converted,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Download a link into a library folder. Direct audio/video links only; page
/// URLs (YouTube etc.) will come from yt-dlp.
#[tauri::command]
async fn import_url(
    app: tauri::AppHandle,
    job: String,
    url: String,
    dest: String,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<audio_core::import::Imported, String> {
    let dest_dir = download::ensure_dest(&dest)?;
    let range = match (start, end) {
        (Some(a), Some(b)) if b > a => Some((a.max(0.0), b)),
        _ => None,
    };
    let app2 = app.clone();
    let job2 = job.clone();
    tauri::async_runtime::spawn_blocking(move || {
        download::fetch(&job2, &url, &dest_dir, range, &mut |pct, stage| {
            emit_progress(&app2, &job2, pct, stage);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Metadata for a link before downloading it (title + duration), so the panel
/// can show what it found and offer a clip range.
#[tauri::command]
async fn import_probe(url: String) -> Result<download::PageInfo, String> {
    tauri::async_runtime::spawn_blocking(move || download::probe_page(&url))
        .await
        .map_err(|e| e.to_string())?
}

/// Install (or refresh) the downloader used for page links.
///
/// Checks GitHub first, so asking for an update when we are already current
/// reports that instead of moving 40 MB again.
#[derive(serde::Serialize)]
struct ToolUpdate {
    version: String,
    /// False when nothing was downloaded because we were already current.
    updated: bool,
}

#[tauri::command]
async fn import_ensure_ytdlp(
    app: tauri::AppHandle,
    job: String,
) -> Result<ToolUpdate, String> {
    let app2 = app.clone();
    let job2 = job.clone();
    tauri::async_runtime::spawn_blocking(move || {
        download::ensure_ytdlp(&job2, &mut |pct, stage| {
            emit_progress(&app2, &job2, pct, stage);
        })
        .map(|(version, updated)| ToolUpdate { version, updated })
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Send panics to the log as well, not just to a stderr nobody reads.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        // First plugin, as the single-instance plugin requires: a second launch
        // must not create a second engine writing to the same virtual sink.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // Everything audio-core logs (device errors, timeline failures) plus
        // panics lands in <app data>/logs, so a user can send one file.
        .plugin(
            // This file is for Kakiri's own diagnostics, not the decoder's
            // internals: symphonia emits a warning per file it probes, which
            // buried everything else. Third-party crates log errors only.
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Error)
                .level_for("audio_core", log::LevelFilter::Info)
                .level_for("soundboard_lib", log::LevelFilter::Info)
                // the builder already has default targets; replace them so each
                // record is written once instead of twice
                .clear_targets()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .build(),
        )
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
            // Where a downloader fetched by the import panel lives.
            if let Ok(dir) = app.path().app_local_data_dir() {
                download::set_tools_dir(dir.join("tools"));
            }
            let fmt = BusFormat::default();
            // Bring the virtual mic up (the drop-in already did, if installed);
            // never block startup waiting for it.
            virtualmic::autostart();
            let target = virtualmic::status().map(|m| m.sink).unwrap_or_default();
            // A log that only ever contains errors is useless in a bug report:
            // record what the app started with.
            let backend = if cfg!(target_os = "linux") { "pipewire" } else { "cpal" };
            log::info!(
                "Kakiri {} starting ({backend} backend, bus {} Hz / {} ch)",
                env!("CARGO_PKG_VERSION"),
                fmt.rate,
                fmt.channels
            );
            match virtualmic::status() {
                Ok(mic) => log::info!(
                    "virtual mic present={} persistent={} sink={} source={}",
                    mic.present,
                    mic.persistent,
                    mic.sink,
                    mic.source
                ),
                Err(e) => log::warn!("virtual mic status unavailable: {e}"),
            }
            log::info!("output target: {}", if target.is_empty() { "default" } else { &target });
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
            timeline_add_clip,
            timeline_remove_clip,
            timeline_set_loop,
            import_tools,
            import_default_folder,
            import_probe,
            import_ensure_ytdlp,
            import_cancel,
            convert_sound,
            import_file,
            import_url,
            clear_cache,
            set_hotkeys,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
