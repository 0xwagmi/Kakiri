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
pub mod online;
pub mod presence;
pub mod secrets;
pub mod update;
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
/// boot on, and survives Wagmify quitting. (Windows/macOS: not supported.)
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

/// RNNoise on the live input: the denoiser Easy Effects offers, applied to the
/// samples the engine already captured -- one 10 ms frame, no extra graph hop.
#[tauri::command]
fn set_denoise(state: tauri::State<'_, AppState>, on: bool) -> bool {
    state.engine.set_denoise(on);
    log::info!("[mic] noise reduction {}", if on { "on" } else { "off" });
    on
}

#[tauri::command]
fn denoise_status(state: tauri::State<'_, AppState>) -> bool {
    state.engine.denoise_on()
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

/// The app used to be called Kakiri: move its folders over once, so a rename does
/// not look like "all my sounds are gone".
fn migrate_old_name(app: &tauri::AppHandle) {
    use tauri::Manager;
    for (get, old) in [
        (app.path().app_local_data_dir().ok(), "com.kakiri.soundboard"),
        (app.path().app_cache_dir().ok(), "com.kakiri.soundboard"),
    ] {
        let Some(new) = get else { continue };
        let Some(parent) = new.parent() else { continue };
        let old_dir = parent.join(old);
        if old_dir.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old_dir, &new);
        }
    }

    // the library folder moved from Music/Kakiri to Music/Wagmify
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let music = std::path::PathBuf::from(home).join("Music");
        let (old, new) = (music.join("Kakiri"), music.join("Wagmify"));
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// Where imports go when the library has no folders yet: `~/Music/Wagmify`,
/// created on demand so a fresh install can still take a dropped file.
#[tauri::command]
fn import_default_folder() -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("cannot find your home folder")?;
    let dir = PathBuf::from(home).join("Music").join("Wagmify");
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

/// What `play_link` reports back: the voice it started, the title, and how long
/// resolving + buffering took (so the UI can be honest about "loading").
#[derive(serde::Serialize)]
struct LinkVoice {
    id: u64,
    title: String,
    buffered_ms: u64,
}

/// Resolutions are valid for hours, so an immediate replay (or a queue advancing
/// to a track twice) should not pay yt-dlp's ~4 s extraction again.
static LINK_CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, (String, String, std::time::Instant)>>> =
    std::sync::Mutex::new(None);

/// Resolve a page URL to a direct audio stream plus a title, without downloading.
fn resolve_link(url: &str) -> Result<(String, String), String> {
    let (bin, _version) = crate::download::ytdlp().ok_or_else(|| {
        "yt-dlp is missing — open Import once to let Wagmify fetch it".to_string()
    })?;
    let mut cmd = std::process::Command::new(bin);
    audio_core::proc::quiet(&mut cmd);
    let out = cmd
        .args([
            "-f",
            // symphonia decodes AAC/mp4 out of the box; opus/webm it does not
            "bestaudio[ext=m4a]/bestaudio",
            "--no-warnings",
            "--no-playlist",
            "--print",
            "TITLE:%(title)s",
            "-g",
            url,
        ])
        .output()
        .map_err(|e| format!("could not run yt-dlp: {e}"))?;
    if !out.status.success() {
        let raw = String::from_utf8_lossy(&out.stderr);
        return Err(crate::download::friendly_ytdlp_error(&raw));
    }
    parse_resolve_output(&String::from_utf8_lossy(&out.stdout))
}

/// `yt-dlp --print "TITLE:%(title)s" -g` writes a prefixed title line and then the
/// stream URL. The prefix matters: a title can be empty, contain newlines, or even
/// *be* a URL, so nothing about it can be recognised by shape.
pub(crate) fn parse_resolve_output(stdout: &str) -> Result<(String, String), String> {
    let direct = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .ok_or_else(|| "yt-dlp returned no stream URL".to_string())?
        .to_string();
    let title = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("TITLE:"))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("link")
        .to_string();
    Ok((direct, title))
}

/// Where streamed tracks are cached. A track that has been streamed through once
/// is marked complete and replayed from disk, so repeats cost no bandwidth.
fn track_cache_path(app: &tauri::AppHandle, key: &str) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let dir = app.path().app_cache_dir().ok()?.join("tracks");
    let _ = std::fs::create_dir_all(&dir);
    // stable, filesystem-safe name; key may be "online:Artist - Title"
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    Some(dir.join(format!("{hash:016x}.m4a")))
}

/// Keep the track cache under `limit` bytes, dropping the least recently used.
fn trim_track_cache(app: &tauri::AppHandle, limit: u64) {
    use tauri::Manager;
    let Ok(dir) = app.path().app_cache_dir().map(|d| d.join("tracks")) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() || e.file_name().to_string_lossy().ends_with(".done") {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|(t, _, _)| *t);
    let mut over = total - limit;
    for (_, len, path) in files {
        if over == 0 {
            break;
        }
        let _ = std::fs::remove_file(audio_core::http::done_marker(&path));
        let _ = std::fs::remove_file(&path);
        over = over.saturating_sub(len);
    }
}

