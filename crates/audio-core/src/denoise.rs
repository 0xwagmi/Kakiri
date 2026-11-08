//! RNNoise voice denoising for the live input, via the pure-Rust `nnnoiseless`
//! port of Xiph's rnnoise.
//!
//! Why here and not in Easy Effects: an external processor is another process
//! *and* another graph hop, so it costs tens of milliseconds. This runs on the
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
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            input: VecDeque::with_capacity(FRAME_SIZE * 2),
            output: VecDeque::with_capacity(FRAME_SIZE * 2),
            last: 0.0,
            vad: 0.0,
            frame_in: vec![0.0; FRAME_SIZE],
            frame_out: vec![0.0; FRAME_SIZE],
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
            self.output
                .extend(self.frame_out.iter().map(|s| s / SCALE));
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
