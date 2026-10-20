//! Timeline / arranger playback.
//!
//! Clips are placed at a start time on a track and the clips are simply played
//! through the existing mixer. Tracks only exist for layout: because a clip
//! starts a normal voice, clips that overlap in time are *mixed together*
//! automatically, exactly like triggering two sounds by hand.
//!
//! A small scheduler thread watches a monotonic clock and starts each clip when
//! the playhead reaches it. No extra audio path, no extra buffering.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::mixer::Route;

/// How often the scheduler wakes up. The audible error of a clip start is at
/// most one tick, which is ~1 frame of video — inaudible in a voice call.
const TICK: Duration = Duration::from_millis(4);

/// Never let two voices of the timeline collide on a key.
const KEY_PREFIX: &str = "timeline:";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Clip {
    /// Stable id from the UI (used to report what's playing).
    #[serde(default)]
    pub id: String,
    /// Absolute path of the audio file.
    pub path: String,
    /// Row in the arrange view. Layout only — does not affect the audio.
    #[serde(default)]
    pub track: usize,
    /// Start time on the timeline, in seconds.
    #[serde(default)]
    pub start: f64,
    /// Fader for this clip.
    #[serde(default = "default_gain")]
    pub gain: f32,
}

fn default_gain() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TimelineStatus {
    pub playing: bool,
    /// Playhead, in seconds.
    pub position: f64,
    /// End of the last clip, in seconds.
    pub length: f64,
}

struct Shared {
    playing: AtomicBool,
    stop: AtomicBool,
    /// Playhead in milliseconds (avoids f64 atomics).
    position: AtomicU64,
    length: AtomicU64,
}

