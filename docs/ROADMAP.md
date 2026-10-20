# ROADMAP

Phases are sequential; each has acceptance criteria. Do the two **spikes**
before committing to the full build — they retire the riskiest unknowns.

---

## Phase 0 — Spikes (prove the risky bits) 🔬

- [x] **S1 Linux routing.** Minimal Rust binary: enumerate cpal devices; take a
  node name; drain a test tone to `pw-play --target <node>`. Confirm Discord
  hears it with the DSP settings from PLATFORM-AUDIO.md.
  → *ADR-4 validated: `spike-routing` + `PipeWireCliOutput`.*
  - Result: tone captured off `Soundboard-Sink` monitor at `peak=0.2500`
    (= requested `--amp 0.25`). Nodes: `Soundboard-Sink` → `Soundboard Mic`.
  - Note: `pw-record --format f32 <file>` writes a **WAV** container, not raw
    PCM — parse the `data` chunk when verifying.
  - Note: cpal on Linux only exposes ALSA aliases (`pipewire`, `default`), not
    individual PipeWire nodes → confirms routing by node name via `pw-play`.
- [x] **S2 Decode + mix.** `symphonia` decode an mp3 + a second overlapping mp3
  into a single f32 mix; no clicks, no dropouts over 2 minutes.
  → *Validates the mixer/cache design.*
  - Result: decoded `Lock-Em-Up.mp3` (48 kHz stereo, 5.06 s, 237312 frames);
    two overlapping copies captured off the mic monitor at `peak=0.68`,
    79.6% non-silent. Unit + integration tests cover decode, resample, mix,
    loop, and library scan.
- [x] **S3 Tauri hello.** `tauri build` on Linux, launch, IPC round-trip, tray
  + a global shortcut. → *Validates packaging + plugins.*
  - Result: Tauri 2 + SvelteKit scaffold builds and launches; commands
    (`list_devices`, `virtual_mic_status`, `create_virtual_mic`,
    `destroy_virtual_mic`, `play_test_tone`) compile and are registered.
    Tray + global-shortcut plugins not added yet (deferred to Phase 2).

**Exit:** all three work; ADR-4 either stays or switches to `pipewire-rs`.
(S1 stays with `pw-play`.)

---

## Phase 1 — MVP: "It plays" 🎵

- [x] Project scaffold: Tauri 2 + Svelte/TS + `src-tauri` module layout
  (ARCHITECTURE §9).
- [x] `AudioEngine` + `Mixer` + `AudioOutput` trait with `PipeWireCliOutput`
  (and `NullOutput`); `CpalOutput` still TODO.
- [x] Decode (symphonia) + resample + **streaming playback** (no whole-file
  cache; see Phase 2).
- [x] Library scan of chosen folders (mp3/wav/flac/ogg/m4a/…) + duration probe.
- [x] Folder/playlist browser UI; click a sound → play to selected device.
- [x] Stop-all + per-voice stop.
- [x] Local **Preview** (plays to real output, never to the mic).
  - Done via a second `AudioOutput` (monitor bus) + per-voice `Route`
    (`Mic` / `Monitor` / `Both`) in the mixer; monitor device is selectable.
- [x] **Live voice input** (`AudioInput`) mixed into the virtual mic, so one
  Discord input carries voice + sounds. No external `pw-link`. UI: 🎙 Mic
  toggle + source picker + level. Validated end-to-end via
  `spike-routing passthrough`.
- [x] Device / output-target picker.
- [x] Linux: virtual mic (`pw-loopback`); status indicator. Auto-created at
  launch; optional **persistent** PipeWire drop-in so Discord keeps a valid
  input device even when Kakiri is closed.
- [ ] Windows/macOS: virtual-cable detection + install guide.
- [ ] Onboarding: Discord settings checklist + **Test tone** button. (test tone
  exists; the checklist is a banner only.)

**Acceptance:** on a fresh Linux machine, from launch to "a friend hears a
clicked mp3 in Discord" in under 5 minutes, with no terminal use.

### What Phase 1 shipped

- `crates/audio-core`: `decode` (symphonia 0.6), `mixer`, `engine`, `library`.
- Tauri commands: `library_scan`, `play_sound`, `stop_voice`, `stop_all`,
  `set_voice_gain`, `active_voices`, `set_output_target`, `default_roots`.
