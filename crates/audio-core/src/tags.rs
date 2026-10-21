//! Read tags and embedded cover art from audio files (via symphonia — no extra
//! dependency). One probe yields title/artist/album, duration, and whether a
//! picture is embedded.

use std::path::Path;

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::TrackType;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTag};

#[derive(Debug, Default, Clone)]
pub struct ProbeInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f32>,
    pub has_cover: bool,
}

fn open_format(path: &Path) -> Option<Box<dyn symphonia::core::formats::FormatReader>> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(&hint, mss, Default::default(), MetadataOptions::default())
        .ok()
}

/// Duration in seconds, from the container when it knows it and from the
/// decoder frame count otherwise.
///
/// Containers differ in which of the two is reliable (an mp4 with an edit list
/// reports no frame count but a solid duration), so ask both before giving up.
pub fn duration_secs(path: &Path) -> Option<f64> {
    if let Some(info) = probe(path) {
        if let Some(d) = info.duration_secs {
            if d > 0.0 {
                return Some(d as f64);
            }
        }
    }
    match crate::decode::StreamDecoder::open(path, crate::bus::BusFormat::default()) {
        Ok(d) if d.duration_secs() > 0.0 => Some(d.duration_secs() as f64),
        _ => None,
    }
}

/// One probe for tags + duration + cover presence.
pub fn probe(path: &Path) -> Option<ProbeInfo> {
    let mut format = open_format(path)?;
    let mut info = ProbeInfo::default();

    {
        let meta = format.metadata();
        if let Some(rev) = meta.current() {
            for tag in &rev.media.tags {
                match &tag.std {
                    Some(StandardTag::TrackTitle(s)) => info.title = Some(s.to_string()),
                    Some(StandardTag::Artist(s)) => info.artist = Some(s.to_string()),
                    Some(StandardTag::Album(s)) => info.album = Some(s.to_string()),
                    _ => {}
                }
            }
            info.has_cover = !rev.media.visuals.is_empty();
        }
    }

    if let Some(track) = format.default_track(TrackType::Audio) {
        if let (Some(frames), Some(params)) = (track.num_frames, track.codec_params.as_ref()) {
            if let Some(rate) = params.audio().and_then(|a| a.sample_rate) {
                info.duration_secs = Some(frames as f32 / rate as f32);
            }
        }
    }

    Some(info)
}

/// Embedded cover art as a `data:` URL, or `None` if the file has no picture.
pub fn cover_data_url(path: &Path) -> Option<String> {
    let mut format = open_format(path)?;
    let meta = format.metadata();
    let rev = meta.current()?;
    let visual = rev.media.visuals.first()?;
    let mime = visual
        .media_type
        .clone()
        .unwrap_or_else(|| "image/jpeg".to_string());
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&visual.data);
    Some(format!("data:{mime};base64,{b64}"))
}
