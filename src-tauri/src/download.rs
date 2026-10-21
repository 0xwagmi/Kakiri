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

/// Jobs that currently own a child process, so the UI can cancel them.
static RUNNING: std::sync::Mutex<Vec<(String, u32)>> = std::sync::Mutex::new(Vec::new());

fn register(job: &str, pid: u32) {
    if job.is_empty() {
        return;
    }
    let mut r = RUNNING.lock().unwrap();
    r.retain(|(j, _)| j != job);
    r.push((job.to_string(), pid));
}

fn unregister(job: &str) {
    if job.is_empty() {
        return;
    }
    RUNNING.lock().unwrap().retain(|(j, _)| j != job);
}

/// Jobs cancelled by the UI (used by downloads that have no child process).
static CANCELLED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn clear_cancel(job: &str) {
    CANCELLED.lock().unwrap().retain(|j| j != job);
}

fn is_cancelled(job: &str) -> bool {
    CANCELLED.lock().unwrap().iter().any(|j| j == job)
}

/// Kill whatever `job` is doing. Returns true when something was running.
pub fn cancel(job: &str) -> bool {
    // remember it, so a request-based download can stop at its next chunk
    {
        let mut c = CANCELLED.lock().unwrap();
        if !c.iter().any(|j| j == job) {
            c.push(job.to_string());
        }
    }
    let pid = {
        let r = RUNNING.lock().unwrap();
        r.iter().find(|(j, _)| j == job).map(|(_, p)| *p)
    };
    let Some(pid) = pid else { return true };
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill").arg(pid.to_string()).output();
    }
    unregister(job);
    true
}

/// Where a downloader we fetched ourselves lives (`<app data>/tools`).
static TOOLS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_tools_dir(dir: PathBuf) {
    let _ = TOOLS_DIR.set(dir);
}

fn tool_exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn managed_tool(name: &str) -> Option<PathBuf> {
    let dir = TOOLS_DIR.get()?;
    let p = dir.join(tool_exe(name));
    p.is_file().then_some(p)
}

fn version_of(exe: &Path) -> Option<String> {
    let out = std::process::Command::new(exe)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// The best `yt-dlp` we can find: our managed copy or whatever is on `PATH`,
/// whichever is newer. Being out of date is the normal failure mode, so a stale
/// system copy must never shadow a fresh one (or the other way round).
pub fn ytdlp() -> Option<(PathBuf, String)> {
    let mut best: Option<(PathBuf, String)> = None;
    for cand in [managed_tool("yt-dlp"), import::find_tool("yt-dlp")]
        .into_iter()
        .flatten()
    {
        let Some(version) = version_of(&cand) else {
            continue;
        };
        let newer = best.as_ref().is_none_or(|(_, v)| version > *v);
        if newer {
            best = Some((cand, version));
        }
    }
    best
}

/// Read a child's pipe line by line on its own thread, so stdout and stderr
/// can both be parsed without deadlocking on a full pipe.
fn pump<R: std::io::Read + Send + 'static>(r: R, tx: std::sync::mpsc::Sender<String>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(r).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

fn friendly_ytdlp_error(raw: &str) -> String {
    let msg = raw.trim();
    let low = msg.to_ascii_lowercase();
    if low.contains("unsupported url") {
        return "The downloader doesn't recognise that site.".into();
    }
    if low.contains("not available on this app") || low.contains("sign in to confirm") {
        return "The site refused the download — the downloader is out of date.".into();
    }
    if low.contains("requested format is not available") {
        return "That video has no audio-only stream the downloader can fetch.".into();
    }
    if low.contains("private") || low.contains("removed") || low.contains("404") {
        return "That video is private, removed or region-locked.".into();
    }
    format!("Downloader said: {msg}")
}

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
/// One link in, a playable file out — direct media links go straight down,
/// page links (YouTube/TikTok/…) go through yt-dlp.
pub fn fetch(
    job: &str,
    url: &str,
    dest_dir: &Path,
    range: Option<(f64, f64)>,
    on_progress: &mut dyn FnMut(f32, &str),
) -> Result<Imported, String> {
    let trimmed = url.trim();
    // A URL that ends in a media extension is a file; download it directly.
    let obvious_media = audio_ext(trimmed).is_some() || video_ext(trimmed).is_some();
    // Otherwise ask the server what it would send. Download endpoints often have
    // no extension but still serve a file, and page links answer text/html.
    let page = if obvious_media {
        false
    } else {
        match peek_content_type(trimmed) {
            Some(ct) => !(ct.starts_with("audio/") || ct.starts_with("video/")),
            None => true, // no answer (blocked, JS-only…) — let the downloader try
        }
    };

    if page {
        if ytdlp().is_some() {
            return fetch_ytdlp(job, trimmed, dest_dir, range, on_progress);
        }
        if range.is_some() {
            return Err("Cutting a clip needs the downloader (yt-dlp) and ffmpeg.".into());
        }
        return Err(MISSING_YTDLP.to_string());
    }
    if range.is_some() {
        return Err("Cutting a clip needs the downloader (yt-dlp) and ffmpeg.".into());
    }
    fetch_direct(trimmed, dest_dir, on_progress)
}

/// Ask for the headers only, to find out what kind of thing a link points at.
fn peek_content_type(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Kakiri/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    Some(ct)
}

/// Download a link that is known to be a file (no page handling).
pub fn fetch_direct(url: &str, dest_dir: &Path, on_progress: &mut dyn FnMut(f32, &str)) -> Result<Imported, String> {
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

/// What a page link resolves to, for the "Get" preview in the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageInfo {
    pub title: String,
    pub duration_secs: f64,
}

/// Read a page's metadata without downloading it (also validates the link).
pub fn probe_page(url: &str) -> Result<PageInfo, String> {
    probe_page_job("", url)
}

fn probe_page_job(job: &str, url: &str) -> Result<PageInfo, String> {
    let (exe, _) = ytdlp().ok_or_else(|| MISSING_YTDLP.to_string())?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["-J", "--no-playlist", "--no-warnings", url])
        .stdin(std::process::Stdio::null())
        // both pipes must be captured: without this the child inherits ours,
        // its output lands in the app's console and the error text is lost
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot run the downloader: {e}"))?;
    register(job, child.id());
    let out = child
        .wait_with_output()
        .map_err(|e| format!("cannot run the downloader: {e}"))?;
    unregister(job);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .find(|l| l.contains("ERROR"))
            .unwrap_or(err.trim())
            .to_string();
        return Err(friendly_ytdlp_error(&line));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("unreadable metadata: {e}"))?;
    Ok(PageInfo {
        title: json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("download")
            .to_string(),
        duration_secs: json.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
    })
}

