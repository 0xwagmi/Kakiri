//! Spike CLI.
//!
//! Spike S1 (Linux routing):
//!   spike-routing list
//!   spike-routing play --target <node.name> [--freq 440] [--ms 1000] [--amp 0.25] [--repeat 1]
//!   spike-routing mic up | down
//!
//! Spike S2 (decode + overlap mix):
//!   spike-routing file <path> [--target X] [--gain 0.8] [--loop] [--seconds 5]
//!                             [--times 2] [--gap 250]
//!
//! Live input passthrough (voice + mix):
//!   spike-routing passthrough --input <source> [--output X] [--gain 1.0] [--seconds 6]

use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use audio_core::bus::{self, BusFormat};
use audio_core::decode;
use audio_core::devices::{self, NodeInfo};
use audio_core::engine::Engine;
use audio_core::input::AudioInput;
use audio_core::output::AudioOutput;
use audio_core::tags;
#[cfg(target_os = "linux")]
use audio_core::input::PipeWireCliInput;
#[cfg(target_os = "linux")]
use audio_core::output::PipeWireOutput;

const MIC_SINK: &str = "Soundboard-Sink";
#[cfg(target_os = "linux")]
const MIC_SOURCE: &str = "Soundboard-Source";

/// Output backend: native PipeWire on Linux, cpal elsewhere.
fn make_output(target: String, fmt: BusFormat) -> Box<dyn AudioOutput> {
    #[cfg(target_os = "linux")]
    {
        Box::new(PipeWireOutput::new(target, fmt.rate, fmt.channels))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(audio_core::output::CpalOutput::new(target, fmt.rate, fmt.channels))
    }
}

/// Input backend: PipeWire CLI on Linux, cpal elsewhere.
fn make_input(target: String, fmt: BusFormat) -> Box<dyn AudioInput> {
    #[cfg(target_os = "linux")]
    {
        Box::new(PipeWireCliInput::new(target, fmt.rate, fmt.channels))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(audio_core::input::CpalInput::new(target, fmt.rate, fmt.channels))
    }
}

fn usage() {
    eprintln!(
        "spike-routing

USAGE:
  spike-routing list
  spike-routing play  --target <node.name> [--freq 440] [--ms 1000] [--amp 0.25] [--repeat 1]
  spike-routing file  <path> [--target X] [--gain 0.8] [--loop] [--seconds 5] [--times 2] [--gap 250]
  spike-routing passthrough --input <source> [--output X] [--gain 1.0] [--seconds 6]
  spike-routing native <path> [--target X] [--gain 0.8] [--seconds 4]   (pipewire-rs output)
  spike-routing meta <path>                                             (tags + cover art)
  spike-routing mic   up|down"
    );
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn cmd_list() -> Result<()> {
    println!("== cpal output devices ==");
    for d in devices::cpal_outputs()? {
        println!(
            "  [{:>2}] {:<45} {} ch, {} Hz",
            d.index, d.name, d.channels, d.rate
        );
    }
    println!("\n== PipeWire nodes (Audio/Sink | Audio/Source) ==");
    let nodes: Vec<NodeInfo> = devices::pipewire_nodes()?;
    if nodes.is_empty() {
        println!("  (none — is pipewire running?)");
    }
    for n in &nodes {
        let hint = if n.looks_like_virtual_mic() {
            "   <-- virtual mic?"
        } else {
            ""
        };
        println!("  {:<13} {:<42} {}{}", n.class, n.name, n.description, hint);
    }
    Ok(())
}

fn cmd_play(args: &[String]) -> Result<()> {
    let target = flag(args, "--target")
        .or_else(|| flag(args, "-t"))
        .unwrap_or_else(|| MIC_SINK.to_string());
    let freq: f32 = flag(args, "--freq").and_then(|v| v.parse().ok()).unwrap_or(440.0);
    let ms: u32 = flag(args, "--ms").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let amp: f32 = flag(args, "--amp").and_then(|v| v.parse().ok()).unwrap_or(0.25);
    let repeat: u32 = flag(args, "--repeat").and_then(|v| v.parse().ok()).unwrap_or(1);

    let fmt = BusFormat::default();
    println!("bus: {} Hz / {} ch  →  target node: {target}", fmt.rate, fmt.channels);
    let pcm = bus::sine(fmt, freq, ms, amp);

    let mut out = make_output(target.clone(), fmt);
    out.start()?;
    for i in 1..=repeat {
        println!("  {i}/{repeat}: {freq} Hz for {ms} ms");
        out.push(&pcm)?;
        if repeat > 1 {
            std::thread::sleep(Duration::from_millis(ms as u64 + 150));
        }
    }
    out.stop();
    println!("done.");
    Ok(())
}

fn cmd_file(args: &[String]) -> Result<()> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: spike-routing file <path> [...]"))?;
    let target = flag(args, "--target").unwrap_or_else(|| MIC_SINK.to_string());
    let gain: f32 = flag(args, "--gain").and_then(|v| v.parse().ok()).unwrap_or(0.8);
    let looped = has_flag(args, "--loop");
    let seconds: u64 = flag(args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(5);
    let times: u32 = flag(args, "--times").and_then(|v| v.parse().ok()).unwrap_or(2);
    let gap: u64 = flag(args, "--gap").and_then(|v| v.parse().ok()).unwrap_or(250);

    let fmt = BusFormat::default();
    let decoded = decode::decode_file(Path::new(&path), fmt)?;
    println!(
        "decoded {:<40} src {} Hz / {} ch → bus {} Hz / {} ch, {:.2}s, {} frames",
        Path::new(&path).file_name().unwrap_or_default().to_string_lossy(),
        decoded.src_rate,
        decoded.src_channels,
        fmt.rate,
        fmt.channels,
        decoded.duration_secs,
        decoded.pcm.len() / fmt.channels as usize,
    );

    let out = make_output(target.clone(), fmt);
    let engine = Engine::new(fmt, out);
    let pcm = Arc::new(decoded.pcm);
    let label = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sound")
        .to_string();

    print!("playing {times} overlapping copies (gap {gap} ms, gain {gain:.2}) → {target}");
    if looped {
        print!(" [loop]");
    }
    println!();
    for i in 0..times {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(gap));
        }
        let id = engine.play(label.clone(), label.clone(), pcm.clone(), gain, looped, audio_core::mixer::Route::Mic);
        println!("  voice #{id} started");
    }
    std::thread::sleep(Duration::from_secs(seconds));
    println!("  active before stop: {}", engine.active().len());
    engine.stop_all();
    std::thread::sleep(Duration::from_millis(120));
    println!("done.");
    Ok(())
}