fn link_cache_get(key: &str) -> Option<(String, String)> {
    let guard = LINK_CACHE.lock().unwrap();
    guard.as_ref().and_then(|m| {
        m.get(key)
            .filter(|(_, _, at)| at.elapsed() < std::time::Duration::from_secs(3 * 3600))
            .map(|(t, d, _)| (t.clone(), d.clone()))
    })
}

fn link_cache_put(key: &str, (title, url): (String, String)) {
    let mut guard = LINK_CACHE.lock().unwrap();
    let map = guard.get_or_insert_with(Default::default);
    if map.len() > 64 {
        map.clear();
    }
    map.insert(key.to_string(), (title, url, std::time::Instant::now()));
}

/// Play a link straight from the network: nothing is downloaded, nothing is added
/// to the library. The voice behaves like any other — stop it, seek it, mix it.
#[tauri::command]
async fn play_link(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    gain: Option<f32>,
    looped: Option<bool>,
    route: Option<String>,
) -> Result<LinkVoice, String> {
    let engine = state.engine.clone();
    let gain = gain.unwrap_or(0.85);
    let looped = looped.unwrap_or(false);
    // "both": the call hears it *and* you do, which is what a music player does
    let route = Route::parse(route.as_deref().unwrap_or("both"));

    tauri::async_runtime::spawn_blocking(move || {
        // cache hit → reuse the resolved URL and title
        let cached = {
            let guard = LINK_CACHE.lock().unwrap();
            guard.as_ref().and_then(|m| {
                m.get(&url)
                    .filter(|(_, _, at)| at.elapsed() < std::time::Duration::from_secs(3 * 3600))
                    .map(|(d, t, _)| (d.clone(), t.clone()))
            })
        };
        let (direct, title) = match cached {
            Some(v) => v,
            None => {
                let v = resolve_link(&url)?;
                let mut guard = LINK_CACHE.lock().unwrap();
                let map = guard.get_or_insert_with(Default::default);
                if map.len() > 32 {
                    map.clear();
                }
                map.insert(url.clone(), (v.0.clone(), v.1.clone(), std::time::Instant::now()));
                v
            }
        };

        let cache = track_cache_path(&app, &format!("link:{url}"));
        let (id, took) = engine
            .play_url_stream(
                format!("link:{url}"),
                title.clone(),
                direct,
                gain,
                looped,
                route,
                cache,
            )
            .map_err(|e| format!("could not play that link: {e}"))?;
        Ok(LinkVoice {
            id,
            title,
            buffered_ms: took.as_millis() as u64,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Normalise text for matching: lowercase, letters and digits only.
fn norm(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words that appear in upload titles without saying anything about the song.
const NOISE_WORDS: &[&str] = &[
    "official", "video", "audio", "lyric", "lyrics", "hd", "hq", "mv", "feat", "ft",
    "remastered", "visualizer", "explicit", "clean",
];

/// Words that mean "this is not the track the catalogue meant".
const JUNK_WORDS: &[&str] = &[
    "8d", "nightcore", "slowed", "reverb", "bassboost", "sped", "speedup", "tiktok",
    "cover", "karaoke", "instrumental", "loop", "hours", "hour", "extended",
];

fn tokens(s: &str) -> Vec<String> {
    norm(s)
        .split(' ')
        .filter(|t| t.len() > 1 && !NOISE_WORDS.contains(t))
        .map(str::to_string)
        .collect()
}

/// How much of `needle`'s words appear in `haystack`.
fn token_hit(needle: &str, haystack: &str) -> f32 {
    let want = tokens(needle);
    if want.is_empty() {
        return 0.0;
    }
    let have = tokens(haystack);
    let hits = want.iter().filter(|t| have.iter().any(|h| h.contains(*t))).count();
    hits as f32 / want.len() as f32
}

/// How much of the candidate's title is accounted for by the query. Without this,
/// "One More Time" and "One More Time (8D Audio 10 hours)" score identically.
fn precision(artist: &str, title: &str, cand_title: &str) -> f32 {
    let have = tokens(cand_title);
    if have.is_empty() {
        return 0.0;
    }
    let want: Vec<String> = tokens(title).into_iter().chain(tokens(artist)).collect();
    let explained = have.iter().filter(|h| want.iter().any(|w| h.contains(w))).count();
    explained as f32 / have.len() as f32
}

fn junk_penalty(cand_title: &str) -> f32 {
    let have = tokens(cand_title);
    let junk = have.iter().filter(|h| JUNK_WORDS.contains(&h.as_str())).count();
    (junk as f32 * 0.3).min(0.6)
}

/// Pick the search result that is actually this song.
///
/// A naive `ytsearch1` returns "song (nightcore 8D remix) [10 hours]" often
/// enough to be embarrassing, so each candidate is scored on three things: does
/// it contain the song's words, does it contain the artist's, and is its title
/// *only* those words. Length from the catalogue then breaks remaining ties.
/// `None` means nothing was close enough — better silence than the wrong track.
pub(crate) fn pick_match(
    artist: &str,
    title: &str,
    want_secs: u32,
    cands: &[(String, String, u32)],
) -> Option<String> {
    let mut best: Option<(f32, &str)> = None;
    for (id, cand_title, cand_secs) in cands {
        let mut score = 0.45 * token_hit(title, cand_title)
            + 0.25 * token_hit(artist, cand_title)
            + 0.30 * precision(artist, title, cand_title);
        score -= junk_penalty(cand_title);
        if want_secs > 0 && *cand_secs > 0 {
            let diff = cand_secs.abs_diff(want_secs) as f32;
            score -= (diff / 30.0).min(1.0) * 0.35;
        }
        if best.map(|(b, _)| score > b).unwrap_or(true) {
            best = Some((score, id));
        }
    }
    best.filter(|(score, _)| *score >= 0.45).map(|(_, id)| id.to_string())
}

/// One yt-dlp call that both searches and resolves.
///
/// `-g` prints a stream URL per entry, and `--print` emits lines before each one,
/// so a single process answers "which candidate is the song" *and* "what do I
/// stream" — halving the wall time, since each launch pays YouTube extraction.
///
/// Observed shape:
///
/// ```text
/// DUR:257
/// Kavinsky - Nightcall (Drive Original Movie Soundtrack) (Official Audio)
/// https://rr3---sn-....googlevideo.com/videoplayback?...
/// ```
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub url: String,
    /// YouTube video id — what "Save" needs to re-fetch the track as a file
    pub id: String,
    pub title: String,
    pub secs: u32,
}

pub(crate) fn parse_candidates(stdout: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut title = String::new();
    let mut id = String::new();
    let mut dur = 0u32;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(d) = line.strip_prefix("DUR:") {
            dur = d.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("ID:") {
            id = v.trim().to_string();
        } else if line.starts_with("http") {
            out.push(Candidate {
                url: line.to_string(),
                id: std::mem::take(&mut id),
                title: std::mem::take(&mut title),
                secs: dur,
            });
            dur = 0;
        } else if !line.is_empty() {
            title = line.to_string();
        }
    }
    out
}

/// The scoring view of a candidate: (what to play, its title, its length).
fn scored(cands: &[Candidate]) -> Vec<(String, String, u32)> {
    cands
        .iter()
        .map(|c| (c.url.clone(), c.title.clone(), c.secs))
        .collect()
}

/// Resolve a catalogue row to a playable stream URL.
///
/// `-g` resolves a stream URL for *every* entry of a search, and each extraction
/// costs ~2 s on top of yt-dlp's 1.2 s startup. Measured on this machine:
///
/// ```text
/// ytsearch1 + -g   5.0 s      ytsearch3 + -g   7.6 s      ytsearch5 + -g  11.5 s
/// flat search      2.7 s      + separate resolve    ~7.9 s total
/// ```
///
/// So the common case asks for exactly one candidate — and because that candidate
/// is then checked against the catalogue's length, a wrong pick is caught rather
/// than played: only a rejected match pays for the wider search.
fn resolve_online(
    artist: &str,
    title: &str,
    want_secs: u32,
) -> Result<(String, String), String> {
    let first = search_and_resolve(artist, title, 1)?;
    if let Some(url) = pick_match(artist, title, want_secs, &scored(&first)) {
        let id = first.iter().find(|c| c.url == url).map(|c| c.id.clone()).unwrap_or_default();
        return Ok((url, id));
    }
    // the top hit wasn't the song: look at a few and score them all
    let wider = search_and_resolve(artist, title, 3)?;
    let url = pick_match(artist, title, want_secs, &scored(&wider))
        .ok_or_else(|| format!("no close match on YouTube for “{artist} — {title}”"))?;
    let id = wider.iter().find(|c| c.url == url).map(|c| c.id.clone()).unwrap_or_default();
    Ok((url, id))
}

/// One yt-dlp launch: search `n` candidates and print each with its length and
/// stream URL. The prints come out interleaved per entry (see `parse_candidates`).
fn search_and_resolve(
    artist: &str,
    title: &str,
    n: u32,
) -> Result<Vec<Candidate>, String> {
    let (bin, _) = crate::download::ytdlp().ok_or_else(|| {
        "yt-dlp is missing — open Import once to let Wagmify fetch it".to_string()
    })?;
    let query = format!("ytsearch{n}:{artist} {title}");
    let mut cmd = std::process::Command::new(bin);
    audio_core::proc::quiet(&mut cmd);
    let out = cmd
        .args([
            "-f",
            "bestaudio[ext=m4a]/bestaudio",
            "--no-warnings",
            "--no-playlist",
            "--print",
            "DUR:%(duration)s",
            "--print",
            "ID:%(id)s",
            "--print",
            "%(title)s",
            "-g",
            &query,
        ])
        .output()
        .map_err(|e| format!("could not run yt-dlp: {e}"))?;
    if !out.status.success() {
        return Err(crate::download::friendly_ytdlp_error(&String::from_utf8_lossy(
            &out.stderr,
        )));
    }
    Ok(parse_candidates(&String::from_utf8_lossy(&out.stdout)))
}

/// Play one row from the Online list.
#[tauri::command]
async fn play_online(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    artist: String,
    title: String,
    duration: Option<u32>,
    gain: Option<f32>,
) -> Result<LinkVoice, String> {
    let engine = state.engine.clone();
    let gain = gain.unwrap_or(0.85);
    let cache = track_cache_path(&app, &format!("online:{artist} - {title}"));
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let key = format!("online:{artist} - {title}");
        let label = format!("{artist} — {title}");

        // On disk already? Play the file instead of resolving a URL: asking
        // yt-dlp first cost ~5 seconds of "buffering" over audio we already had.
        // A saved file is the strongest case — it is not going to be evicted.
        let local = saved_file(&artist, &title)
            .or_else(|| cache.clone().filter(|p| audio_core::http::is_complete(p)));
        if let Some(path) = local {
            let id = engine.play_stream_from(
                key.clone(),
                label.clone(),
                path.clone(),
                gain,
                false,
                Route::Both,
                0.0,
            )
            .map_err(|e| format!("could not play the cached track: {e}"))?;
            return Ok(LinkVoice { id, title: label, buffered_ms: 0 });
        }

        if let Some((_t, url)) = link_cache_get(&key) {
            let (id, took) = engine
                .play_url_stream(
                    key,
                    format!("{artist} — {title}"),
                    url,
                    gain,
                    false,
                    Route::Both,
                    cache.clone(),
                )
                .map_err(|e| format!("could not play that track: {e}"))?;
            return Ok(LinkVoice { id, title: format!("{artist} — {title}"), buffered_ms: took.as_millis() as u64 });
        }
        let (url, video_id) = resolve_online(&artist, &title, duration.unwrap_or(0))?;
        link_cache_put(&key, (video_id, url.clone()));
        let (id, took) = engine
            .play_url_stream(
                key,
                format!("{artist} — {title}"),
                url,
                gain,
                false,
                Route::Both,
                cache,
            )
            .map_err(|e| format!("could not play that track: {e}"))?;
        trim_track_cache(&handle, 1_500 * 1024 * 1024);
        Ok(LinkVoice { id, title: format!("{artist} — {title}"), buffered_ms: took.as_millis() as u64 })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Resolve a catalogue row to a video page URL, for "Save" — the import pipeline
/// then downloads it once into the library, after which it plays from disk and
/// can be dropped into the timeline or bound to a pad.
#[tauri::command]
async fn online_video_url(
    artist: String,
    title: String,
    duration: Option<u32>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = format!("online:{artist} - {title}");
        if let Some((id, _url)) = link_cache_get(&key) {
            if !id.is_empty() {
                return Ok(format!("https://www.youtube.com/watch?v={id}"));
            }
        }
        let (url, id) = resolve_online(&artist, &title, duration.unwrap_or(0))?;
        link_cache_put(&key, (id.clone(), url));
        if id.is_empty() {
            return Err("could not identify that video".to_string());
        }
        Ok(format!("https://www.youtube.com/watch?v={id}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Saved tracks keep their identity beside the audio in a tiny JSON file.
///
/// The file name cannot be trusted: the import pipeline writes `Title - Artist`
/// while the UI shows `Artist — Title`, so parsing it back swapped the two. The
/// sidecar holds the truth (plus the cover, so the library has artwork without
/// re-muxing the stream).
fn meta_path(audio: &std::path::Path) -> std::path::PathBuf {
    let mut p = audio.as_os_str().to_owned();
    p.push(".meta.json");
    std::path::PathBuf::from(p)
}

#[derive(serde::Deserialize)]
struct SavedMeta {
    artist: String,
    title: String,
    artwork: Option<String>,
}

fn write_meta(audio: &std::path::Path, artist: &str, title: &str, artwork: Option<&str>) {
    let meta = serde_json::json!({
        "artist": artist,
        "title": title,
        "artwork": artwork.filter(|u| u.starts_with("http")),
    });
    let _ = std::fs::write(meta_path(audio), meta.to_string());
}

/// Names that survive every filesystem we ship on.
fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// A track the user saved: a real file in the Online folder, which outlives the
/// LRU cache. Playing it never needs the network.
fn saved_file(artist: &str, title: &str) -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(import_default_folder().ok()?).join("Online");
    let name = safe_name(&format!("{artist} - {title}"));
    let path = dir.join(format!("{name}.m4a"));
    path.is_file().then_some(path)
}

/// Put an already-cached track into the library: no network, just a copy.
fn copy_cached(cache: &std::path::Path, artist: &str, title: &str) -> Result<audio_core::import::Imported, String> {
    let base = std::path::PathBuf::from(import_default_folder()?);
    let dest_dir = base.join("Online");
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("cannot create {}: {e}", dest_dir.display()))?;
    let name = format!("{} - {}", artist, title);
    let dest = dest_dir.join(format!("{}.m4a", safe_name(&name)));
    std::fs::copy(cache, &dest).map_err(|e| format!("could not copy the cached track: {e}"))?;
    let duration_secs = audio_core::tags::duration_secs(&dest).unwrap_or(0.0);
    Ok(audio_core::import::Imported {
        path: dest.display().to_string(),
        name,
        duration_secs,
        converted: false,
    })
}

#[derive(serde::Serialize)]
struct SavedTrack {
    path: String,
    name: String,
    artist: String,
    title: String,
    duration: u32,
    artwork: Option<String>,
    /// false for files saved before the sidecar existed: artist/title came from
    /// the name, whose order cannot be trusted
    has_meta: bool,
}

/// The Saved playlist, read from disk rather than remembered by the UI.
///
/// Local storage drifted from reality the moment a file was deleted or added by
/// other means, which left rows marked "saved" with nothing behind them. The
/// folder is the truth: `Artist - Title.m4a` is the whole schema.
#[tauri::command]
async fn online_saved() -> Result<Vec<SavedTrack>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let dir = std::path::PathBuf::from(import_default_folder()?).join("Online");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new()); // no saves yet
        };
        let mut out: Vec<SavedTrack> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let file_name = path.file_name()?.to_str()?.to_string();
                // yt-dlp leaves `.wagmify-….m4a` and `*.part` files mid-download
                if file_name.starts_with('.') || file_name.ends_with(".part") {
                    return None;
                }
                let ext = path.extension()?.to_str()?.to_ascii_lowercase();
                if !matches!(ext.as_str(), "m4a" | "mp3" | "opus" | "wav" | "flac" | "aac" | "ogg")
                {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let meta: Option<SavedMeta> = std::fs::read_to_string(meta_path(&path))
                    .ok()
                    .and_then(|raw| serde_json::from_str(&raw).ok());
                let has_meta = meta.is_some();
                // fallback for files saved before the sidecar existed: the name is all we
                // have, and its order is not trustworthy
                let (artist, title, artwork) = match meta {
                    Some(m) => (m.artist, m.title, m.artwork.filter(|u| u.starts_with("http"))),
                    None => match name.split_once(" - ") {
                        Some((a, t)) => (a.trim().to_string(), t.trim().to_string(), None),
                        None => (String::new(), name.clone(), None),
                    },
                };
                let duration = audio_core::tags::duration_secs(&path).unwrap_or(0.0);
                Some(SavedTrack {
                    path: path.display().to_string(),
                    name,
                    artist,
                    title,
                    duration: duration.round() as u32,
                    artwork,
                    has_meta,
                })
            })
            .collect();
        // newest first is impossible without a stored time; name order is stable
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Look a saved track's cover up by name, for files saved before we started
/// writing the cover beside them. One catalogue search, no audio.
#[derive(serde::Serialize)]
struct Lookup {
    artist: String,
    title: String,
    artwork: Option<String>,
}

/// Best catalogue guess for a saved file: repairs saves that predate the metadata
/// sidecar (their artist/title came from a file name that puts the title first)
/// and gives them a cover.
#[tauri::command]
async fn online_lookup(artist: String, title: String) -> Result<Option<Lookup>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let q = format!("{artist} {title}");
        Ok(crate::online::search(&q, 1)?.into_iter().next().map(|t| Lookup {
            artist: t.artist,
            title: t.title,
            artwork: t.artwork,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Show (or stop showing) what Wagmify is doing in the Discord member list.
/// Passing nothing clears the presence; the toggle in Settings decides whether
/// the frontend ever calls this with a value.
#[tauri::command]
fn presence_set(
    details: Option<String>,
    state: Option<String>,
    started: Option<i64>,
    ends: Option<i64>,
    image: Option<String>,
) -> bool {
    crate::presence::show(details.map(|details| crate::presence::Now {
        details,
        state: state.unwrap_or_default(),
        started,
        ends,
        image: image.filter(|i| !i.is_empty()),
    }));
    crate::presence::connected()
}

/// Is there a newer release? `None` covers both "you are current" and "could not
/// ask", because neither should interrupt anyone.
#[tauri::command]
async fn update_check() -> Option<crate::update::Update> {
    tauri::async_runtime::spawn_blocking(crate::update::check).await.ok().flatten()
}

/// Whether the Discord socket is up, so Settings can say why nothing shows yet.
/// Connecting happens on a worker thread, so this is polled rather than awaited.
#[tauri::command]
fn presence_connected() -> bool {
    crate::presence::connected()
}

/// Last.fm credentials live in a private file, not in `sb.prefs`: `localStorage`
/// is a plaintext key-value store shared with every other setting.
#[tauri::command]
fn secret_get(app: tauri::AppHandle, name: String) -> Option<String> {
    crate::secrets::get(&app, &name)
}

#[tauri::command]
fn secret_set(app: tauri::AppHandle, name: String, value: String) -> Result<(), String> {
    crate::secrets::set(&app, &name, &value)
}

/// Remove a saved track from the library.
///
/// Deliberately narrow: only files inside `~/Music/Wagmify/Online` can be deleted,
/// so a stale UI entry can never take something else with it. The track cache is
/// separate and is left alone — it is evicted on its own schedule.
#[tauri::command]
fn unsave_online(path: String) -> Result<(), String> {
    let base = std::path::PathBuf::from(import_default_folder()?).join("Online");
    remove_saved(&base, &path)
}

/// Delete `path`, but only if it really sits inside `base` (the Online folder).
/// Missing files are success: the UI just needs to forget them.
fn remove_saved(base: &std::path::Path, path: &str) -> Result<(), String> {
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let target = std::path::PathBuf::from(path);
    let resolved = target.canonicalize().unwrap_or_else(|_| target.clone());
    if !resolved.starts_with(&base) {
        return Err(format!(
            "refusing to delete {} — it is not in the Online library folder",
            target.display()
        ));
    }
    let _ = std::fs::remove_file(meta_path(&resolved));
    match std::fs::remove_file(&resolved) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not remove {}: {e}", resolved.display())),
    }
}

#[derive(serde::Serialize)]
struct CacheState {
    /// bytes already on disk
    cached: u64,
    /// size of the whole track (0 when nothing is cached yet)
    total: u64,
    /// streamed through once → playing and saving are instant
    complete: bool,
}

/// How much of this row is cached, for the Save control's fill.
#[tauri::command]
fn online_cache_state(app: tauri::AppHandle, artist: String, title: String) -> CacheState {
    let Some(path) = track_cache_path(&app, &format!("online:{artist} - {title}")) else {
        return CacheState { cached: 0, total: 0, complete: false };
    };
    let (cached, total) = audio_core::http::cache_progress(&path);
    CacheState {
        cached,
        total,
        complete: audio_core::http::is_complete(&path),
    }
}

/// Save an Online row as a real file in the library, next to imported sounds.
///
/// This is the "keep it" half of the pair: the track cache makes repeats cheap
/// until it is evicted, a saved track is a file the user owns — instant to play,
/// works offline, and can go in the timeline or on a pad like anything else.
#[tauri::command]
async fn save_online(
    app: tauri::AppHandle,
    artist: String,
    title: String,
    duration: Option<u32>,
    artwork: Option<String>,
    job: Option<String>,
) -> Result<audio_core::import::Imported, String> {
    let job = job.unwrap_or_else(|| format!("save-{artist}-{title}"));
    let app2 = app.clone();
    let job2 = job.clone();

    // saved before? hand back the file we have rather than writing it twice
    if let Some(existing) = saved_file(&artist, &title) {
        write_meta(&existing, &artist, &title, artwork.as_deref());
        let name = existing.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return Ok(audio_core::import::Imported {
            duration_secs: audio_core::tags::duration_secs(&existing).unwrap_or(0.0),
            path: existing.display().to_string(),
            name,
            converted: false,
        });
    }

    // already streamed through once? then "saving" is a local copy, not a download
    if let Some(cache) = track_cache_path(&app, &format!("online:{artist} - {title}")) {
        if audio_core::http::is_complete(&cache) {
            return tauri::async_runtime::spawn_blocking(move || {
                let got = copy_cached(&cache, &artist, &title)?;
                write_meta(std::path::Path::new(&got.path), &artist, &title, artwork.as_deref());
                Ok(got)
            })
            .await
            .map_err(|e| e.to_string())?;
        }
    }

    tauri::async_runtime::spawn_blocking(move || {
        // the same resolution the player uses, so "Save" saves what you hear
        let key = format!("online:{artist} - {title}");
        let watch = match link_cache_get(&key) {
            Some((id, _url)) if !id.is_empty() => format!("https://www.youtube.com/watch?v={id}"),
            _ => {
                let (url, id) = resolve_online(&artist, &title, duration.unwrap_or(0))?;
                link_cache_put(&key, (id.clone(), url));
                format!("https://www.youtube.com/watch?v={id}")
            }
        };
        let base = std::path::PathBuf::from(import_default_folder()?);
        let dest = base.join("Online");
        std::fs::create_dir_all(&dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
        let label = format!("{artist} — {title}");
        let got = crate::download::fetch(&job2, &watch, &dest, None, &mut |pct, stage| {
            emit_progress(&app2, &job2, pct, &format!("{label}: {stage}"));
        })?;
        write_meta(std::path::Path::new(&got.path), &artist, &title, artwork.as_deref());
        Ok(got)
    })
    .await
    .map_err(|e| e.to_string())?
}

// --- Online mode: what to play, never the audio itself ---

#[tauri::command]
async fn online_charts(limit: Option<u32>) -> Result<Vec<crate::online::OnlineTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::online::charts(limit.unwrap_or(30)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn online_search(
    query: String,
    limit: Option<u32>,
) -> Result<Vec<crate::online::OnlineTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::online::search(&query, limit.unwrap_or(30))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn online_loved(
    user: String,
    api_key: String,
) -> Result<Vec<crate::online::OnlineTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::online::loved(&user, &api_key))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn online_tag(
    tag: String,
    api_key: String,
) -> Result<Vec<crate::online::OnlineTrack>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::online::tag_station(&tag, &api_key))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod link_tests {
    use super::{
        meta_path, parse_candidates, parse_resolve_output, pick_match, remove_saved, scored,
        write_meta, SavedMeta,
    };

    #[test]
    fn pulls_the_last_url_and_the_prefixed_title() {
        let out = "TITLE:Rick Astley - Never Gonna Give You Up\nhttps://rr1---sn-abc.googlevideo.com/videoplayback?x=1\n";
        let (url, title) = parse_resolve_output(out).unwrap();
        assert!(url.starts_with("https://rr1---sn-abc"));
        assert_eq!(title, "Rick Astley - Never Gonna Give You Up");
    }

    #[test]
    fn survives_a_title_that_looks_like_a_url() {
        let out = "TITLE:https://youtu.be/xyz\n\nhttps://cdn.example/stream.m4a\n";
        let (url, title) = parse_resolve_output(out).unwrap();
        assert_eq!(url, "https://cdn.example/stream.m4a");
        assert_eq!(title, "https://youtu.be/xyz");
    }

    #[test]
    fn empty_title_falls_back_to_link() {
        let out = "TITLE:\nhttps://cdn.example/stream.m4a\n";
        let (url, title) = parse_resolve_output(out).unwrap();
        assert_eq!(url, "https://cdn.example/stream.m4a");
        assert_eq!(title, "link");
    }

    #[test]
    fn picks_the_real_song_not_the_remix() {
        let cands = vec![
            ("aaa".to_string(), "Daft Punk - One More Time (8D Audio)".to_string(), 320),
            ("bbb".to_string(), "Daft Punk - One More Time".to_string(), 320),
            ("ccc".to_string(), "One More Time - Daft Punk (10 hours)".to_string(), 36_000),
        ];
        assert_eq!(pick_match("Daft Punk", "One More Time", 320, &cands).unwrap(), "bbb");
    }

    #[test]
    fn rejects_ten_hour_loops_and_covers() {
        let cands = vec![
            ("hours".to_string(), "Daft Punk - One More Time (10 hours)".to_string(), 36_000),
            ("karaoke".to_string(), "One More Time (Karaoke Version)".to_string(), 320),
        ];
        assert!(pick_match("Daft Punk", "One More Time", 320, &cands).is_none());
    }

    #[test]
    fn rejects_a_result_that_is_not_the_song() {
        let cands = vec![("zzz".to_string(), "Some Cooking Video".to_string(), 600)];
        assert!(pick_match("Daft Punk", "One More Time", 320, &cands).is_none());
    }

    #[test]
    fn duration_breaks_ties_between_duplicates() {
        // same title twice: the one matching the catalogue length wins
        let cands = vec![
            ("short".to_string(), "Kavinsky Nightcall".to_string(), 200),
            ("real".to_string(), "Kavinsky Nightcall".to_string(), 258),
        ];
        assert_eq!(pick_match("Kavinsky", "Nightcall", 258, &cands).unwrap(), "real");
    }

    #[test]
    fn parses_urls_with_their_ids_titles_and_durations() {
        let out = "DUR:257\nID:abc123\nKavinsky - Nightcall (Official Audio)\nhttps://cdn/one\n\
                   DUR:255\nID:def456\nKavinsky - Nightcall \"Drive\"\nhttps://cdn/two\n";
        let cands = parse_candidates(out);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].url, "https://cdn/one");
        assert_eq!(cands[0].id, "abc123");
        assert_eq!(cands[0].secs, 257);
        assert!(cands[1].title.contains("Drive"));
    }

    #[test]
    fn single_call_result_still_matches_the_right_track() {
        let cands = parse_candidates(
            "DUR:3600\nID:hours\nKavinsky Nightcall 10 hours\nhttps://cdn/hours\n\
             DUR:258\nID:real\nKavinsky - Nightcall (Official Audio)\nhttps://cdn/real\n",
        );
        assert_eq!(
            pick_match("Kavinsky", "Nightcall", 258, &scored(&cands)).unwrap(),
            "https://cdn/real"
        );
    }

    #[test]
    fn saved_metadata_survives_a_round_trip() {
        let dir = std::env::temp_dir().join("wagmify-meta-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // the import pipeline names files "Title - Artist"; the sidecar must win
        let audio = dir.join("Some Title - Some Artist.m4a");
        std::fs::write(&audio, b"x").unwrap();
        write_meta(&audio, "Some Artist", "Some Title", Some("https://cdn/cover.jpg"));

        let raw = std::fs::read_to_string(meta_path(&audio)).unwrap();
        let meta: SavedMeta = serde_json::from_str(&raw).unwrap();
        assert_eq!(meta.artist, "Some Artist");
        assert_eq!(meta.title, "Some Title");
        assert_eq!(meta.artwork.as_deref(), Some("https://cdn/cover.jpg"));
    }

    #[test]
    fn unsave_refuses_anything_outside_the_online_folder() {
        let dir = std::env::temp_dir().join("wagmify-unsave-test");
        let inside = dir.join("Online");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&inside).unwrap();
        let keep = dir.join("keep.txt");
        std::fs::write(&keep, b"do not delete me").unwrap();
        let mine = inside.join("Artist - Title.m4a");
        std::fs::write(&mine, b"track").unwrap();

        assert!(remove_saved(&inside, &mine.display().to_string()).is_ok());
        assert!(!mine.exists(), "the saved file should be gone");

        let err = remove_saved(&inside, &keep.display().to_string());
        assert!(err.is_err(), "must not delete outside the Online folder");
        assert!(keep.exists(), "that file had to survive");

        // already gone → success, so a stale UI entry can still be dropped
        assert!(remove_saved(&inside, &mine.display().to_string()).is_ok());
    }

    #[test]
    fn missing_url_is_an_error() {
        assert!(parse_resolve_output("only a title\n").is_err());
    }
}

/// Pause/resume a stream voice. Position and buffered audio are kept, so this is
/// a real pause rather than a stop that loses its place.
#[tauri::command]
fn pause_voice(state: tauri::State<'_, AppState>, id: u64, paused: bool) {
    state.engine.set_paused(id, paused);
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

/// Hide instead of quitting: a soundboard is useful while its window is closed,
/// and Quit lives in the tray menu so the app can never become unreachable.
fn hide_on_close(window: &tauri::WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });
    Ok(())
}

fn toggle_main(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::Manager;
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "Show Wagmify", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Wagmify", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &sep, &quit])?;

    TrayIconBuilder::with_id("wagmify-tray")
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray-white.png"
        ))?)
        // macOS renders "template" icons in the menu bar's own colour
        .icon_as_template(true)
        .tooltip("Wagmify")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => toggle_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        })
        .build(app)?;

    if let Some(window) = app.get_webview_window("main") {
        hide_on_close(&window)?;
    }
    log::info!("tray icon ready; closing the window keeps Wagmify running");
    Ok(())
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
            // This file is for Wagmify's own diagnostics, not the decoder's
            // internals: symphonia emits a warning per file it probes, which
            // buried everything else. Third-party crates log errors only.
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Error)
                .level_for("audio_core", log::LevelFilter::Info)
                .level_for("wagmify_lib", log::LevelFilter::Info)
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
            migrate_old_name(app.handle());
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
                "Wagmify {} starting ({backend} backend, bus {} Hz / {} ch)",
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
            // Tray: the app keeps running when the window is closed, and this is
            // the only way back to it (or out of it).
            setup_tray(app).map_err(|e| e.to_string())?;

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
            set_denoise,
            denoise_status,
            input_status,
            play_test_tone,
            default_roots,
            library_scan,
            artwork,
            play_sound,
            play_link,
            play_online,
            online_video_url,
            save_online,
            unsave_online,
            update_check,
            presence_set,
            presence_connected,
            secret_get,
            secret_set,
            online_saved,
            online_lookup,
            online_cache_state,
            online_charts,
            online_search,
            online_loved,
            online_tag,
            pause_voice,
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
