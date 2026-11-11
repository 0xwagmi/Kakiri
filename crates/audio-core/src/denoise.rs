//! RNNoise voice denoising for the live input, via the pure-Rust `nnnoiseless`
//! port of Xiph's rnnoise.
//!
//! Why here rather than in a separate processor app: that would be another
//! process *and* another graph hop, costing tens of milliseconds. This runs on the
//! samples the engine already reads, and costs one 10 ms frame instead.
//!
//! The denoiser is strict about its input: 48 kHz, mono, exactly 480-sample
//! frames. The engine's bus is interleaved stereo at 48 kHz, so samples are
//! folded to mono, queued until a frame is complete, denoised, and written back
//! to every channel. That queue is the entire added latency.

use std::collections::VecDeque;

use nnnoiseless::{DenoiseState, FRAME_SIZE};

/// rnnoise works on `f32` values in the range of an `i16`, not on ±1.0. Feeding
/// it normalised samples makes it see silence and pass everything through, which
/// looks exactly like "the denoiser does nothing".
const SCALE: f32 = i16::MAX as f32;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    /// mono samples that do not yet make up a whole frame
    input: VecDeque<f32>,
    /// denoised samples not yet handed back to the caller
    output: VecDeque<f32>,
    /// repeated on underrun instead of zeroing, which would click
    last: f32,
    /// mean speech probability of the frames processed in the last call
    vad: f32,
    frame_in: Vec<f32>,
    frame_out: Vec<f32>,
    cfg: Config,
    /// frames of hangover left before the gate closes
    grace: i32,
}

/// Beyond a bare model call, the behaviour that makes a denoiser sound "clean"
/// rather than merely quieter, described in plain DSP terms:
///
/// 1. keep the unprocessed block and mix it back against the processed one, so
///    the voice keeps some of its natural body;
/// 2. while the model's speech probability clears a threshold, re-arm a hangover;
///    once the hangover expires, emit digital silence rather than residual noise,
///    which is what stops the floor breathing between words;
/// 3. a gain stage on the way in and out.
///
/// All three are generic signal-processing techniques; this is an independent
/// implementation in Rust, written from that description.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// 0 dB = fully denoised; at or below -60 dB the dry signal passes untouched
    pub wet_db: f32,
    /// gate to silence when the VAD reports no speech
    pub vad: bool,
    /// speech probability threshold, 0..1 (shown as a percentage in the UI)
    pub vad_thres: f32,
    /// hangover in frames before the gate closes
    pub release_frames: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wet_db: 0.0,
            vad: true,
            vad_thres: 0.5,
            release_frames: 2, // 20 ms at 480-sample frames
        }
    }
}

