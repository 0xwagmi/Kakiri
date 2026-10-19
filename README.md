<p align="center">
  <img src="static/icon.png" width="140" height="140" alt="Kakiri Soundboard" />
</p>

<h1 align="center">Kakiri Soundboard</h1>

<p align="center"><em>Play sounds through a virtual microphone in Discord — with your voice.</em></p>

<p align="center">
  <img alt="status" src="https://img.shields.io/badge/status-early%20alpha-orange" />
  <img alt="platform" src="https://img.shields.io/badge/audio-Linux%20(PipeWire)-informational" />
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue" />
</p>

Cross-platform desktop soundboard that plays audio files (and later, streams)
**through a virtual microphone** so other people in Discord hear it as if it
were your voice.

> **Status: early alpha — Linux-only audio.** The app is fully functional on
> **Linux / PipeWire** (native `pipewire-rs`). Windows and macOS *build and
> launch* but have **no audio backend yet** (`cpal` output/input is TODO).
> Not production-ready. See [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Platform support

| Platform | Builds / runs | Audio | Virtual mic |
|----------|:-------------:|:-----:|-------------|
| **Linux** (PipeWire) | ✅ | ✅ native `pipewire-rs` (+ `pw-play`/`pw-record` fallback) | ✅ auto (`pw-loopback`) |
| **Windows** | ✅ | ❌ no backend yet (`cpal` TODO) | manual: VB-CABLE |
| **macOS** | ✅ | ❌ no backend yet (`cpal` TODO) | manual: BlackHole |

## Features

- Folder / playlist browser with search, durations, and a now-playing bar
- Play into a virtual mic; **monitor + per-sound preview**
- 16-pad grid with **global hotkeys**; retrigger modes **Layer / Choke / Toggle**
- **Live voice input** mixed into the virtual mic (talk *and* play sounds)
- Decodes mp3 / m4a / flac / ogg / wav + more via `symphonia` (no ffmpeg)

## Why Rust

- Single self-contained binary per OS — no Python runtime, no ffmpeg, no
  `pw-play` shell-outs required for decoding (`symphonia` is pure Rust).
- Low-latency, real-time-friendly audio (`cpal` + `rodio`).
- Native-looking UI via **Tauri 2** (Rust core + web frontend) with a small
  download; global hotkeys, tray, single-instance all have first-class plugins.
- Cross-platform packaging (`tauri build` → `.msi`, `.dmg`, `.AppImage`/`.deb`).

## The one hard problem

Every OS needs a **virtual microphone** to feed Discord. The app cannot
magically create one everywhere:

| OS      | Virtual cable                      | Who installs it        |
|---------|------------------------------------|------------------------|
| Linux   | PipeWire `pw-loopback`             | **App can create it**  |
| Windows | VB-Audio VB-CABLE / VoiceMeeter    | User (free download)   |
| macOS   | BlackHole 2ch                      | User (`brew install`)  |

The app detects it, guides setup, and targets it. Details in
[`docs/PLATFORM-AUDIO.md`](docs/PLATFORM-AUDIO.md).

## Folder map

```
README.md              ← you are here
Cargo.toml             cargo workspace (audio-core + src-tauri)
crates/audio-core/     the real-time audio engine (Rust lib)
  src/bus.rs             mix-bus format + test-tone generator
  src/decode.rs          symphonia decode + channel-map + linear resample
  src/mixer.rs           multi-voice mixer with soft limiter
  src/engine.rs          engine + writer/reader threads; mic + monitor + live input
  src/input/             AudioInput trait + pw-record backend (voice passthrough)
  src/library.rs         folder tree scan + duration probe
  src/devices.rs         cpal device enum + PipeWire node listing
  src/output/            AudioOutput trait + pw-play backend (ADR-4)
  src/bin/spike-routing.rs   Spike CLI (list / play / file / passthrough / mic)
src-tauri/             Tauri backend (IPC commands)
  src/lib.rs             devices, virtual mic, library_scan, play_sound, voices
src/                   SvelteKit frontend
  routes/+page.svelte    top bar, folder/playlist browser, view switch, now-playing
  lib/SoundCard.svelte   sound tile (click = to call, 🎧 = local preview)
  lib/Pads.svelte        16-pad grid, edit mode, global hotkeys
docs/
  PLAN.md              goals, non-goals, user stories, feature tiers, open questions
  ARCHITECTURE.md      components, crates, audio pipeline, threading, packaging
  DECISIONS.md         ADR log (language, UI, audio engine, routing strategy)
  PLATFORM-AUDIO.md    per-OS virtual cable setup + Discord settings gotchas
  ROADMAP.md           phased milestones with acceptance criteria
  UI.md                wireframes / screen inventory
```

## Quickstart (dev, Linux)

```bash
npm install
npm run tauri dev          # builds + launches the app
```

The backend can also be exercised headlessly with the spike CLI:

```bash
cargo run -p audio-core --bin spike-routing -- list
cargo run -p audio-core --bin spike-routing -- mic up
cargo run -p audio-core --bin spike-routing -- play --target Soundboard-Sink --repeat 2
```

What Spike S1 proved: a generated 440 Hz tone written through
`AudioOutput`/`PipeWireCliOutput` was captured back off `Soundboard-Sink`'s
monitor at exactly `peak = 0.25` (the requested amplitude), and `pw-loopback`
exposed `Soundboard Mic` as a Discord-recordable source.

## Relationship to the existing project
The Python `../voice_to_caption_translator/` repo proved out the pieces this
app reuses conceptually:

- `recreate_ai_mic.sh` — how to make a PipeWire virtual mic (`pw-loopback`).
- `speak.py` — the "play PCM into a named sink" pattern (`pw-play --target`).
- `song_to_mic.py` — the "route *other* app audio into the mic" pattern
  (later feature: capture an app/stream, not just files).

This project is a **clean-room Rust rewrite/expansion**, not a port. The Python
repo keeps working; nothing here imports from it.

## License

MIT © 0xwagmi — see [LICENSE](LICENSE).
