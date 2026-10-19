//! Library scanning: walk folders, find playable audio files, read durations.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::tags;

pub const AUDIO_EXTS: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "aiff", "aif", "mp4", "mka", "wv",
];

#[derive(Debug, Clone, Serialize)]
pub struct Sound {
    /// Stable id: the absolute path.
    pub id: String,
    pub path: String,
    /// Display name (filename stem).
    pub name: String,
    /// Embedded title tag, if any.
    pub title: Option<String>,
    pub artist: Option<String>,
    pub ext: String,
    pub duration_secs: Option<f32>,
    /// Whether the file has embedded cover art (fetch lazily via `artwork`).
    pub has_cover: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub name: String,
    pub path: String,
    /// Direct children folders.
    pub children: Vec<Folder>,
    /// Sounds directly in this folder.
    pub sounds: Vec<Sound>,
    /// Total sounds including descendants (convenience for the UI).
    pub total_sounds: usize,
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

fn scan_dir(dir: &Path) -> Folder {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| dir.display().to_string());

    let mut sounds = Vec::new();
    let mut children = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if is_hidden(&path) {
                continue;
            }
            if path.is_dir() {
                let child = scan_dir(&path);
                if child.total_sounds > 0 {
                    children.push(child);
                }
            } else if is_audio(&path) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let info = tags::probe(&path);
                sounds.push(Sound {
                    id: path.display().to_string(),
                    path: path.display().to_string(),
                    name: stem,
                    title: info.as_ref().and_then(|i| i.title.clone()),
                    artist: info.as_ref().and_then(|i| i.artist.clone()),
                    ext,
                    duration_secs: info.as_ref().and_then(|i| i.duration_secs),
                    has_cover: info.as_ref().map(|i| i.has_cover).unwrap_or(false),
                });
            }
        }
    }

    let total_sounds = sounds.len() + children.iter().map(|c| c.total_sounds).sum::<usize>();
    Folder {
        name,
        path: dir.display().to_string(),
        children,
        sounds,
        total_sounds,
    }
}

/// Scan each root into a folder tree (empty/childless roots are dropped).
pub fn scan(roots: &[PathBuf]) -> Vec<Folder> {
    roots
        .iter()
        .filter(|r| r.is_dir())
        .map(|r| scan_dir(r))
        .filter(|f| f.total_sounds > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_test_playlist() -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test_playlist");
        root.is_dir().then_some(root)
    }

    fn first_sound(folder: &Folder) -> Option<&Sound> {
        folder.sounds.first().or_else(|| folder.children.iter().find_map(first_sound))
    }

    #[test]
    fn scans_real_playlist_when_present() {
        let Some(root) = repo_test_playlist() else {
            return; // repo fixture not present; nothing to assert
        };
        let folders = scan(&[root]);
        let total: usize = folders.iter().map(|f| f.total_sounds).sum();
        assert!(total >= 1, "expected at least one sound in test_playlist");
        let sound = folders.iter().find_map(first_sound).expect("a sound");
        assert!(matches!(sound.ext.as_str(), "mp3" | "wav" | "flac" | "ogg"));
        assert!(
            sound.duration_secs.unwrap_or(0.0) > 1.0,
            "duration should be probed, got {:?}",
            sound.duration_secs
        );
    }
}