pub const MISSING_YTDLP: &str =
    "Page links (YouTube, TikTok, …) need the downloader. Set it up from the import panel.";

/// Download a page link's audio into `dest_dir`, optionally just a slice.
pub fn fetch_ytdlp(
    job: &str,
    url: &str,
    dest_dir: &Path,
    range: Option<(f64, f64)>,
    on_progress: &mut dyn FnMut(f32, &str),
) -> Result<Imported, String> {
    let (exe, _version) = ytdlp().ok_or_else(|| MISSING_YTDLP.to_string())?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("cannot create {}: {e}", dest_dir.display()))?;

    on_progress(-1.0, "reading the page");
    let info = probe_page_job(job, url)?;
    if range.is_some() && import::ffmpeg().is_none() {
        return Err("Cutting a clip needs ffmpeg installed.".into());
    }

    let stem = import::sanitize_stem(&info.title);
    // Unique prefix so two jobs into the same folder can't mix up their files.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let prefix = format!(".kakiri-{}-{stamp}", std::process::id());
    let template = dest_dir.join(format!("{prefix}.%(ext)s"));

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("-f")
        .arg("bestaudio[ext=m4a]/bestaudio/best")
        .args(["--no-playlist", "--no-warnings", "--newline", "--no-simulate"])
        .arg("-o")
        .arg(&template);
    if let Some((start, end)) = range {
        let (a, b) = (start.max(0.0), end.max(start + 0.05));
        cmd.arg("--download-sections")
            .arg(format!("*{a:.2}-{b:.2}"))
            .args(["--force-keyframes-at-cuts"]);
    }
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    on_progress(0.0, "downloading");
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot run the downloader: {e}"))?;
    register(job, child.id());
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        pump(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        pump(err, tx.clone());
    }
    drop(tx);

    let mut complaint = String::new();
    for line in rx {
        if line.contains("ERROR") {
            complaint = line.clone();
        }
        if let Some(pct) = parse_ytdlp_percent(&line) {
            on_progress((pct / 100.0).clamp(0.0, 0.98), "downloading");
        } else if line.contains("[ExtractAudio]") || line.contains("[Merger]") {
            on_progress(0.99, "extracting");
        }
    }

    let status = child.wait().map_err(|e| format!("downloader: {e}"))?;
    unregister(job);
    if !status.success() {
        for e in std::fs::read_dir(dest_dir).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        return Err(if complaint.is_empty() {
            format!("The downloader failed ({status}).")
        } else {
            friendly_ytdlp_error(&complaint)
        });
    }

    // Find what it wrote (<prefix>.<ext>) and give it the real title.
    let produced = std::fs::read_dir(dest_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        });
    let produced = produced.ok_or("The downloader produced no file.")?;
    let ext = produced
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("m4a")
        .to_string();

    // Clipped downloads are already playable as-is (m4a); anything else gets
    // normalised by the importer so the library never sees a raw container.
    let target = import::unique_path(dest_dir, &stem, &ext);
    std::fs::rename(&produced, &target)
        .map_err(|e| format!("cannot finish the download: {e}"))?;

    on_progress(1.0, "done");
    Ok(Imported {
        name: target
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&stem)
            .to_string(),
        duration_secs: if range.is_some() {
            (range.unwrap().1 - range.unwrap().0).max(0.0)
        } else if info.duration_secs > 0.0 {
            info.duration_secs
        } else {
            import::duration_of(&target)
        },
        path: target.to_string_lossy().to_string(),
    })
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// How old a `YYYY.MM.DD` version string is, in days.
fn version_age_days(version: &str) -> Option<i64> {
    let date = version.split('.').take(3).collect::<Vec<_>>();
    if date.len() < 3 {
        return None;
    }
    let y: i64 = date[0].parse().ok()?;
    let m: i64 = date[1].parse().ok()?;
    let d: i64 = date[2].parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64
        / 86_400;
    Some(now - days_from_civil(y, m, d))
}

