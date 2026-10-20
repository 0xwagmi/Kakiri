//! Stateful, chunk-boundary-continuous resampler.
//!
//! Uses Catmull-Rom cubic interpolation (4-point, C1-continuous) which is a good
//! quality/complexity tradeoff for realtime streaming — far better than linear
//! interpolation, and it keeps its history + fractional cursor across calls so
//! there are no discontinuities at chunk boundaries.

/// Interpolation history: 4 frames are enough for Catmull-Rom (needs i1-1..i1+2).
const HIST: usize = 4;

pub struct StreamResampler {
    channels: usize,
    step: f64,   // input frames advanced per output frame (= src/dst)
    bypass: bool,
    history: Vec<f32>, // HIST * channels, frames immediately before the next chunk
    cursor: f64,       // absolute input-frame position of the next output sample
    processed: u64,    // absolute input frames consumed so far
}

impl StreamResampler {
    pub fn new(src_rate: u32, dst_rate: u32, channels: u16) -> Self {
        let ch = channels as usize;
        Self {
            channels: ch,
            step: src_rate as f64 / dst_rate as f64,
            bypass: src_rate == dst_rate,
            history: vec![0.0; HIST * ch],
            cursor: 0.0,
            processed: 0,
        }
    }

    /// Reset to the start (e.g. after a seek).
    pub fn reset(&mut self) {
        self.history.iter_mut().for_each(|s| *s = 0.0);
        self.cursor = 0.0;
        self.processed = 0;
    }

    /// Resample `chunk` (interleaved, `channels` ch) appending to `out`.
    pub fn process(&mut self, chunk: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        let n = chunk.len() / ch;
        if n == 0 {
            return;
        }
        if self.bypass {
            out.extend_from_slice(chunk);
            self.processed += n as u64;
            return;
        }

        // combined = history (HIST frames) ++ chunk (n frames)
        let frames = HIST + n;
        let mut buf = Vec::with_capacity(frames * ch);
        buf.extend_from_slice(&self.history);
        buf.extend_from_slice(chunk);

        let tp = self.processed as i64;
        let last_abs = tp + n as i64 - 1; // absolute index of the last available frame
        // combined index of absolute frame `a`
        let comb = |a: i64| -> i64 { a - (tp - HIST as i64) };
        let base = |j: i64| -> usize { (j.clamp(0, frames as i64 - 1) as usize) * ch };

        while (self.cursor.floor() as i64) + 2 <= last_abs {
            let i1 = self.cursor.floor() as i64;
            let t = (self.cursor - i1 as f64) as f32;
            let j1 = comb(i1);
            let b0 = base(j1 - 1);
            let b1 = base(j1);
            let b2 = base(j1 + 1);
            let b3 = base(j1 + 2);
            for c in 0..ch {
                let (y0, y1, y2, y3) = (buf[b0 + c], buf[b1 + c], buf[b2 + c], buf[b3 + c]);
                out.push(catmull_rom(y0, y1, y2, y3, t));
            }
            self.cursor += self.step;
        }

        // keep the last HIST frames as history for the next call
        self.history.clear();
        for j in (frames - HIST)..frames {
            let b = j * ch;
            self.history.extend_from_slice(&buf[b..b + ch]);
        }
        self.processed += n as u64;
    }
}

/// Catmull-Rom cubic interpolation between y1 and y2 (t in 0..1).
fn catmull_rom(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * y1)
        + (-y0 + y2) * t
        + (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3) * t2
        + (-y0 + 3.0 * y1 - 3.0 * y2 + y3) * t3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_when_same_rate() {
        let mut r = StreamResampler::new(48000, 48000, 2);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn doubles_frame_count_44k_to_88k() {
        let mut r = StreamResampler::new(22050, 44100, 1);
        let mut out = Vec::new();
        // feed in chunks to exercise history continuity
        for _ in 0..10 {
            let chunk: Vec<f32> = (0..100).map(|i| (i as f32 * 0.01).sin()).collect();
            r.process(&chunk, &mut out);
        }
        // ~2x the input frames (± a few due to edge handling)
        assert!((out.len() as i64 - 2000).abs() < 8, "got {}", out.len());
    }

    #[test]
    fn preserves_dc_level() {
        // After the initial step transient, a constant signal stays constant
        // (no interpolation ringing on DC).
        let mut r = StreamResampler::new(44100, 48000, 1);
        let mut out = Vec::new();
        r.process(&vec![0.5f32; 4410], &mut out);
        assert!(out.iter().skip(20).all(|s| (*s - 0.5).abs() < 1e-3));
    }
}
