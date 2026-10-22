//! The URL import path, tested against a real (tiny) HTTP server on localhost,
//! so content-type sniffing, extension choice and the error messages are all
//! exercised for real.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use soundboard_lib::download;

/// Serve one canned response per connection, for `requests` connections.
fn serve(requests: usize, status: &'static str, ctype: &'static str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&body);
            let _ = sock.flush();
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// A tiny but genuinely decodable WAV, so the post-download playability check
/// passes exactly like it would for a real sound.
fn wav_bytes(secs: f32) -> Vec<u8> {
    let rate = 8000u32;
    let frames = (rate as f32 * secs) as u32;
    let mut data = Vec::new();
    for i in 0..frames {
        let v = ((i as f32 * 0.1).sin() * 8000.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kakiri-dl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn downloads_an_audio_file_and_names_it_from_the_url() {
    let dir = temp_dir("audio");
    let body = wav_bytes(0.3);
    let base = serve(1, "200 OK", "audio/wav", body.clone());

    let mut seen: Vec<f32> = Vec::new();
    let got = download::fetch("test", &format!("{base}/sounds/My%20Drop.wav"), &dir, None, &mut |pct, _| {
        seen.push(pct)
    })
    .expect("fetch");

    assert_eq!(got.name, "My Drop");
    assert!(got.path.ends_with("My Drop.wav"), "{}", got.path);
    assert!(!got.converted, "a playable download is kept as-is");
    assert_eq!(std::fs::read(&got.path).unwrap().len(), body.len());
    // no leftover .part file, and progress ran to completion
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("part"))
        .collect();
    assert!(leftovers.is_empty(), "partial file left behind");
    assert_eq!(seen.last().copied(), Some(1.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_extension_is_recovered_from_the_content_type() {
    let dir = temp_dir("ctype");
    // no extension → the content type is peeked, then the file is fetched
    let base = serve(4, "200 OK", "audio/wav", wav_bytes(0.3));
    let got = download::fetch("test", &format!("{base}/stream?id=99"), &dir, None, &mut |_, _| {})
        .expect("fetch");
    assert!(got.path.ends_with(".wav"), "{}", got.path);
    assert!(!got.converted);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_web_page_is_reported_as_such_instead_of_saved() {
    let dir = temp_dir("page");
    let base = serve(
        1,
        "200 OK",
        "text/html; charset=utf-8",
        b"<!doctype html><html>hi</html>".to_vec(),
    );
    let err = download::fetch_direct(&format!("{base}/watch?v=abc"), &dir, &mut |_, _| {}).unwrap_err();
    assert!(err.contains("web page"), "{err}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing saved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn http_errors_surface_the_status_code() {
    let dir = temp_dir("404");
    let base = serve(1, "404 Not Found", "text/plain", b"nope".to_vec());
    let err = download::fetch_direct(&format!("{base}/gone.mp3"), &dir, &mut |_, _| {}).unwrap_err();
    assert!(err.contains("404"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_year_old_downloader_is_flagged_as_stale() {
    // yt-dlp versions are release dates, so age is directly computable
    assert!(download::is_stale("2024.01.01"));
    assert!(!download::is_stale("2999.01.01"), "far future is not stale");
    assert!(download::is_stale("2025.08.11"), "the copy that broke on YouTube");
    assert!(!download::is_stale("garbage"));
}

#[test]
fn progress_percentages_are_parsed_from_the_downloader_output() {
    // the shapes yt-dlp actually prints
    assert_eq!(
        download::parse_ytdlp_percent("[download]  42.3% of  5.00MiB at 1.20MiB/s ETA 00:03"),
        Some(42.3)
    );
    assert_eq!(download::parse_ytdlp_percent("[download] 100% of 655.71KiB"), Some(100.0));
    assert_eq!(download::parse_ytdlp_percent("[youtube] extracting URL"), None);
    assert_eq!(download::parse_ytdlp_percent("WARNING: something"), None);
}

#[test]
fn a_file_link_is_recognised_by_its_extension_without_asking_the_server() {
    // .mp3 → straight to the file path, no downloader involved
    let dir = temp_dir("route-file");
    let base = serve(1, "200 OK", "audio/wav", wav_bytes(0.3));
    let got = download::fetch("test", &format!("{base}/drop.wav"), &dir, None, &mut |_, _| {})
        .expect("fetch");
    assert!(got.path.ends_with("drop.wav"), "{}", got.path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clip_range_without_a_downloader_says_so() {
    let dir = temp_dir("range");
    let err = download::fetch("test", "http://127.0.0.1:1/x.mp3", &dir, Some((1.0, 2.0)), &mut |_, _| {})
        .unwrap_err();
    assert!(err.contains("yt-dlp"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_destination_that_is_not_a_folder_is_refused_up_front() {
    let err = download::ensure_dest("/definitely/not/a/folder").unwrap_err();
    assert!(err.contains("not a folder"), "{err}");
}