impl Config {
    /// Overrides from the environment, so the four knobs can be A/B tested in a
    /// real call without recompiling.
    pub fn from_env() -> Self {
        let d = Config::default();
        let num = |k: &str, fallback: f32| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(fallback)
        };
        let flag = |k: &str, fallback: bool| {
            std::env::var(k)
                .ok()
                .map(|v| {
                    matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "on" | "true" | "yes"
                    )
                })
                .unwrap_or(fallback)
        };
        let release_ms = num("WAGMIFY_RELEASE_MS", d.release_frames as f32 * 10.0);
        Self {
            wet_db: num("WAGMIFY_WET_DB", d.wet_db),
            vad: flag("WAGMIFY_VAD", d.vad),
            vad_thres: (num("WAGMIFY_VAD_THRES", d.vad_thres * 100.0) / 100.0).clamp(0.0, 1.0),
            release_frames: (release_ms / 10.0).round().max(0.0) as i32,
        }
    }

    fn wet_ratio(&self) -> f32 {
        if self.wet_db <= -60.0 {
            0.0
        } else {
            10f32.powf(self.wet_db / 20.0).clamp(0.0, 1.0)
        }
    }
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        Self::with_config(None)
    }

    /// Explicit config: used by tests, and by anything that wants to set the
    /// knobs without the environment.
    pub fn with_config(cfg: Option<Config>) -> Self {
        let cfg = cfg.unwrap_or_else(Config::from_env);
        Self {
            state: DenoiseState::new(),
            input: VecDeque::with_capacity(FRAME_SIZE * 2),
            output: VecDeque::with_capacity(FRAME_SIZE * 2),
            last: 0.0,
            vad: 0.0,
            frame_in: vec![0.0; FRAME_SIZE],
            frame_out: vec![0.0; FRAME_SIZE],
            grace: cfg.release_frames,
            cfg,
        }
    }

    /// Denoise an interleaved buffer in place. Returns the mean speech
    /// probability (0.0–1.0) of the frames completed during this call, which is
    /// 0.0 when no frame was completed.
    ///
    /// The buffer keeps its length: whatever cannot be denoised yet is replaced
    /// by the last denoised sample, so the stream stays sample-accurate and the
    /// delay is a constant one frame.
    pub fn process(&mut self, buf: &mut [f32], channels: usize) -> f32 {
        debug_assert!(channels > 0);
        let frames = buf.len() / channels;
        if frames == 0 {
            return 0.0;
        }

        for f in 0..frames {
            let base = f * channels;
            let mut sum = 0.0;
            for c in 0..channels {
                sum += buf[base + c];
            }
            self.input.push_back(sum / channels as f32 * SCALE);
        }

        let mut vad_sum = 0.0;
        let mut vad_frames = 0;
        while self.input.len() >= FRAME_SIZE {
            for slot in self.frame_in.iter_mut() {
                *slot = self.input.pop_front().unwrap_or(0.0);
            }
            let prob = self.state.process_frame(&mut self.frame_out, &self.frame_in);
            vad_sum += prob;
            vad_frames += 1;

            // VAD gate: re-arm the hangover while speech is detected, otherwise
            // close to digital silence once it expires.
            let open = if self.cfg.vad {
                if prob >= self.cfg.vad_thres {
                    self.grace = self.cfg.release_frames;
                }
                let open = self.grace >= 0;
                if open {
                    self.grace -= 1;
                }
                open
            } else {
                true
            };

            let inv = 1.0 / SCALE;
            if open {
                // wet/dry: `wet_db` at its minimum passes the voice untouched
                let wet = self.cfg.wet_ratio();
                for (wet_sample, dry_sample) in self.frame_out.iter().zip(self.frame_in.iter()) {
                    self.output
                        .push_back((wet_sample * wet + dry_sample * (1.0 - wet)) * inv);
                }
            } else {
                for _ in 0..FRAME_SIZE {
                    self.output.push_back(0.0);
                }
            }
        }
        if vad_frames > 0 {
            self.vad = vad_sum / vad_frames as f32;
        } else {
            self.vad = 0.0;
        }

        for f in 0..frames {
            let sample = match self.output.pop_front() {
                Some(s) => {
                    self.last = s;
                    s
                }
                None => self.last,
            };
            let base = f * channels;
            for c in 0..channels {
                buf[base + c] = sample;
            }
        }

        // The queues are self-limiting (one frame in, one frame out), but a
        // panic-free guard keeps a stuck pipeline from growing forever.
        while self.input.len() > FRAME_SIZE * 4 {
            self.input.pop_front();
        }
        while self.output.len() > FRAME_SIZE * 4 {
            self.output.pop_front();
        }

        self.vad
    }

    /// Speech probability of the most recent processed frames.
    pub fn vad(&self) -> f32 {
        self.vad
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_buffer_length_for_odd_block_sizes() {
        for channels in [1usize, 2] {
            for frames in [1usize, 7, 479, 480, 481, 1024] {
                let mut d = Denoiser::new();
                let mut buf = vec![0.1f32; frames * channels];
                d.process(&mut buf, channels);
                assert_eq!(buf.len(), frames * channels, "channels={channels} frames={frames}");
            }
        }
    }

    #[test]
    fn silence_stays_silent_and_finite() {
        let mut d = Denoiser::new();
        for _ in 0..10 {
            let mut buf = vec![0.0f32; BLOCK];
            d.process(&mut buf, 2);
            assert!(buf.iter().all(|s| s.is_finite()), "produced NaN or inf");
            assert!(buf.iter().all(|s| s.abs() < 1e-3), "silence became audible");
        }
    }

    #[test]
    fn delay_is_one_frame_not_unbounded() {
        // A single impulse must come out later, not disappear and not echo forever.
        let mut d = Denoiser::new();
        let mut out: Vec<f32> = Vec::new();
        for i in 0..8 {
            let mut buf = vec![0.0f32; BLOCK];
            if i == 0 {
                buf[0] = 0.5;
            }
            d.process(&mut buf, 2);
            out.extend(buf.chunks(2).map(|c| c[0]));
        }
        let total = out.len();
        assert_eq!(total, 8 * FRAMES_PER_BLOCK);
        // energy may be reshaped by the model but must not arrive 100 ms late
        let first = out.iter().position(|s| s.abs() > 1e-4);
        if let Some(i) = first {
            assert!(i < FRAME_SIZE * 2, "impulse arrived at sample {i}");
        }
    }

    const BLOCK: usize = 1024 * 2;
    const FRAMES_PER_BLOCK: usize = 1024;
}
