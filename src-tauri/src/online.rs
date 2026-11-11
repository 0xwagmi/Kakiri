//! The "Online" mode's data: what to play, not the audio itself.
//!
//! Two kinds of source, deliberately:
//!
//! - **key-less** (Deezer for charts and search, iTunes as a fallback): works on
//!   first launch with no account, and carries artwork and a duration, which the
//!   player uses to reject a bad match.
//! - **Last.fm** (optional, needs an API key from your own account): the personal
//!   layer — loved tracks, and tag "stations" standing in for playlists, which
//!   Last.fm no longer has as an object.
//!
//! Neither of them serves audio. Playback resolves each row to a video with
//! yt-dlp and streams it, so nothing is ever downloaded or stored.
//!
//! All calls are blocking and run on Tauri's blocking pool; they never touch an
//! audio thread.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OnlineTrack {
    /// stable enough to be a voice key: "deezer:123", "lastfm:artist|title"
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    /// seconds; 0 = unknown (Last.fm doesn't say)
    pub duration: u32,
    pub source: String,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(6))
        .user_agent("Wagmify")
        .build()
        .map_err(|e| format!("http client: {e}"))
}

// ---------------------------------------------------------------------------
// Deezer: no key, has artwork + duration
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DzEnvelope<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct DzTrack {
    id: u64,
    title: String,
    #[serde(default)]
    duration: u32,
    artist: DzArtist,
    #[serde(default)]
    album: Option<DzAlbum>,
}

#[derive(Deserialize)]
struct DzArtist {
    name: String,
}

#[derive(Deserialize)]
struct DzAlbum {
    cover_medium: Option<String>,
}

impl From<DzTrack> for OnlineTrack {
    fn from(t: DzTrack) -> Self {
        Self {
            id: format!("deezer:{}", t.id),
            title: t.title,
            artist: t.artist.name,
            artwork: t.album.and_then(|a| a.cover_medium),
            duration: t.duration,
            source: "deezer".into(),
        }
    }
}

fn deezer(url: &str) -> Result<Vec<OnlineTrack>, String> {
    let text = client()?
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("could not reach the charts: {e}"))?
        .text()
        .map_err(|e| format!("could not read the chart response: {e}"))?;
    let body: DzEnvelope<DzTrack> = serde_json::from_str(&text)
        .map_err(|e| format!("unexpected chart response: {e}"))?;
    Ok(body.data.into_iter().map(OnlineTrack::from).collect())
}

pub fn charts(limit: u32) -> Result<Vec<OnlineTrack>, String> {
    deezer(&format!(
        "https://api.deezer.com/chart/0/tracks?limit={}",
        limit.clamp(1, 50)
    ))
}

pub fn search(query: &str, limit: u32) -> Result<Vec<OnlineTrack>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let encoded: String = urlencode(q);
    let limit = limit.clamp(1, 50);
    match deezer(&format!(
        "https://api.deezer.com/search?q={encoded}&limit={limit}"
    )) {
        Ok(v) if !v.is_empty() => Ok(v),
        // Deezer is occasionally unavailable/NRU; iTunes needs no key either
        _ => itunes(q, limit),
    }
}

#[derive(Deserialize)]
struct ItEnvelope {
    results: Vec<ItTrack>,
}

#[derive(Deserialize)]
struct ItTrack {
    #[serde(default)]
    track_id: u64,
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    artwork_url100: Option<String>,
    #[serde(default)]
    track_time_millis: u64,
}

fn itunes(query: &str, limit: u32) -> Result<Vec<OnlineTrack>, String> {
    let text = client()?
        .get(format!(
            "https://itunes.apple.com/search?media=music&entity=song&limit={limit}&term={}",
            urlencode(query)
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("could not reach the music search: {e}"))?
        .text()
        .map_err(|e| format!("could not read the search response: {e}"))?;
    let body: ItEnvelope = serde_json::from_str(&text)
        .map_err(|e| format!("unexpected search response: {e}"))?;
    Ok(body
        .results
        .into_iter()
        .map(|t| OnlineTrack {
            id: format!("itunes:{}", t.track_id),
            title: t.track_name,
            artist: t.artist_name,
            artwork: t.artwork_url100,
            duration: (t.track_time_millis / 1000) as u32,
            source: "itunes".into(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Last.fm: the personal layer (needs your own API key)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LfEnvelope {
    #[serde(default)]
    lovedtracks: Option<LfTracks>,
    #[serde(default)]
    toptracks: Option<LfTracks>,
}

#[derive(Deserialize)]
struct LfTracks {
    #[serde(default)]
    track: Vec<LfTrack>,
}

#[derive(Deserialize)]
struct LfTrack {
    name: String,
    artist: LfArtist,
}

#[derive(Deserialize)]
struct LfArtist {
    #[serde(rename = "#text")]
    name: String,
}

impl From<LfTrack> for OnlineTrack {
    fn from(t: LfTrack) -> Self {
        Self {
            id: format!("lastfm:{}|{}", t.artist.name, t.name),
            title: t.name,
            artist: t.artist.name,
            artwork: None, // Last.fm sends no image here; the player falls back
            duration: 0,
            source: "lastfm".into(),
        }
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn lastfm(method: &str, extra: &str, api_key: &str) -> Result<Vec<OnlineTrack>, String> {
    if api_key.trim().is_empty() {
        return Err("add a Last.fm API key in Settings to use this".into());
    }
    let url = format!(
        "https://ws.audioscrobbler.com/2.0/?method={method}&api_key={}&format=json{extra}",
        urlencode(api_key.trim())
    );
    let raw = client()?
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("could not reach Last.fm: {e}"))?
        .text()
        .map_err(|e| format!("unexpected Last.fm response: {e}"))?;
    let env: LfEnvelope = serde_json::from_str(&raw)
        .map_err(|_| "Last.fm rejected that API key or user".to_string())?;
    let list = env
        .lovedtracks
        .or(env.toptracks)
        .map(|t| t.track)
        .unwrap_or_default();
    Ok(list.into_iter().map(OnlineTrack::from).collect())
}

/// Tracks you loved, newest first.
pub fn loved(user: &str, api_key: &str) -> Result<Vec<OnlineTrack>, String> {
    lastfm(
        "user.getlovedtracks",
        &format!("&user={}&limit=30", urlencode(user.trim())),
        api_key,
    )
}

/// A tag as a "playlist": e.g. `synthwave`, `lofi`, `dnb`.
pub fn tag_station(tag: &str, api_key: &str) -> Result<Vec<OnlineTrack>, String> {
    lastfm(
        "tag.gettoptracks",
        &format!("&tag={}&limit=30", urlencode(tag.trim())),
        api_key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencodes_spaces_and_symbols() {
        assert_eq!(urlencode("daft punk"), "daft+punk");
        assert_eq!(urlencode("AC/DC"), "AC%2FDC");
    }

    #[test]
    fn lastfm_ids_are_stable_and_unique() {
        let t = OnlineTrack::from(LfTrack {
            name: "One More Time".into(),
            artist: LfArtist { name: "Daft Punk".into() },
        });
        assert_eq!(t.id, "lastfm:Daft Punk|One More Time");
        assert_eq!(t.duration, 0);
    }
}
