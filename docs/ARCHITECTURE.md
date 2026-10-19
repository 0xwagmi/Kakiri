# ARCHITECTURE

## 1. High-level shape

```
┌─────────────────────────────────────────────────────────────┐
│ Tauri app                                                    │
│                                                              │
│  ┌───────────────┐        commands (invoke)                 │
│  │ Frontend      │ ───────────────────────────┐              │
│  │ (Svelte/TS)   │ <─────────────────────────┐ │ events       │
│  │ pads, search, │                            │ │ (levels,     │
│  │ settings      │                            │ │  state)      │
│  └───────────────┘                            ▼ │              │
│                                        ┌──────────────┐       │
│                                        │ Rust core    │       │
│                                        │ (this crate) │       │
│  ┌───────────────┐   ┌─────────────┐   │              │       │
│  │ Library       │   │ Hotkeys /   │   │  AudioEngine │       │
│  │ (scan, tags)  │   │ tray / cfg  │   │  ┌────────┐  │       │
│  └──────┬────────┘   └─────────────┘   │  │ Mixer  │  │       │
│         │                              │  └───┬────┘  │       │
│         │ decode (symphonia)            │      │ PCM   │       │
│         ▼                              │      ▼       │       │
│  ┌───────────────┐                     │ AudioOutput  │       │
│  │ Decoded cache │────────────────────▶│  trait       │       │
│  └───────────────┘                     └──────┬───────┘       │
└───────────────────────────────────────────────┼──────────────┘
                                                 │
              ┌──────────────────────────────────┼───────────────────────┐
              │ Linux                            │ Windows/macOS         │
              ▼                                  ▼                       │
      pw-play --target <node>          cpal device by name (WASAPI/       │
        └─▶ Soundboard-Sink              CoreAudio) ─▶ CABLE Input /      │
              └─▶ "Soundboard Mic"                       BlackHole 2ch   │
                    └─▶ Discord input                                     │
                                                                          │
   Monitor path (optional): same mix bus ─▶ your real headphones ─────────┘
```

## 2. Crates

| Concern            | Crate | Notes |
|--------------------|-------|-------|
| App shell + UI     | `tauri` 2, `tauri-plugin-{global-shortcut,dialog,fs,store,single-instance,tray,opener}` | Frontend built with Vite |
| Decode             | `symphonia` (features: mp3, aac, isomp4, flac, vorbis, wav, alac) | Pure Rust, no ffmpeg |
| Device I/O         | `cpal` | WASAPI / CoreAudio / ALSA enumeration + streams |
| Mixing (optional)  | `rodio` | If we lean on it for sinks; otherwise our own mixer |
| Resample           | `rubato` (or `dasp`/`rodio` adapters) | Only if we don't run everything at the device rate |
| Tags / metadata    | `lofty` | title/artist/album/cover |
| Serialization      | `serde`, `serde_json` | config, library index, IPC |
| Errors/logging     | `anyhow`/`thiserror`, `tracing`, `tracing-subscriber` | |
| Linear resampling for waveform | `symphonia` + simple peak reducer | |

## 3. Audio pipeline

```
trigger(pad)
   │
   ├─ cache hit? ──yes──▶ Voice { pcm: Arc<[f32]>, pos, gain, loop }
   │
   └─ no ─▶ decode(symphonia) ─▶ resample to bus rate ─▶ store in cache
                                   │
                                   ▼
                            Voice added to Mixer
                                   │
   audio callback (real-time):     ▼
   for each Voice: out[i] += pcm[pos+i] * gain * fade
   sum → soft clip / limiter → interleaved f32 buffer
                                   │
              ┌────────────────────┴────────────────────┐
              ▼                                          ▼
        Primary AudioOutput                        Monitor AudioOutput
        (virtual mic)                              (your headphones)
```

Design rules:

- **Real-time thread never allocates or locks.** Voices are passed in via a
  lock-free channel (e.g. `rtrb`/`crossbeam`); the callback only mixes.
- **Everything runs at one bus sample rate** (default 48 kHz, configurable).
  Files are decoded/resampled once and cached.
- **Hot pads are pre-decoded**; long files (music, streams) are decoded
  incrementally on a worker thread and fed as chunks, so a 5-minute song
  doesn't block.
- **Monitor is a second sink of the same buffer** — never re-mic'd, so no echo
  into the call.
- **The live voice input is summed into the mic bus** by the engine (from a
  bounded ring buffer), so the app emits a single stream containing voice +
  sounds. It is deliberately *not* added to the monitor bus.

### AudioOutput trait (ADR-4)

```rust
pub trait AudioOutput: Send {
    fn start(&mut self, bus_rate: u32, bus_channels: u16) -> anyhow::Result<()>;
    fn push(&mut self, interleaved: &[f32]) -> anyhow::Result<()>; // non-blocking-ish
    fn stop(&mut self);
    fn latency_hint(&self) -> Duration;
}
```

