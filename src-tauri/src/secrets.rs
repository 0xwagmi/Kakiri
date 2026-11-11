//! Where the Last.fm key lives.
//!
//! A credential we have to *send* cannot be hashed — hashing is one-way, and the
//! API needs the original. So the choice is between a full OS keyring integration
//! and something small: this is the small version. The value is XOR-obfuscated
//! with a fixed app salt and base64'd, in `secrets.json` inside the app data
//! directory, created `0600` on Unix.
//!
//! That keeps it out of sight in `sb.prefs` and out of reach of other users on the
//! machine. It is **obfuscation, not encryption**: anyone who can read the file as
//! you can decode it. If that matters, move to the OS keyring later — the calls
//! below are the whole surface, so swapping the backing store is contained.

use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;

use tauri::Manager;

const SALT: &[u8] = b"wagmify-soundboard-v1";

fn xform(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ SALT[i % SALT.len()])
        .collect()
}

fn path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("secrets.json"))
}

fn read(app: &tauri::AppHandle) -> std::collections::BTreeMap<String, String> {
    path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write(app: &tauri::AppHandle, map: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let path = path(app).ok_or("no app data directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string(map).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn set(app: &tauri::AppHandle, name: &str, value: &str) -> Result<(), String> {
    let mut map = read(app);
    if value.is_empty() {
        map.remove(name);
    } else {
        map.insert(name.to_string(), STANDARD.encode(xform(value.as_bytes())));
    }
    write(app, &map)
}

pub fn get(app: &tauri::AppHandle, name: &str) -> Option<String> {
    let raw = read(app).get(name)?.clone();
    let bytes = STANDARD.decode(raw).ok()?;
    String::from_utf8(xform(&bytes)).ok()
}
