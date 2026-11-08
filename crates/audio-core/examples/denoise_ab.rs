//! Measure RNNoise on real captured audio: raw f32le mono 48 kHz on stdin.
//!
//!   ffmpeg -i room.wav -f f32le -ac 1 -ar 48000 - \
//!     | cargo run -p audio-core --example denoise_ab
//!
//! Prints the RMS of the input and of the denoised output, in dBFS, so "does it
//! actually remove the hiss" has a number instead of an opinion.
use std::io::Read;

fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum / samples.len() as f32).sqrt();
    if rms <= 1e-9 {
        -120.0
    } else {
        20.0 * rms.log10()
    }
}

fn main() {
    let mut raw = Vec::new();
    std::io::stdin().read_to_end(&mut raw).expect("read stdin");
    let input: Vec<f32> = raw
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if input.is_empty() {
        eprintln!("no samples on stdin");
        std::process::exit(2);
    }

    let mut d = audio_core::denoise::Denoiser::new();
    let mut out = Vec::with_capacity(input.len());
    for block in input.chunks(1024) {
        let mut buf = block.to_vec();
        d.process(&mut buf, 1);
        out.extend_from_slice(&buf);
    }

    // Skip the first frame: it is the queue filling, not signal.
    let skip = 480.min(input.len());
    println!("  samples: {}", input.len());
    println!("  input   : {:.1} dBFS", rms_db(&input[skip..]));
    println!("  denoised: {:.1} dBFS", rms_db(&out[skip..]));
    println!("  change  : {:.1} dB", rms_db(&out[skip..]) - rms_db(&input[skip..]));
}