Implementations: `CpalOutput` (Win/mac/Linux default), `PipeWireCliOutput`
(Linux, `pw-play --target`), `NullOutput` (tests). Device switching = stop old,
start new, keep the same upstream mixer.

### AudioInput trait (live voice passthrough)

So that **one Discord input carries your voice _and_ the sounds**, the engine
captures the chosen voice source itself and mixes it in — no external `pw-link`
wiring.

```rust
pub trait AudioInput: Send {
    fn start(&mut self) -> anyhow::Result<()>;
    /// Non-blocking: returns 0 when no data is ready; Err = stream ended.
    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize>;
    fn stop(&mut self);
}
```

Implementations: `PipeWireCliInput` (Linux, `pw-record --target` with `O_NONBLOCK`
so the reader thread stays cancellable), `NullInput`. On Windows/macOS this
becomes a `cpal` input. The engine runs one `audio-input` reader thread that
fills a bounded ring `InputRing`; the writer thread blends the ring into the
**mic bus only** (never the monitor, which would echo) before the soft limiter.

Why this shape: the app is the single source of truth for the mix, so voice
passthrough is identical on every OS, composes with Easy Effects (you just pick
`Easy Effects Source` as the input), and unlocks ducking / per-source gain /
meters later. `pw-link` graph surgery is never required.

## 4. Library

- Scan user-chosen folders for `mp3, wav, flac, ogg, m4a, aac, opus`.
- Read tags with `lofty`; fall back to filename.
- Store an index (`library.json` initially): path, title, artist, duration,
  tags, favorite, hash (for move detection).
- Cache directory: decoded PCM for hot pads + waveform peaks + cover art.
- Watch folders (optional) with `notify` for live updates.

## 5. Frontend ↔ core contract (draft)

Commands (frontend → Rust):

```
library_scan(paths) -> Library
library_list(query, tag, favorite) -> [Sound]
pad_play(sound_id, {gain, loop, device?}) -> VoiceId
pad_stop(voice_id) | stop_all()
preview_play(sound_id)        // local only, not to mic
devices_list() -> {outputs, virtual_mics, selected}
virtual_mic_status() -> {present, name, kind, can_create}
virtual_mic_create() | virtual_mic_destroy()   // Linux only
settings_get() | settings_set(patch)
```

Events (Rust → frontend):

```
playback_state(voice_id, state)   // started/stopped/looped
levels(peak_l, peak_r)            // for VU meters, throttled
device_changed()
error(message)
```

## 6. Threading

| Thread | Responsibility |
|--------|----------------|
| RT audio callback (cpal or pw-play writer) | mix voices → output; no alloc/lock/log |
| Decode pool (`rayon`/`tokio::task`) | symphonia decode + resample + cache |
| Library scanner | walks dirs, reads tags, hashes |
| Tauri main / async runtime | IPC, window, tray, hotkeys, config |
| PipeWire child stdout (Linux) | blocking writer to `pw-play` stdin |

## 7. Onboarding / setup detection

`virtual_mic_status()` implements per-OS detection:

- **Linux:** scan `pipewire` nodes for a sink→source loopback pair. Offer
  "Create" (spawns `pw-loopback`, tracked, killed on exit / on demand).
- **Windows:** enumerate outputs; match `CABLE Input`, `VoiceMeeter Input`,
  `Line 1 (Virtual Audio Cable)`.
- **macOS:** enumerate outputs; match `BlackHole 2ch`, `BlackHole 16ch`,
  `Loopback Audio`, `Soundflower`.

Then a **Discord checklist** screen (see PLATFORM-AUDIO.md) — the app cannot
change Discord's settings, so it instructs and offers a test tone.

## 8. Packaging / CI

- `tauri build` per OS.
- GitHub Actions matrix: `ubuntu-latest` (AppImage/deb), `windows-latest`
  (msi/nsis), `macos-latest` (dmg, both arches or universal).
- Code signing / notarization is a later concern; docs note unsigned-build
  warnings.
- No bundled virtual cable (licensing + OS drivers) — detect + guide only.

## 9. Suggested crate layout

```
src-tauri/
  src/
    main.rs
    audio/        mod.rs, engine.rs, mixer.rs, output/{mod,cpal,pw_cli,null}.rs, decode.rs, resample.rs
    library/      mod.rs, scan.rs, tags.rs, index.rs
    platform/     mod.rs, virtual_mic/{linux,windows,macos}.rs, devices.rs
    state.rs      # Tauri managed state, commands
    config.rs
  tauri.conf.json
src/            # frontend (Svelte/TS)
  routes/ , lib/pads/ , lib/settings/ , lib/api.ts
```