/// A running (or finished) timeline playback.
pub struct Player {
    shared: Arc<Shared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Player {
    pub fn status(&self) -> TimelineStatus {
        TimelineStatus {
            playing: self.shared.playing.load(Ordering::Relaxed),
            position: self.shared.position.load(Ordering::Relaxed) as f64 / 1000.0,
            length: self.shared.length.load(Ordering::Relaxed) as f64 / 1000.0,
        }
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.playing.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Total length of `clips` (end of the last one), in seconds, using `dur` to
/// look up each file's duration.
pub fn length_with(clips: &[Clip], dur: impl Fn(&str) -> f64) -> f64 {
    clips
        .iter()
        .map(|c| c.start.max(0.0) + dur(&c.path).max(0.0))
        .fold(0.0, f64::max)
}

/// Total length of `clips` (end of the last one), in seconds.
pub fn length_of(clips: &[Clip], engine: &Engine) -> f64 {
    length_with(clips, |p| duration_of(p, engine))
}

/// Duration of a file. Uses the stream decoder's frame count; falls back to a
/// small non-zero value so a clip still gets a sensible width in the UI.
fn duration_of(path: &str, engine: &Engine) -> f64 {
    match crate::decode::StreamDecoder::open(&PathBuf::from(path), engine.format()) {
        Ok(d) if d.duration_secs() > 0.0 => d.duration_secs() as f64,
        _ => 1.0,
    }
}

fn clip_len(engine: &Engine, c: &Clip) -> f64 {
    duration_of(&c.path, engine)
}

/// Start playing `clips`, looping if `looped`.
pub fn play(
    engine: Arc<Engine>,
    clips: Vec<Clip>,
    looped: bool,
    from_secs: f64,
) -> Result<Player, String> {
    let clips: Vec<Clip> = clips.into_iter().filter(|c| c.start >= 0.0).collect();
    let length = length_with(&clips, |p| duration_of(p, &engine));

    let shared = Arc::new(Shared {
        playing: AtomicBool::new(true),
        stop: AtomicBool::new(false),
        position: AtomicU64::new((from_secs.max(0.0) * 1000.0) as u64),
        length: AtomicU64::new((length * 1000.0) as u64),
    });

    let sh = shared.clone();
    let handle = thread::Builder::new()
        .name("timeline".into())
        .spawn(move || run(engine, clips, looped, length, from_secs.max(0.0), sh))
        .map_err(|e| format!("spawn timeline thread: {e}"))?;

    Ok(Player {
        shared,
        handle: Mutex::new(Some(handle)),
    })
}

fn run(
    engine: Arc<Engine>,
    clips: Vec<Clip>,
    looped: bool,
    length: f64,
    from: f64,
    sh: Arc<Shared>,
) {
    // Clips that already finished before `from` are skipped outright; a clip
    // that *spans* `from` starts immediately, seeked into its middle.
    let mut started: Vec<bool> = clips
        .iter()
        .map(|c| c.start + clip_len(&engine, c) <= from)
        .collect();
    let mut voices: Vec<u64> = Vec::new();
    // Round trips (wrapping) of the whole timeline.
    let mut round = 0u64;
    // Shift the clock so the playhead begins at `from`.
    let started_at = Instant::now() - Duration::from_secs_f64(from);

    loop {
        if sh.stop.load(Ordering::Relaxed) {
            break;
        }

        let elapsed = started_at.elapsed().as_secs_f64();
        let mut pos = elapsed - round as f64 * length;
        if length > 0.0 && pos >= length {
            if looped {
                round += 1;
                pos = elapsed - round as f64 * length;
                started.iter_mut().for_each(|s| *s = false);
            } else {
                sh.position
                    .store((length * 1000.0) as u64, Ordering::Relaxed);
                sh.playing.store(false, Ordering::Relaxed);
                break;
            }
        }
        sh.position
            .store((pos.max(0.0) * 1000.0) as u64, Ordering::Relaxed);

        for (i, clip) in clips.iter().enumerate() {
            if started[i] {
                continue;
            }
            // Start slightly before the nominal time so the first samples are
            // already flowing when the playhead crosses the clip.
            if pos < clip.start {
                continue;
            }
            started[i] = true;
            let key = format!("{KEY_PREFIX}{round}:{}", clip.id);
            // Scrub support: if the playhead is already inside this clip, start
            // it part-way through instead of from the top.
            let skip = (pos - clip.start).max(0.0);
            match engine.play_stream_from(
                key,
                PathBuf::from(&clip.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("clip")
                    .to_string(),
                PathBuf::from(&clip.path),
                clip.gain,
                false,
                // Mirror manual triggering: also play to the monitor if it's on,
                // so you can hear the arrangement you're building.
                if engine.monitor_enabled() { Route::Both } else { Route::Mic },
                skip,
            ) {
                Ok(id) => voices.push(id),
                Err(e) => eprintln!("[timeline] {}: {e}", clip.path),
            }
        }

        thread::sleep(TICK);
    }

    // Stop only the voices this timeline started; anything the user triggered
    // by hand keeps playing.
    for id in voices {
        engine.stop(id);
    }
    sh.playing.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, start: f64) -> Clip {
        Clip { id: id.into(), path: format!("/{id}.mp3"), track: 0, start, gain: 1.0 }
    }

    #[test]
    fn length_is_end_of_last_clip() {
        let clips = vec![clip("a", 0.0), clip("b", 5.0)];
        // every fake file is 2 s long → 5 + 2
        let len = length_with(&clips, |_| 2.0);
        assert!((len - 7.0).abs() < 1e-9);
    }

    #[test]
    fn length_ignores_overlap_and_negative_starts() {
        // `b` overlaps `a`; the length is still the end, not the sum
        let clips = vec![clip("a", 0.0), clip("b", 1.0), clip("c", -4.0)];
        let len = length_with(&clips, |_| 3.0);
        assert!((len - 4.0).abs() < 1e-9);
    }

    #[test]
    fn empty_timeline_has_zero_length() {
        assert_eq!(length_with(&[], |_| 5.0), 0.0);
    }

    #[test]
    fn clip_deserializes_with_defaults() {
        let c: Clip = serde_json::from_str(r#"{"path":"/x.mp3"}"#).unwrap();
        assert_eq!(c.gain, 1.0);
        assert_eq!(c.track, 0);
        assert_eq!(c.start, 0.0);
        assert!(c.id.is_empty());
    }

    #[test]
    fn clip_round_trips() {
        let c = Clip {
            id: "7".into(),
            path: "/a/b.mp3".into(),
            track: 3,
            start: 12.5,
            gain: 0.4,
        };
        let back: Clip = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.id, c.id);
        assert_eq!(back.path, c.path);
        assert_eq!(back.track, c.track);
        assert_eq!(back.start, c.start);
        assert_eq!(back.gain, c.gain);
    }
}
