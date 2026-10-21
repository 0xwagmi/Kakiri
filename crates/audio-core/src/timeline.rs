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
    /// Loop the timeline. Live, so toggling it never restarts playback.
    looped: AtomicBool,
    /// Playhead in milliseconds (avoids f64 atomics).
    position: AtomicU64,
    /// End of the last clip, in milliseconds.
    length: AtomicU64,
}

impl Shared {
    fn set_length(&self, secs: f64) {
        self.length.store((secs.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
    }
}

/// The clips a running player is playing.
///
/// This lives behind a mutex (rather than being owned by the scheduler thread)
/// so writing to the timeline while it plays — adding a sound, deleting one,
/// dragging one — does not need a restart. A restart would drop the playhead
/// and cut every voice, which is exactly what you don't want mid-call.
struct Arrangement {
    clips: Vec<Clip>,
    /// Durations probed once, so the scheduler never touches the filesystem.
    durs: Vec<f64>,
    /// Which clips this round has already started.
    started: Vec<bool>,
}

impl Arrangement {
    fn new(clips: Vec<Clip>, durs: Vec<f64>, from: f64) -> Self {
        // Clips that already finished before `from` are skipped outright.
        let started = clips
            .iter()
            .zip(&durs)
            .map(|(c, d)| c.start + d <= from)
            .collect();
        Self {
            clips,
            durs,
            started,
        }
    }

    fn length(&self) -> f64 {
        self.clips
            .iter()
            .zip(&self.durs)
            .map(|(c, d)| c.start.max(0.0) + d.max(0.0))
            .fold(0.0, f64::max)
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.id == id)
    }

    /// Add or replace a clip, keeping the two vecs the same length.
    fn put(&mut self, clip: Clip, dur: f64) {
        match self.index_of(&clip.id) {
            Some(i) => {
                // Same id → keep `started`: a clip that is already sounding
                // must not be started a second time by the replacement.
                self.clips[i] = clip;
                self.durs[i] = dur;
            }
            None => {
                self.clips.push(clip);
                self.durs.push(dur);
                self.started.push(false);
            }
        }
    }

    fn take(&mut self, id: &str) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.clips.remove(i);
                self.durs.remove(i);
                self.started.remove(i);
                true
            }
            None => false,
        }
    }

    fn restart_round(&mut self) {
        for s in &mut self.started {
            *s = false;
        }
    }
}

/// A running (or finished) timeline playback.
pub struct Player {
    shared: Arc<Shared>,
    arrangement: Arc<Mutex<Arrangement>>,
    /// (clip id, voice id) so a removed clip can be silenced on its own.
    voices: Arc<Mutex<Vec<(String, u64)>>>,
    engine: Arc<Engine>,
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

    /// Add a clip mid-playback without disturbing the playhead or the voices
    /// that are already sounding. A clip whose start is already behind the
    /// playhead begins immediately, part-way in.
    pub fn add(&self, clip: Clip) -> Result<TimelineStatus, String> {
        if clip.start < 0.0 {
            return Err("clip start must not be negative".into());
        }
        let dur = duration_of(&clip.path, &self.engine);
        let len = {
            let mut a = self.arrangement.lock().unwrap();
            a.put(clip, dur);
            a.length()
        };
        self.shared.set_length(len);
        Ok(self.status())
    }

    /// Remove a clip mid-playback, silencing only that clip's voice.
    pub fn remove(&self, id: &str) -> TimelineStatus {
        let len = {
            let mut a = self.arrangement.lock().unwrap();
            if !a.take(id) {
                return self.status();
            }
            a.length()
        };
        self.shared.set_length(len);
        self.silence(id);
        self.status()
    }

    /// Live loop toggle — no restart, the playhead keeps running.
    pub fn set_looped(&self, looped: bool) {
        self.shared.looped.store(looped, Ordering::Relaxed);
    }

