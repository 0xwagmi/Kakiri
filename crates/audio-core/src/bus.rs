//! The mix-bus format: one sample rate + channel layout that the entire engine
//! runs at. Files are decoded/resampled to this once and then mixed.

pub const DEFAULT_RATE: u32 = 48_000;
pub const DEFAULT_CHANNELS: u16 = 2;
pub const MAX_VOICES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusFormat {
    pub rate: u32,
    pub channels: u16,
}

impl Default for BusFormat {
    fn default() -> Self {
        Self {
            rate: DEFAULT_RATE,
            channels: DEFAULT_CHANNELS,
        }
    }
}

impl BusFormat {
    pub fn new(rate: u32, channels: u16) -> Self {
        Self { rate, channels }
    }

    /// Number of frames (per-channel samples) in `ms` milliseconds.
    pub fn frames(&self, ms: u32) -> usize {
        (self.rate as u64 * ms as u64 / 1000) as usize
    }

    pub fn bytes_per_frame(&self) -> usize {
        self.channels as usize * size_of::<f32>()
    }
}

/// Interleaved f32 PCM for the whole bus.
pub type Pcm = Vec<f32>;

/// A sine test tone with short fades in/out so it never clicks.
/// Returns interleaved samples for `format.channels` channels.
pub fn sine(format: BusFormat, freq: f32, ms: u32, amp: f32) -> Pcm {
    let frames = format.frames(ms).max(1);
    let fade = format.frames(10).clamp(1, frames / 2);
    let ch = format.channels as usize;
    let mut out = Vec::with_capacity(frames * ch);
    for i in 0..frames {
        let t = i as f32 / format.rate as f32;
        let env = if i < fade {
            i as f32 / fade as f32
        } else if i >= frames - fade {
            (frames - i) as f32 / fade as f32
        } else {
            1.0
        };
        let s = (t * freq * std::f32::consts::TAU).sin() * amp * env;
        for _ in 0..ch {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_length_and_peak() {
        let fmt = BusFormat::default();
        let pcm = sine(fmt, 440.0, 1000, 0.25);
        assert_eq!(pcm.len(), fmt.frames(1000) * fmt.channels as usize);
        let peak = pcm.iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!((peak - 0.25).abs() < 0.01, "peak was {peak}");
    }

    #[test]
    fn sine_starts_and_ends_silent() {
        let fmt = BusFormat::default();
        let pcm = sine(fmt, 440.0, 200, 1.0);
        assert_eq!(pcm[0], 0.0);
        assert!(pcm[pcm.len() - 1].abs() < 0.05);
    }
}