- Folder tree sidebar (roots + subfolders, counts, add/remove), sound grid,
  cross-library search, now-playing bar with per-voice progress + stop,
  master volume, loop toggle, mic badge, output picker, test tone.
- Tests: 8 passing in `audio-core`, including real-mp3 decode and scan.

---

## Phase 2 — v1: "It's usable" 🎛️

- [x] Overlapping playback with independent gain/mute per voice. (overlap yes;
  per-voice `set_voice_gain` exists, no UI slider yet)
- [~] Loop a pad with fade in/out — loop works (global toggle); fade not yet.
- [x] Global hotkeys per pad (`tauri-plugin-global-shortcut`), best-effort on
  Wayland; in-app shortcuts always.
  - `set_hotkeys` registers accelerators from Rust and emits a `hotkey` event;
    the Pads view maps canonical accelerators → pads. On Wayland, X11 global
    grabs may not fire while a native Wayland window has focus (risk R4).
- [~] Search, categories/tabs, favorites; drag-and-drop import. (search +
  folder tabs yes; favorites/dnd not yet)
- [x] Persist library, layout, settings (localStorage: roots, prefs, pads).
- [x] **Timeline / arranger** — stack sounds as clips on 6 tracks, drag to move,
  drag between tracks, click the ruler to scrub, loop, zoom. Overlapping clips
  mix (they start normal voices), so a jingle + a bed can be layered. Clips and
  zoom persist in `localStorage`.
  - `audio-core::timeline` runs a 4 ms scheduler over the existing mixer — no
    second audio path. `play_timeline(clips, looped, from)` supports scrubbing;
    a clip spanning the playhead resumes mid-file via `spawn_from`.
  - **Editable while it plays**: the arrangement lives behind a mutex, so
    adding/removing a clip or toggling loop does not restart playback —
    `timeline_add_clip` / `timeline_remove_clip` / `timeline_set_loop`. Adding
    a clip whose start is behind the playhead starts it part-way in; removing
    one silences only that clip's voice.
  - Verified with synthetic tones: 1 s clip → silence gap → two overlapping
    clips (louder sum), loop wrap at the timeline length, and `from` resuming
    two clips mid-way. Mid-play add measured off the mic: 440 Hz clip keeps
    playing across the add while 660 Hz joins (RMS 0.35 → 0.50, no dropout);
    removing the added clip drops the voice count back to 1. A clip added at
    start 3 s extended a 4 s timeline to 5 s and still ended correctly.
- [ ] Tray menu: mute all, volume, quit; single-instance.
- [ ] VU/peak meters (throttled `levels` events).
- [x] Long-file streaming path (don't preload whole songs).
  - Per-voice decoder thread + lock-free SPSC ring (`stream.rs`); stateful
    Catmull-Rom resampler (`resample.rs`) continuous across packets.
    Start latency **6.16 s → 8 ms** on a 4½-min m4a; memory ~0.8 MB/voice
    (2 s ring) instead of ~104 MB. 0 dropouts (verified off the mic monitor).
- [ ] Basic error surfacing (missing file, dead device → reconnect).

**Acceptance:** daily-driver-able; a streamer could run an ambience loop and
fire memes with hotkeys without opening the window.

---

## Phase 3 — Later: "It's powerful" 🚀

- [ ] URL/stream playback via bundled `yt-dlp` sidecar (US-15).
- [ ] Capture another app's audio into the mic, cross-platform (US-16) — the
  cross-platform `song_to_mic.py`.
- [ ] Multi-output (mic + monitor + OBS) (US-17).
- [ ] Record the mix bus to file (US-18).
- [ ] MIDI pad controller / OSC (US-19).
- [ ] Waveform view + trim/normalize (US-20).
- [ ] Code signing / notarization; auto-updater.

---

## Distribution checklist (per release)

- [ ] Linux: `.deb` + AppImage; test on a clean Ubuntu with PipeWire.
- [ ] Windows: `.msi`/NSIS; test on a machine with VB-CABLE.
- [ ] macOS: universal `.dmg`; test with BlackHole; notarize (later).
- [ ] Docs updated: PLATFORM-AUDIO.md, README quickstart.
