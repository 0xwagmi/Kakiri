//! Multi-voice mixer. Voices are routed to the virtual-mic bus, the local
//! monitor bus, or both, and summed into one interleaved f32 block each.
//!
//! A voice's audio can be either fully decoded (`Buffered`) or decoded on the
//! fly from a per-voice decoder thread (`Stream`) — the latter keeps memory
//! bounded for long files while starting playback quickly.

use std::sync::Arc;

use serde::Serialize;

use crate::bus::Pcm;
use crate::stream::StreamVoice;

pub type VoiceId = u64;

/// Where a voice is heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    /// Only the virtual mic (the call hears it, you don't).
    Mic,
    /// Only your monitor output (you hear it, the call doesn't) — "preview".
    Monitor,
    /// Both.
    Both,
}

impl Route {
    pub fn to_mic(self) -> bool {
        matches!(self, Route::Mic | Route::Both)
    }
    pub fn to_monitor(self) -> bool {
        matches!(self, Route::Monitor | Route::Both)
    }
    pub fn parse(s: &str) -> Route {
        match s {
            "monitor" => Route::Monitor,
            "both" => Route::Both,
            _ => Route::Mic,
        }
    }
}

/// What to do when the same sound is triggered while it's already playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Retrigger {
    Layer,
    Choke,
    Toggle,
}

impl Retrigger {
    pub fn parse(s: &str) -> Retrigger {
        match s {
            "choke" => Retrigger::Choke,
            "toggle" => Retrigger::Toggle,
            _ => Retrigger::Layer,
        }
    }
}

pub enum VoiceSource {
    /// Fully decoded PCM.
    Buffered(Arc<Pcm>),
    /// Streamed from a decoder thread.
    Stream(StreamVoice),
}