/// These sites change constantly; anything older than this is a liability.
const STALE_AFTER_DAYS: i64 = 120;

pub fn is_stale(version: &str) -> bool {
    version_age_days(version).is_some_and(|age| age > STALE_AFTER_DAYS)
}

/// `[download]  42.3% of ...` → 42.3
pub fn parse_ytdlp_percent(line: &str) -> Option<f32> {
    let rest = line.split_once("[download]")?.1.trim_start();
    let pct = rest.split_once('%')?.0.trim();
    pct.parse::<f32>().ok()
}

/// What the import panel needs to know about its helper tools.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Tools {
    pub ffmpeg: bool,
    pub ytdlp: Option<String>,
    /// True when the downloader exists but is old enough to be untrustworthy.
    pub ytdlp_stale: bool,
}

pub fn tools() -> Tools {
    let ytdlp = ytdlp();
    Tools {
        ffmpeg: import::ffmpeg().is_some(),
        ytdlp_stale: ytdlp.as_ref().map(|(_, v)| is_stale(v)).unwrap_or(false),
        ytdlp: ytdlp.map(|(_, v)| v),
    }
}

/// Fetch the downloader itself into `<app data>/tools` (17–39 MB, once).
pub fn ensure_ytdlp(job: &str, on_progress: &mut dyn FnMut(f32, &str)) -> Result<String, String> {
    // No child process here, so cancellation is a flag the copy loop checks.
    clear_cancel(job);
    let dir = TOOLS_DIR
        .get()
        .cloned()
        .ok_or("no tools directory configured")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let asset = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let url =
        format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
    let target = dir.join(tool_exe("yt-dlp"));
    let part = target.with_extension("part");

    on_progress(0.0, "downloading the downloader");
    let mut resp = reqwest::blocking::Client::builder()
        .user_agent(concat!("Kakiri/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .map_err(|e| format!("cannot reach GitHub: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}", resp.status().as_u16()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file =
        std::fs::File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    loop {
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if is_cancelled(job) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err("Cancelled.".into());
        }
        got += n as u64;
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        if total > 0 {
            on_progress((got as f32 / total as f32).clamp(0.0, 0.99), "downloading the downloader");
        }
    }
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("cannot make it executable: {e}"))?;
    }
    std::fs::rename(&part, &target).map_err(|e| format!("cannot install it: {e}"))?;

    let version = version_of(&target).ok_or_else(|| {
        let _ = std::fs::remove_file(&target);
        "The downloaded file does not run (wrong platform?).".to_string()
    })?;
    on_progress(1.0, "done");
    Ok(version)
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
