//! Getting sounds *into* the library.
//!
//! Two jobs, both purely local:
//!
//! * copy an audio file into a library folder (one that is already playable is
//!   just copied — never re-encoded, never moved, so the user's original is
//!   left alone);
//! * pull the audio track out of a video file with `ffmpeg`, because that is
//!   how most sounds arrive from the internet (mp4/webm/mkv) and the decoder
//!   only understands audio containers.
//!
//! Nothing here talks to the network; downloads live in the app layer so this
//! crate stays dependency-free apart from the decoders.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Extensions the decoder can play as-is.
pub const AUDIO_EXT: &[&str] = &[
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "oga", "mp4a", "aiff", "aif", "wma", "mka",
];

/// What an import produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Imported {
    /// Absolute path of the file that is now in the library.
    pub path: String,
    /// Display name (file stem), the same thing the library uses.
    pub name: String,
    /// Duration in seconds, when the decoder can tell.
    pub duration_secs: f64,
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Is this something the mixer can play directly?
pub fn is_audio_path(p: &Path) -> bool {
    AUDIO_EXT.contains(&ext_of(p).as_str())
}

/// Turn anything into a safe file stem: no separators, no control chars, no
/// leading dots, and never empty.
pub fn sanitize_stem(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_space = false;
    for ch in raw.chars() {
        // Tabs read as word breaks, not as "control character to delete".
        let ch = if ch == '\t' { ' ' } else { ch };
        let keep = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => false,
            c if (c as u32) < 0x20 => false,
            _ => true,
        };
        if !keep {
            continue;
        }
        if ch == ' ' {
            if last_space || out.is_empty() {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(ch);
    }
    let trimmed = out.trim_matches(['.', ' ']).to_string();
    let clipped: String = trimmed.chars().take(120).collect();
    if clipped.is_empty() {
        "sound".to_string()
    } else {
        clipped
    }
}

/// `dir/stem.ext`, or `dir/stem (2).ext` when that already exists.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
        if n > 999 {
            break;
        }
    }
    candidate
}

/// Look for an external tool on `PATH` (and next to our own binary).
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            for cand in [dir.join(&exe), dir.join("tools").join(&exe)] {
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// `ffmpeg` is only needed for video files (and later for mp3/time ranges).
pub fn ffmpeg() -> Option<PathBuf> {
    find_tool("ffmpeg")
}

/// Duration in seconds, 0.0 when nothing can tell.
pub fn duration_of(path: &Path) -> f64 {
    crate::tags::duration_secs(path).unwrap_or(0.0)
}

/// Copy an audio file, or extract the audio from a video file, into `dest_dir`.
///
/// `on_progress` is called with 0..1 and a short stage label ("copying",
/// "extracting"). Errors are user-facing strings: the UI shows them verbatim.
pub fn import_file(
    src: &Path,
    dest_dir: &Path,
    mut on_progress: impl FnMut(f32, &str),
) -> Result<Imported, String> {
    if !src.is_file() {
        return Err(format!("{} is not a file", src.display()));
    }
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("cannot create {}: {e}", dest_dir.display()))?;

    let stem = sanitize_stem(
        src.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound"),
    );

    let target = if is_audio_path(src) {
        let ext = ext_of(src);
        let target = unique_path(dest_dir, &stem, &ext);
        on_progress(0.0, "copying");
        std::fs::copy(src, &target)
            .map_err(|e| format!("cannot copy {}: {e}", src.display()))?;
        on_progress(1.0, "copying");
        target
    } else {
        extract_audio(src, dest_dir, &stem, &mut on_progress)?
    };

    Ok(Imported {
        name: target
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&stem)
            .to_string(),
        duration_secs: duration_of(&target),
        path: target.to_string_lossy().to_string(),
    })
}

