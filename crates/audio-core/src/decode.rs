//! Decode any supported audio file to the bus format using symphonia
//! (pure Rust, no ffmpeg).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::TrackType;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::bus::{BusFormat, Pcm};

pub struct Decoded {
    /// Interleaved f32 PCM at the target bus format.
    pub pcm: Pcm,
    pub src_rate: u32,
    pub src_channels: usize,
    pub duration_secs: f32,
}

fn make_hint(path: &Path) -> Hint {
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    hint
}

/// Cheap: probe the container/codec and read the duration without decoding.
pub fn probe_duration(path: &Path) -> Option<f32> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let format = symphonia::default::get_probe()
        .probe(&make_hint(path), mss, Default::default(), MetadataOptions::default())
        .ok()?;
    let track = format.default_track(TrackType::Audio)?;
    let params = track.codec_params.as_ref()?.audio()?;
    let rate = params.sample_rate?;
    let frames = track.num_frames?;
    Some(frames as f32 / rate as f32)
}

/// Decode a whole file to interleaved f32 in the target bus format.
pub fn decode_file(path: &Path, target: BusFormat) -> Result<Decoded> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&make_hint(path), mss, Default::default(), MetadataOptions::default())
        .map_err(|e| anyhow!("unsupported/corrupt format: {e}"))?;

    let (track_id, src_rate, src_channels, num_frames, mut decoder) = {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| anyhow!("no audio track"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| anyhow!("no audio codec parameters"))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| anyhow!("unsupported codec: {e}"))?;
        (
            track.id,
            params.sample_rate.unwrap_or(target.rate),
            params.channels.as_ref().map(|c| c.count()).unwrap_or(2),
            track.num_frames,
            decoder,
        )
    };

    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => {
                eprintln!("[decode] {e}");
                break;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let n = buf.samples_interleaved();
                let start = interleaved.len();
                interleaved.resize(start + n, 0.0);
                buf.copy_to_slice_interleaved(&mut interleaved[start..]);
            }
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let pcm = to_bus(&interleaved, src_channels, src_rate, target);
    let decoded_secs = pcm.len() as f32 / target.channels as f32 / target.rate as f32;
    let duration_secs = num_frames
        .map(|n| n as f32 / src_rate as f32)
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(decoded_secs);

    Ok(Decoded {
        pcm,
        src_rate,
        src_channels,
        duration_secs,
    })
}

/// Channel-map and resample decoded PCM to the bus format.
fn to_bus(input: &[f32], src_ch: usize, src_rate: u32, target: BusFormat) -> Pcm {
    let tch = target.channels as usize;
    let src_ch = src_ch.max(1);
    let src_frames = input.len() / src_ch;

    let mut mapped = Vec::with_capacity(src_frames * tch);
    for f in 0..src_frames {
        let base = f * src_ch;
        if src_ch == tch {
            mapped.extend_from_slice(&input[base..base + src_ch]);
        } else if src_ch == 1 {
            let s = input[base];
            for _ in 0..tch {
                mapped.push(s);
            }
        } else if tch == 1 {
            let s = (0..src_ch).map(|c| input[base + c]).sum::<f32>() / src_ch as f32;
            mapped.push(s);
        } else {
            // copy up to tch channels, repeating the last available one
            for c in 0..tch {
                mapped.push(input[base + c.min(src_ch - 1)]);
            }
        }
    }

    resample_linear(&mapped, tch, src_rate, target.rate)
}

/// Simple linear-interpolation resampler (fine for short soundboard clips).
fn resample_linear(input: &[f32], channels: usize, from: u32, to: u32) -> Pcm {
    if from == to || input.is_empty() || channels == 0 {
        return input.to_vec();
    }
    let in_frames = input.len() / channels;
    if in_frames == 0 {
        return Vec::new();
    }
    let ratio = to as f64 / from as f64;
    let out_frames = ((in_frames as f64) * ratio).round() as usize;
    let mut out = vec![0f32; out_frames * channels];
    for o in 0..out_frames {
        let src = o as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(in_frames - 1);
        let t = (src - i0 as f64) as f32;
        for c in 0..channels {
            let a = input[i0 * channels + c];
            let b = input[i1 * channels + c];
            out[o * channels + c] = a + (b - a) * t;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_doubles_frames() {
        let input = vec![0.0f32, 0.0, 1.0, 1.0]; // 2 frames stereo
        let out = resample_linear(&input, 2, 24000, 48000);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn mono_to_stereo_duplicates() {
        let input = vec![0.1f32, 0.2, 0.3];
        let out = to_bus(&input, 1, 48000, BusFormat::new(48000, 2));
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], 0.1);
        assert_eq!(out[1], 0.1);
        assert_eq!(out[4], 0.3);
    }

    #[test]
    fn decodes_real_mp3_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test_playlist/memes/Lock-Em-Up.mp3");
        if !path.is_file() {
            return; // repo fixture not present
        }
        let fmt = BusFormat::default();
        let d = decode_file(&path, fmt).expect("decode mp3");
        assert!(!d.pcm.is_empty());
        assert_eq!(d.pcm.len() % fmt.channels as usize, 0);
        assert!(d.duration_secs > 4.0 && d.duration_secs < 6.0, "dur {}", d.duration_secs);
        let peak = d.pcm.iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.05, "decoded mp3 is silent (peak {peak})");
    }
}