    /// Stop the voices belonging to one clip, leaving the rest playing.
    fn silence(&self, id: &str) {
        let mut v = self.voices.lock().unwrap();
        let mut kept = Vec::with_capacity(v.len());
        for (clip_id, voice) in v.drain(..) {
            if clip_id == id {
                self.engine.stop(voice);
            } else {
                kept.push((clip_id, voice));
            }
        }
        *v = kept;
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

/// Duration of a file; falls back to a small non-zero value so a clip still
/// gets a sensible width in the UI.
fn duration_of(path: &str, _engine: &Engine) -> f64 {
    crate::tags::duration_secs(&PathBuf::from(path)).unwrap_or(1.0)
}

/// Start playing `clips`, looping if `looped`.
pub fn play(
    engine: Arc<Engine>,
    clips: Vec<Clip>,
    looped: bool,
    from_secs: f64,
) -> Result<Player, String> {
    let from = from_secs.max(0.0);
    let clips: Vec<Clip> = clips.into_iter().filter(|c| c.start >= 0.0).collect();
    // Probe each file once, up front: the scheduler must never block on I/O.
    let durs: Vec<f64> = clips.iter().map(|c| duration_of(&c.path, &engine)).collect();
    let arrangement = Arc::new(Mutex::new(Arrangement::new(clips, durs, from)));
    let length = arrangement.lock().unwrap().length();

    let shared = Arc::new(Shared {
        playing: AtomicBool::new(true),
        stop: AtomicBool::new(false),
        looped: AtomicBool::new(looped),
        position: AtomicU64::new((from * 1000.0) as u64),
        length: AtomicU64::new((length * 1000.0) as u64),
    });

    let voices: Arc<Mutex<Vec<(String, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let sh = shared.clone();
    let arr = arrangement.clone();
    let vox = voices.clone();
    let eng = engine.clone();
    let handle = thread::Builder::new()
        .name("timeline".into())
        .spawn(move || run(eng, arr, vox, from, sh))
        .map_err(|e| format!("spawn timeline thread: {e}"))?;

    Ok(Player {
        shared,
        arrangement,
        voices,
        engine,
        handle: Mutex::new(Some(handle)),
    })
}

fn run(
    engine: Arc<Engine>,
    arr: Arc<Mutex<Arrangement>>,
    voices: Arc<Mutex<Vec<(String, u64)>>>,
    from: f64,
    sh: Arc<Shared>,
) {
    // Round trips (wrapping) of the whole timeline.
    let mut round = 0u64;
    // Each round gets its own clock, so the playhead stays correct even when
    // the arrangement grows mid-play and the loop length changes under it.
    let mut round_start = Instant::now() - Duration::from_secs_f64(from);

    loop {
        if sh.stop.load(Ordering::Relaxed) {
            break;
        }

        let length = sh.length.load(Ordering::Relaxed) as f64 / 1000.0;
        let mut pos = round_start.elapsed().as_secs_f64();

        if length > 0.0 && pos >= length {
            if sh.looped.load(Ordering::Relaxed) {
                round += 1;
                round_start = Instant::now();
                pos = 0.0;
                arr.lock().unwrap().restart_round();
            } else {
                sh.set_length(length);
                sh.position.store((length * 1000.0) as u64, Ordering::Relaxed);
                sh.playing.store(false, Ordering::Relaxed);
                break;
            }
        }
        sh.position
            .store((pos.max(0.0) * 1000.0) as u64, Ordering::Relaxed);

        // Collect the clips that are due, then start them outside the lock so a
        // slow file open can never stall the playhead.
        let mut due: Vec<(Clip, f64, f64)> = Vec::new();
        {
            let mut a = arr.lock().unwrap();
            for i in 0..a.clips.len() {
                if a.started[i] || pos < a.clips[i].start {
                    continue;
                }
                a.started[i] = true;
                // Scrub support: if the playhead is already inside this clip,
                // start it part-way through instead of from the top.
                let skip = (pos - a.clips[i].start).max(0.0);
                if skip >= a.durs[i] - 0.02 {
                    continue; // entirely behind the playhead — nothing to hear
                }
                due.push((a.clips[i].clone(), skip, a.durs[i]));
            }
        }

        for (clip, skip, dur) in due {
            spawn_clip(&engine, &voices, &sh, round, clip, skip, dur);
        }

        thread::sleep(TICK);
    }

    // Stop only the voices this timeline started; anything the user triggered
    // by hand keeps playing.
    for (_, id) in voices.lock().unwrap().drain(..) {
        engine.stop(id);
    }
    sh.playing.store(false, Ordering::Relaxed);
}

fn spawn_clip(
    engine: &Arc<Engine>,
    voices: &Arc<Mutex<Vec<(String, u64)>>>,
    sh: &Arc<Shared>,
    round: u64,
    clip: Clip,
    skip: f64,
    _dur: f64,
) {
    let key = format!("{KEY_PREFIX}{round}:{}", clip.id);
    // Mirror manual triggering: also play to the monitor if it's on, so you can
    // hear the arrangement you're building.
    let route = if engine.monitor_enabled() {
        Route::Both
    } else {
        Route::Mic
    };
    let label = PathBuf::from(&clip.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string();
    let path = PathBuf::from(&clip.path);
    let gain = clip.gain;
    let clip_id = clip.id;
    let eng = engine.clone();
    let ids = voices.clone();
    let stop = sh.clone();
    let spawned = thread::Builder::new()
        .name("timeline-clip".into())
        .spawn(move || {
            if stop.stop.load(Ordering::Relaxed) {
                return; // the timeline was stopped while we were starting
            }
            match eng.play_stream_from(key, label, path, gain, false, route, skip) {
                Ok(id) => ids.lock().unwrap().push((clip_id, id)),
                Err(e) => eprintln!("[timeline] {e}"),
            }
        });
    if spawned.is_err() {
        eprintln!("[timeline] could not spawn clip thread");
    }
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
    fn adding_a_clip_keeps_the_arrangement_consistent() {
        let mut a = Arrangement::new(vec![clip("a", 0.0)], vec![2.0], 0.0);
        assert!((a.length() - 2.0).abs() < 1e-9);
        a.put(clip("b", 3.0), 2.0);
        // `durs` and `started` must grow with `clips` or the scheduler panics
        assert_eq!(a.clips.len(), 2);
        assert_eq!(a.durs.len(), 2);
        assert_eq!(a.started.len(), 2);
        assert!((a.length() - 5.0).abs() < 1e-9, "a longer clip extends the end");
    }

    #[test]
    fn adding_an_existing_id_replaces_without_unstarting_it() {
        let mut a = Arrangement::new(vec![clip("a", 0.0)], vec![2.0], 0.0);
        a.started[0] = true;
        let mut moved = clip("a", 4.0);
        moved.gain = 0.5;
        a.put(moved, 2.0);
        assert_eq!(a.clips.len(), 1, "no duplicate clip");
        assert!(a.started[0], "a clip that is already sounding stays started");
        assert_eq!(a.clips[0].gain, 0.5);
        assert!((a.length() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn removing_a_clip_shrinks_the_length() {
        let mut a = Arrangement::new(vec![clip("a", 0.0), clip("b", 5.0)], vec![2.0, 2.0], 0.0);
        assert!((a.length() - 7.0).abs() < 1e-9);
        assert!(a.take("b"));
        assert_eq!(a.clips.len(), 1);
        assert_eq!(a.durs.len(), 1);
        assert_eq!(a.started.len(), 1);
        assert!((a.length() - 2.0).abs() < 1e-9);
        assert!(!a.take("nope"), "removing an unknown id is a no-op");
    }

    #[test]
    fn restarted_round_starts_every_clip_again() {
        let mut a = Arrangement::new(vec![clip("a", 0.0), clip("b", 1.0)], vec![2.0, 2.0], 0.0);
        a.started.iter_mut().for_each(|s| *s = true);
        a.restart_round();
        assert!(a.started.iter().all(|s| !*s));
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
