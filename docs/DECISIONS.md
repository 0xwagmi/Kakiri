# DECISIONS (ADR log)

Lightweight Architecture Decision Records. Status: proposed / accepted /
superseded.

---

## ADR-1 — Language: Rust ✅ proposed

**Context.** Build a cross-platform, low-latency desktop audio app that must be
easy to distribute to non-developers.

**Options**

| Option | Pros | Cons |
|--------|------|------|
| Python (reuse existing repo) | Fastest to prototype; `pw-play` path already proven; `soundfile` already decodes mp3 | Distribution needs a bundled interpreter (PyInstaller/Briefcase); PortAudio wheel here is ALSA-only (no Pulse host API) so Linux device targeting is bad; global hotkeys/tray/UI need extra libs |
| **Rust** | Single static-ish binary; pure-Rust decode (`symphonia`); real-time audio (`cpal`/`rodio`); Tauri gives native UI + plugins (hotkeys, tray, single-instance, updater); great packaging | Steeper; smaller ecosystem; Linux/PipeWire device targeting has sharp edges |
| C++ / Qt | Mature audio + GUI | Build/packaging pain; slow iteration |
| Electron + node | Rich UI, easy | Huge binaries; native audio needs native modules anyway |

**Decision:** Rust. The user explicitly wants Rust; it matches the "ship a real
cross-platform app" goal.

---

## ADR-2 — UI: Tauri 2 web frontend ✅ proposed

**Context.** Need pads, drag-drop, search, settings, onboarding.

**Options**

| Option | Pros | Cons |
|--------|------|------|
| **Tauri 2** (Rust + HTML/CSS/JS) | Beautiful, flexible UI; tiny vs Electron; first-party plugins: `global-shortcut`, `dialog`, `fs`, `store`, `tray`, `single-instance`, `updater`; reuse web skills | Depends on system webview (WebView2/WKWebView/WebKitGTK); Linux packaging needs webkit2gtk |
| `egui` | Pure Rust, tiny, instant iterations, great for tools | Less polished; audio-reactive fancy layouts are fiddly |
| `iced` | Pure Rust, Elm-style | Smaller ecosystem; slower to build rich lists/drag-drop |
| Native (GTK/Cocoa/WinUI) | Best platform fit | Three UIs to maintain |

**Decision:** Tauri 2 with a small frontend (recommend **Svelte + TypeScript +
Vite**, or vanilla TS if we want zero framework). Backend = Rust audio engine.

**Note:** keep DSP out of the webview. The frontend only sends commands and
receives events; all audio runs in Rust.

---

## ADR-3 — Decode: `symphonia` (pure Rust), not ffmpeg ✅ proposed

**Context.** Must decode mp3 at minimum; ideally m4a/aac, flac, ogg, wav.

- `symphonia` — pure Rust; supports mp3, aac (incl. isomp4/m4a), flac, vorbis,
  wav, alac. No system deps. **Chosen.**
- `ffmpeg` shell-out — broadest format support but an external binary to
  bundle per OS; heavier.
- `miniaudio` — great but no aac/m4a.

**Decision:** `symphonia` for decoding. Offer optional ffmpeg fallback later
only if a needed codec is missing (e.g. opus-in-webm from streams).

---

## ADR-4 — Linux output routing: mix bus → `pw-play --target` (v1) ✅ proposed

**Context.** The app must send the mix to a *specific* PipeWire node (the
virtual mic). `cpal` speaks ALSA on Linux; targeting one node by name requires
the `PIPEWIRE_NODE` env var (process-global) or the `pipewire` ALSA plugin
default. PortAudio/sounddevice has the same problem.

**Options**

| Option | Pros | Cons |
|--------|------|------|
| **A. Custom mix bus → `pw-play --target <node>`** | Proven in `speak.py`; dynamic target switching; simple | External `pw-play` (already standard on PipeWire systems); a pipe hop |
| B. `cpal` straight to a device | One code path for all OSes | On Linux can't reliably pick a node by name dynamically |
| C. `pipewire-rs` native stream with `node.target` | Correct, no subprocess | More code/FFI; Linux-only path anyway |

**Decision:** Define an `AudioOutput` trait and use a custom **mix bus** (our
own mixer produces interleaved f32 PCM):
- **Windows/macOS:** `CpalOutput` — enumerate devices by name (WASAPI /
  CoreAudio), stream the mix bus.
- **Linux:** `PipeWireCliOutput` — drain the mix bus to
  `pw-play --target <node.name> --rate … --format f32 -`.
- Monitor = a second `AudioOutput` instance fed the same bus.

This keeps one mixer, supports dynamic device switching, and sidesteps the
`cpal`-on-ALSA targeting problem. If `pipewire-rs` proves painless later, ADR-4
can move to option C behind the same trait.

**Status: implemented (option A → C).** Shipped `pw-play`/`pw-record` first
(ADR-4 option A) to prove the routing, then added native `pipewire` streams in
`output/pw_native.rs` + `input/pw_native.rs` (option C). Native is now the Linux
default; the CLI backends remain behind `SOUNDBOARD_PW_BACKEND=cli`.

Latency note: `pw-cat` defaults to a **100 ms** node buffer; the native streams
run at the graph quantum, and the CLI path got `--latency 40ms` + a 512-frame
block as a fallback.

Native gotcha (fixed): a `pw_stream` without a `SPA_PARAM_Buffers` param defaults
to a huge buffer (16384 frames = 341 ms here) — the process callback then only
refills every 341 ms, which both lags and starves our ring (audible stutter).
Both native streams now pin the buffer to **1024 frames** via `SPA_PARAM_Buffers`,
so the callback runs every ~21 ms and playback is smooth (verified: 0 dropouts).

---

## ADR-5 — Virtual mic: create on Linux, detect+guide elsewhere ✅ proposed

**Context.** Discord needs a *recording* device that carries our audio.

- **Linux/PipeWire:** we can create it ourselves with `pw-loopback`
  (as `recreate_ai_mic.sh` does). App offers "Create virtual mic", manages the
  process, cleans up on exit.
- **Windows:** VB-CABLE / VoiceMeeter must be installed by the user. Detect by
  name; link to installer.
- **macOS:** BlackHole must be installed by the user (`brew install
  blackhole-2ch`). Detect by name; link to instructions.

**Decision:** `VirtualMic` abstraction with per-OS detection + onboarding.
Never silently install anything. Provide a "Test" button that plays a tone and
tells the user which device to select in Discord.

---

## ADR-6 — Naming: dedicated `Soundboard` nodes on Linux ✅ proposed

Reuse `AI Mic`? Pros: Discord is already pointed at it. Cons: it's owned by the
Python pipeline; two things writing to it is confusing.

**Decision:** create a dedicated pair (`Soundboard-Sink` → `Soundboard Mic`)
by default, but allow selecting *any* output device — including the existing
`AI-Mic-Sink` — from the device picker. Don't fight the user's setup.

---

## ADR-7 — Persistence: Tauri `store` plugin / JSON ✅ proposed

Library index, pad layout, settings, hotkeys → a single JSON (or SQLite later
if libraries get huge). Start with `tauri-plugin-store` + a `library.json`;
revisit with `rusqlite` if scanning tens of thousands of files.