/// Pull the audio stream out of a video container.
///
/// Tries a lossless stream copy first (mp4/mov almost always carry AAC), and
/// falls back to re-encoding when the container/codec combination can't be
/// remuxed into m4a.
fn extract_audio(
    src: &Path,
    dest_dir: &Path,
    stem: &str,
    on_progress: &mut impl FnMut(f32, &str),
) -> Result<PathBuf, String> {
    let ffmpeg = ffmpeg().ok_or_else(|| {
        "Video files need ffmpeg. Install it (sudo apt install ffmpeg / brew install ffmpeg / \
         winget install ffmpeg) and try again."
            .to_string()
    })?;

    let total = duration_of(src);
    let attempts: [&[&str]; 2] = [
        &["-vn", "-c:a", "copy", "-movflags", "+faststart"],
        &["-vn", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"],
    ];

    let mut last_error = String::from("ffmpeg failed");
    for args in attempts {
        // Each attempt gets a fresh name so a failed try can't leave a file
        // behind that then looks like a valid import.
        let target = unique_path(dest_dir, stem, "m4a");
        let part = target.with_extension("part.m4a");
        let mut cmd = Command::new(&ffmpeg);
        cmd.arg("-hide_banner")
            .arg("-nostdin")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-i")
            .arg(src)
            .args(args)
            .arg("-progress")
            .arg("pipe:1")
            .arg("-nostats")
            .arg(&part)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        on_progress(0.0, "extracting");
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Err(format!("cannot run ffmpeg: {e}")),
        };

        if let Some(stdout) = child.stdout.take() {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // ffmpeg -progress emits key=value lines; out_time_ms is the
                // position within the *output*.
                if let Some(v) = line.strip_prefix("out_time_ms=") {
                    if total > 0.0 {
                        let secs = v.trim().parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
                        on_progress((secs / total).clamp(0.0, 0.99) as f32, "extracting");
                    }
                }
            }
        }
        let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
        if status.success() && part.is_file() {
            std::fs::rename(&part, &target)
                .map_err(|e| format!("cannot finish {}: {e}", target.display()))?;
            on_progress(1.0, "extracting");
            return Ok(target);
        }
        last_error = format!("ffmpeg exited with {status}");
        let _ = std::fs::remove_file(&part);
    }
    Err(format!("could not extract audio from that file ({last_error})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_are_recognised() {
        assert!(is_audio_path(Path::new("/a/b.mp3")));
        assert!(is_audio_path(Path::new("/a/b.M4A")));
        assert!(!is_audio_path(Path::new("/a/b.mp4")));
        assert!(!is_audio_path(Path::new("/a/b.webm")));
    }

    #[test]
    fn stems_are_made_filesystem_safe() {
        assert_eq!(sanitize_stem("Song: Part 2/3?"), "Song Part 23");
        assert_eq!(sanitize_stem("  ..hidden..  "), "hidden");
        assert_eq!(sanitize_stem("a\tb"), "a b");
        assert_eq!(sanitize_stem("///"), "sound");
        assert_eq!(sanitize_stem("   "), "sound");
    }

    #[test]
    fn long_stems_are_clipped() {
        let long = "x".repeat(400);
        assert_eq!(sanitize_stem(&long).chars().count(), 120);
    }

    #[test]
    fn duplicate_names_get_a_counter() {
        let dir = std::env::temp_dir().join(format!("kakiri-unique-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "song", "mp3");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "song", "mp3");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("song (2).mp3"));
        std::fs::write(&second, b"x").unwrap();
        let third = unique_path(&dir, "song", "mp3");
        assert!(third.to_string_lossy().contains("song (3).mp3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_an_audio_file_copies_it_untouched() {
        let base = std::env::temp_dir().join(format!("kakiri-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src_dir = base.join("in");
        let dest_dir = base.join("out");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("My Sound!.wav");
        std::fs::write(&src, b"not really audio").unwrap();

        let mut last = -1.0;
        let got = import_file(&src, &dest_dir, |p, _| last = p).expect("import");
        assert_eq!(got.name, "My Sound!");
        assert!(got.path.ends_with("My Sound!.wav"));
        // the source is left alone and the bytes are identical
        assert!(src.exists());
        assert_eq!(std::fs::read(&got.path).unwrap(), b"not really audio");
        assert_eq!(last, 1.0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_source_is_a_readable_error() {
        let err = import_file(
            Path::new("/definitely/not/here.mp3"),
            &std::env::temp_dir(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(err.contains("not a file"), "{err}");
    }
}