fn cmd_passthrough(args: &[String]) -> Result<()> {
    let input = flag(args, "--input").unwrap_or_default();
    let output = flag(args, "--output").unwrap_or_else(|| MIC_SINK.to_string());
    let gain: f32 = flag(args, "--gain").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let seconds: u64 = flag(args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(6);

    let fmt = BusFormat::default();
    let out = make_output(output.clone(), fmt);
    let engine = Engine::new(fmt, out);
    engine.set_input_gain(gain);
    engine.set_input(Some(make_input(input.clone(), fmt)))?;
    println!("passthrough: {input} -> {output} (gain {gain:.2}) for {seconds}s");
    std::thread::sleep(Duration::from_secs(seconds));
    engine.set_input(None).ok();
    std::thread::sleep(Duration::from_millis(150));
    println!("done.");
    Ok(())
}

fn cmd_native(args: &[String]) -> Result<()> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: spike-routing native <path> [...]"))?;
    let target = flag(args, "--target").unwrap_or_else(|| MIC_SINK.to_string());
    let gain: f32 = flag(args, "--gain").and_then(|v| v.parse().ok()).unwrap_or(0.8);
    let seconds: u64 = flag(args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(4);

    let fmt = BusFormat::default();
    let decoded = decode::decode_file(Path::new(&path), fmt)?;
    let label = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sound")
        .to_string();

    let out = make_output(target.clone(), fmt);
    let engine = Engine::new(fmt, out);
    println!(
        "native pipewire-rs out → {target}: {} frames, {:.2}s",
        decoded.pcm.len() / fmt.channels as usize,
        decoded.duration_secs
    );
    engine.play(label.clone(), label, Arc::new(decoded.pcm), gain, false, audio_core::mixer::Route::Mic);
    std::thread::sleep(Duration::from_secs(seconds));
    engine.stop_all();
    std::thread::sleep(Duration::from_millis(200));
    println!("done.");
    Ok(())
}

fn cmd_meta(args: &[String]) -> Result<()> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: spike-routing meta <path>"))?;
    match tags::probe(Path::new(&path)) {
        Some(i) => {
            println!(
                "title={:?} artist={:?} album={:?} dur={:?} cover={}",
                i.title, i.artist, i.album, i.duration_secs, i.has_cover
            );
            if let Some(url) = tags::cover_data_url(Path::new(&path)) {
                println!("cover data URL: {} bytes", url.len());
            } else {
                println!("cover: none");
            }
        }
        None => println!("could not probe"),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn cmd_mic(action: &str) -> Result<()> {
    let _ = Command::new("pkill")
        .args(["-9", "-f", "pw-loopback.*Soundboard-Sink"])
        .stderr(Stdio::null())
        .status();
    std::thread::sleep(Duration::from_millis(400));

    match action {
        "down" => {
            println!("Soundboard mic stopped (if it was running).");
            return Ok(());
        }
        "up" => {}
        other => anyhow::bail!("unknown mic action {other:?} (use up|down)"),
    }

    let child = Command::new("pw-loopback")
        .args([
            format!("--capture-props=node.name={MIC_SINK} media.class=Audio/Sink"),
            format!(
                "--playback-props=node.name={MIC_SOURCE} media.class=Audio/Source node.description=\"Soundboard Mic\""
            ),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;
    println!("pw-loopback pid {}", child.id());
    std::thread::sleep(Duration::from_millis(700));

    let ok = devices::pipewire_nodes()?.iter().any(|n| n.name == MIC_SINK);
    if ok {
        println!("Soundboard mic up: sink '{MIC_SINK}' → source 'Soundboard Mic'.");
    } else {
        anyhow::bail!("pw-loopback started but '{MIC_SINK}' did not appear");
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(),
        Some("play") => cmd_play(&args[1..]),
        Some("file") => cmd_file(&args[1..]),
        Some("passthrough") => cmd_passthrough(&args[1..]),
        Some("native") => cmd_native(&args[1..]),
        Some("meta") => cmd_meta(&args[1..]),
        #[cfg(target_os = "linux")]
        Some("mic") => cmd_mic(args.get(1).map(String::as_str).unwrap_or("")),
        Some("help") | Some("-h") | Some("--help") | None => {
            usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown subcommand: {other}\n");
            usage();
            std::process::exit(2);
        }
    }
}
