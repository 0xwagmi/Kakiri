//! Fetching a sound from a URL.
//!
//! Deliberately dumb: it downloads one http(s) link, decides from the URL or
//! the `Content-Type` whether it got audio or a video, and hands the file to
//! `audio_core::import`, which knows how to get audio out of a container.
//!
//! Site scraping (YouTube/TikTok pages) is *not* here — that is what `yt-dlp`
//! is for, and it plugs in at the same seam later.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use audio_core::import::{self, Imported};

const MAX_BYTES: u64 = 512 * 1024 * 1024; // 512 MB is plenty for a sound

/// Extensions we will keep as-is when the URL advertises one.
fn audio_ext(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let last = lower.rsplit('/').next().unwrap_or("");
    let ext = last.rsplit_once('.').map(|(_, e)| e)?;
    let ext = ext.split(['?', '#']).next().unwrap_or(ext);
    import::AUDIO_EXT.iter().copied().find(|a| *a == ext)
}

fn video_ext(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let last = lower.rsplit('/').next().unwrap_or("");
    let ext = last.rsplit_once('.').map(|(_, e)| e)?;
    let ext = ext.split(['?', '#']).next().unwrap_or(ext);
    ["mp4", "m4v", "mov", "webm", "mkv", "avi", "flv"]
        .into_iter()
        .find(|a| *a == ext)
}

fn stem_from_url(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = no_query.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let stem = last.rsplit_once('.').map(|(s, _)| s).unwrap_or(last);
    let decoded = percent_decode(stem);
    import::sanitize_stem(if decoded.is_empty() { "download" } else { &decoded })
}

/// Just enough percent-decoding for file names (no external dependency).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn kind_from_content_type(ctype: &str) -> Option<&'static str> {
    let c = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Some(match c.as_str() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" | "audio/aacp" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/webm" => "ogg",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        _ => return None,
    })
}

/// Download `url` into `dest_dir` as a playable audio file.
///
/// Progress is reported as 0..1 (or a negative value while the size is
/// unknown). The file appears atomically: it is written as `*.part` and only
/// renamed when complete, so an interrupted download never shows up as a sound.
pub fn fetch(url: &str, dest_dir: &Path, on_progress: &mut dyn FnMut(f32, &str)) -> Result<Imported, String> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("Paste a link that starts with http:// or https://".into());
    }
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("cannot create {}: {e}", dest_dir.display()))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Kakiri/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|e| format!("cannot start the downloader: {e}"))?;

    on_progress(0.0, "connecting");
    let resp = client.get(url.trim()).send().map_err(|e| {
        if e.is_timeout() {
            "The site took too long to answer.".to_string()
        } else if e.is_connect() {
            "Could not reach that site — check your connection.".to_string()
        } else {
            format!("Download failed: {e}")
        }
    })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("That link answered {}", status.as_u16()));
    }

    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let total = resp.content_length().unwrap_or(0);

    // Decide what we actually got. A web page means the user pasted a link to
    // a *page* — say so plainly instead of saving 40 KB of HTML as a sound.
    let (ext, is_video) = match audio_ext(url) {
        Some(e) => (e.to_string(), false),
        None => match video_ext(url) {
            Some(e) => (e.to_string(), true),
            None => match kind_from_content_type(&ctype) {
                Some(e) => {
                    let video = e.starts_with("video/") || ["mp4", "mov", "webm", "mkv"].contains(&e);
                    (e.to_string(), video)
                }
                None if ctype.starts_with("text/html") => {
                    return Err(
                        "That link is a web page, not an audio file. Use the direct file link \
                         (it usually ends in .mp3, .m4a or .wav)."
                            .into(),
                    )
                }
                None => {
                    return Err(format!(
                        "That link is not an audio or video file (the server called it \"{ctype}\")."
                    ))
                }
            },
        },
    };

    let stem = stem_from_url(url);
    let target = import::unique_path(dest_dir, &stem, &ext);
    let part = target.with_extension(format!("part.{ext}"));

    let mut file = std::fs::File::create(&part)
        .map_err(|e| format!("cannot write to {}: {e}", part.display()))?;

    let mut body = resp;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got: u64 = 0;
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        got += n as u64;
        if got > MAX_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err("That file is larger than 512 MB.".into());
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("cannot write to {}: {e}", part.display()))?;
        if total > 0 {
            on_progress((got as f32 / total as f32).clamp(0.0, 0.99), "downloading");
        } else {
            on_progress(-1.0, "downloading");
        }
    }
    file.flush().ok();
    drop(file);

    if got < 1024 {
        let _ = std::fs::remove_file(&part);
        return Err("That download was empty — the link may need a login or has expired.".into());
    }

    if is_video {
        // Videos go through the same extraction path as a dropped file.
        let tmp_name = import::unique_path(dest_dir, &format!("{stem}.download"), &ext);
        std::fs::rename(&part, &tmp_name)
            .map_err(|e| format!("cannot finish the download: {e}"))?;
        let out = import::import_file(&tmp_name, dest_dir, |p, stage| {
            on_progress(p.max(0.0) * 0.99, stage)
        });
        let _ = std::fs::remove_file(&tmp_name);
        let imported = out?;
        on_progress(1.0, "done");
        return Ok(imported);
    }

    std::fs::rename(&part, &target).map_err(|e| format!("cannot finish the download: {e}"))?;
    on_progress(1.0, "done");
    Ok(Imported {
        name: target
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&stem)
            .to_string(),
        duration_secs: import::duration_of(&target),
        path: target.to_string_lossy().to_string(),
    })
}

/// Where URL downloads are allowed to land: an existing directory that the user
/// already trusts (a library folder, or a folder they picked).
pub fn ensure_dest(dest: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(dest);
    if p.is_dir() {
        Ok(p)
    } else {
        Err(format!("{} is not a folder", p.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_win_over_query_strings() {
        assert_eq!(audio_ext("https://x.test/a/Song.mp3?token=1"), Some("mp3"));
        assert_eq!(audio_ext("https://x.test/a.Song.M4A"), Some("m4a"));
        assert_eq!(audio_ext("https://x.test/watch?v=1"), None);
    }

    #[test]
    fn stems_come_from_the_last_path_segment() {
        assert_eq!(stem_from_url("https://x.test/a/My%20Song.mp3?t=2"), "My Song");
        assert_eq!(stem_from_url("https://x.test/a/"), "a");
        assert_eq!(stem_from_url("https://x.test"), "x.test");
    }

    #[test]
    fn content_types_map_to_extensions() {
        assert_eq!(kind_from_content_type("audio/mpeg; charset=binary"), Some("mp3"));
        assert_eq!(kind_from_content_type("audio/mp4"), Some("m4a"));
        assert_eq!(kind_from_content_type("video/mp4"), Some("mp4"));
        assert_eq!(kind_from_content_type("application/octet-stream"), None);
    }

    #[test]
    fn percent_decoding_handles_utf8() {
        assert_eq!(percent_decode("Caf%C3%A9%20Mix"), "Café Mix");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
    }

    #[test]
    fn non_http_links_are_rejected_before_any_io() {
        let err = fetch("file:///etc/passwd", Path::new("/tmp"), &mut |_, _| {}).unwrap_err();
        assert!(err.contains("http"), "{err}");
    }
}