pub struct Voice {
    pub id: VoiceId,
    pub key: String,
    pub label: String,
    pub source: VoiceSource,
    pub pos: usize, // Buffered only
    pub gain: f32,
    pub looped: bool,
    pub route: Route,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceInfo {
    pub id: VoiceId,
    pub key: String,
    pub label: String,
    pub progress: f32,
    pub gain: f32,
    pub looped: bool,
    pub route: Route,
}

pub struct Mixer {
    voices: Vec<Voice>,
    scratch: Vec<f32>,
}

impl Mixer {
    pub fn new(_channels: u16) -> Self {
        Self {
            voices: Vec::new(),
            scratch: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        id: VoiceId,
        key: String,
        label: String,
        pcm: Arc<Pcm>,
        gain: f32,
        looped: bool,
        route: Route,
    ) {
        self.voices.push(Voice {
            id,
            key,
            label,
            source: VoiceSource::Buffered(pcm),
            pos: 0,
            gain,
            looped,
            route,
            done: false,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_stream(
        &mut self,
        id: VoiceId,
        key: String,
        label: String,
        voice: StreamVoice,
        gain: f32,
        looped: bool,
        route: Route,
    ) {
        self.voices.push(Voice {
            id,
            key,
            label,
            source: VoiceSource::Stream(voice),
            pos: 0,
            gain,
            looped,
            route,
            done: false,
        });
    }

    pub fn stop(&mut self, id: VoiceId) {
        for v in &mut self.voices {
            if v.id == id {
                v.done = true;
            }
        }
        self.voices.retain(|v| !v.done);
    }

    pub fn stop_key(&mut self, key: &str) -> usize {
        let mut n = 0;
        for v in &mut self.voices {
            if v.key == key {
                v.done = true;
                n += 1;
            }
        }
        self.voices.retain(|v| !v.done);
        n
    }

    pub fn is_key_active(&self, key: &str) -> bool {
        self.voices.iter().any(|v| v.key == key)
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    pub fn set_gain(&mut self, id: VoiceId, gain: f32) {
        for v in &mut self.voices {
            if v.id == id {
                v.gain = gain;
            }
        }
    }

    pub fn set_all_gain(&mut self, gain: f32) {
        for v in &mut self.voices {
            v.gain = gain;
        }
    }

    /// Seek to `fraction` (0..1) of the source.
    pub fn seek(&mut self, id: VoiceId, fraction: f32) {
        let f = fraction.clamp(0.0, 1.0);
        for v in &mut self.voices {
            if v.id != id {
                continue;
            }
            match &mut v.source {
                VoiceSource::Buffered(pcm) => {
                    if !pcm.is_empty() {
                        let len = pcm.len();
                        v.pos = ((len as f32 * f) as usize).min(len - 1);
                    }
                }
                VoiceSource::Stream(sv) => {
                    let total = sv.total_frames();
                    sv.request_seek((total as f64 * f as f64) as u64);
                }
            }
        }
    }

    pub fn active(&self) -> Vec<VoiceInfo> {
        self.voices
            .iter()
            .map(|v| {
                let progress = match &v.source {
                    VoiceSource::Buffered(pcm) => {
                        let total = pcm.len().max(1);
                        (v.pos as f32 / total as f32).clamp(0.0, 1.0)
                    }
                    VoiceSource::Stream(sv) => sv.progress(),
                };
                VoiceInfo {
                    id: v.id,
                    key: v.key.clone(),
                    label: v.label.clone(),
                    progress,
                    gain: v.gain,
                    looped: v.looped,
                    route: v.route,
                }
            })
            .collect()
    }

    /// Render one block into the mic bus and (optionally) the monitor bus.
    /// Returns the mic-bus peak level.
    pub fn render(&mut self, mic: &mut [f32], mut monitor: Option<&mut [f32]>) -> f32 {
        mic.fill(0.0);
        if let Some(m) = monitor.as_deref_mut() {
            m.fill(0.0);
        }

        let mut scratch = std::mem::take(&mut self.scratch);
        if scratch.len() < mic.len() {
            scratch.resize(mic.len(), 0.0);
        }

        for v in &mut self.voices {
            if v.done {
                continue;
            }
            let gain = v.gain;
            let to_mic = v.route.to_mic();
            let to_monitor = v.route.to_monitor();
            match &mut v.source {
                VoiceSource::Buffered(pcm) => {
                    if pcm.is_empty() {
                        continue;
                    }
                    let len = pcm.len();
                    let mut i = 0;
                    while i < mic.len() {
                        if v.pos >= len {
                            if v.looped {
                                v.pos = 0;
                            } else {
                                v.done = true;
                                break;
                            }
                        }
                        let n = (len - v.pos).min(mic.len() - i);
                        if to_mic {
                            for k in 0..n {
                                mic[i + k] += pcm[v.pos + k] * gain;
                            }
                        }
                        if to_monitor {
                            if let Some(m) = monitor.as_deref_mut() {
                                for k in 0..n {
                                    m[i + k] += pcm[v.pos + k] * gain;
                                }
                            }
                        }
                        v.pos += n;
                        i += n;
                    }
                    if !v.looped && v.pos >= len {
                        v.done = true;
                    }
                }
                VoiceSource::Stream(sv) => {
                    let n = sv.read(&mut scratch[..mic.len()]);
                    if n == 0 && sv.eof() {
                        v.done = true;
                        continue;
                    }
                    if n > 0 {
                        if to_mic {
                            for k in 0..n {
                                mic[k] += scratch[k] * gain;
                            }
                        }
                        if to_monitor {
                            if let Some(m) = monitor.as_deref_mut() {
                                for k in 0..n {
                                    m[k] += scratch[k] * gain;
                                }
                            }
                        }
                    }
                }
            }
        }

        self.scratch = scratch;
        self.voices.retain(|v| !v.done);

        soft_limit(mic);
        if let Some(m) = monitor.as_deref_mut() {
            soft_limit(m);
        }
        mic.iter().fold(0f32, |p, s| p.max(s.abs())).min(1.0)
    }
}

/// Soft limiter above ~0.99: transparent at low level, asymptotes to 1.0.
pub fn soft_limit(buf: &mut [f32]) {
    if buf.iter().fold(0f32, |m, s| m.max(s.abs())) > 0.99 {
        for s in buf.iter_mut() {
            let a = s.abs();
            if a > 0.99 {
                let over = a - 0.99;
                *s = s.signum() * (0.99 + 0.01 * (over / (over + 0.5)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix(route: Route) -> (f32, f32) {
        let mut m = Mixer::new(2);
        m.add(1, "k".into(), "a".into(), Arc::new(vec![0.5f32; 8]), 1.0, false, route);
        let mut mic = vec![0f32; 8];
        let mut mon = vec![0f32; 8];
        m.render(&mut mic, Some(&mut mon));
        (mic[0], mon[0])
    }

    #[test]
    fn route_mic_only() {
        assert_eq!(mix(Route::Mic), (0.5, 0.0));
    }

    #[test]
    fn route_monitor_only() {
        assert_eq!(mix(Route::Monitor), (0.0, 0.5));
    }

    #[test]
    fn route_both() {
        assert_eq!(mix(Route::Both), (0.5, 0.5));
    }

    #[test]
    fn monitor_optional() {
        let mut m = Mixer::new(2);
        m.add(1, "k".into(), "x".into(), Arc::new(vec![0.25f32; 4]), 1.0, false, Route::Monitor);
        let mut mic = vec![0f32; 4];
        m.render(&mut mic, None);
        assert!(mic.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn voices_overlap_and_sum() {
        let mut m = Mixer::new(2);
        m.add(1, "a".into(), "a".into(), Arc::new(vec![0.5f32; 8]), 1.0, false, Route::Mic);
        m.add(2, "b".into(), "b".into(), Arc::new(vec![0.25f32; 8]), 1.0, false, Route::Mic);
        let mut out = vec![0f32; 8];
        m.render(&mut out, None);
        assert!(out.iter().all(|s| (*s - 0.75).abs() < 1e-6));
        assert!(m.active().is_empty());
    }

    #[test]
    fn stop_key_removes_all_copies() {
        let mut m = Mixer::new(2);
        m.add(1, "same".into(), "s".into(), Arc::new(vec![0.4f32; 8]), 1.0, true, Route::Mic);
        m.add(2, "same".into(), "s".into(), Arc::new(vec![0.4f32; 8]), 1.0, true, Route::Mic);
        assert_eq!(m.stop_key("same"), 2);
        assert!(!m.is_key_active("same"));
    }

    #[test]
    fn looping_voice_persists() {
        let mut m = Mixer::new(2);
        m.add(1, "loop".into(), "loop".into(), Arc::new(vec![0.5f32; 4]), 1.0, true, Route::Mic);
        let mut out = vec![0f32; 16];
        m.render(&mut out, None);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-6));
        assert_eq!(m.active().len(), 1);
    }
}
