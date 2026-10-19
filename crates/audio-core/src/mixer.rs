//! Multi-voice mixer. Voices are routed to the virtual-mic bus, the local
//! monitor bus, or both, and summed into one interleaved f32 block each.
//!
//! Voices can repeat (the same sound triggered again layers on top). Retrigger
//! policy is decided by the caller (see `Retrigger`), using the voice `key`.

use std::sync::Arc;

use serde::Serialize;

use crate::bus::Pcm;

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
    /// Parse from the IPC string ("mic" | "monitor" | "both").
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
    /// Start another copy on top (classic soundboard).
    Layer,
    /// Stop the existing copy and restart (re-trigger / "choke").
    Choke,
    /// Stop it if playing, otherwise play (play/stop toggle).
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

pub struct Voice {
    pub id: VoiceId,
    /// Identity of the source sound (e.g. file path) — used for retrigger.
    pub key: String,
    pub label: String,
    pub pcm: Arc<Pcm>,
    pub pos: usize,
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
    /// 0.0..1.0 playback progress.
    pub progress: f32,
    pub gain: f32,
    pub looped: bool,
    pub route: Route,
}

pub struct Mixer {
    voices: Vec<Voice>,
}

impl Mixer {
    pub fn new(_channels: u16) -> Self {
        Self {
            voices: Vec::new(),
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
            pcm,
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

    /// Stop every voice with this key. Returns how many were stopped.
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

    /// Jump a playing voice to `fraction` (0.0..1.0) of its length.
    pub fn seek(&mut self, id: VoiceId, fraction: f32) {
        for v in &mut self.voices {
            if v.id == id && !v.pcm.is_empty() {
                let len = v.pcm.len();
                let f = fraction.clamp(0.0, 1.0);
                v.pos = ((len as f32 * f) as usize).min(len - 1);
            }
        }
    }

    pub fn active(&self) -> Vec<VoiceInfo> {
        self.voices
            .iter()
            .map(|v| {
                let total = v.pcm.len().max(1);
                VoiceInfo {
                    id: v.id,
                    key: v.key.clone(),
                    label: v.label.clone(),
                    progress: (v.pos as f32 / total as f32).clamp(0.0, 1.0),
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

        for v in &mut self.voices {
            if v.done || v.pcm.is_empty() {
                continue;
            }
            let len = v.pcm.len();
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
                if v.route.to_mic() {
                    for k in 0..n {
                        mic[i + k] += v.pcm[v.pos + k] * v.gain;
                    }
                }
                if v.route.to_monitor() {
                    if let Some(m) = monitor.as_deref_mut() {
                        for k in 0..n {
                            m[i + k] += v.pcm[v.pos + k] * v.gain;
                        }
                    }
                }
                v.pos += n;
                i += n;
            }
            if !v.looped && v.pos >= v.pcm.len() {
                v.done = true;
            }
        }

        self.voices.retain(|v| !v.done);

        soft_limit(mic);
        if let Some(m) = monitor.as_deref_mut() {
            soft_limit(m);
        }
        mic.iter().fold(0f32, |p, s| p.max(s.abs())).min(1.0)
    }
}

/// Soft limiter above ~0.99: stays transparent at low level and asymptotes to
/// 1.0 so the bus never exceeds full scale (which clips the device / Discord).
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
        m.render(&mut mic, None); // no monitor bus → silently dropped
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
    fn same_key_can_layer() {
        let mut m = Mixer::new(2);
        m.add(1, "same".into(), "s".into(), Arc::new(vec![0.4f32; 8]), 1.0, false, Route::Mic);
        m.add(2, "same".into(), "s".into(), Arc::new(vec![0.4f32; 8]), 1.0, false, Route::Mic);
        assert!(m.is_key_active("same"));
        let mut out = vec![0f32; 8];
        m.render(&mut out, None);
        assert!((out[0] - 0.8).abs() < 1e-6); // both layered
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
