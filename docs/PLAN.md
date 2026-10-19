# PLAN

## 1. Problem

Today, playing a sound into a Discord call means: create a virtual mic, set it
as Discord's input, then get a file into it. The existing Python flow works for
one song at a time (`song_to_mic.py`, `speak.py`) but:

- It routes a **browser** stream, not files you pick.
- It's Linux-only (`pw-link` / `pw-play` / `pw-loopback`).
- There is no library, no UI, no simultaneous sounds, no per-sound control.

We want a **desktop app**: pick sounds from a UI, hit a pad, and the call hears
it — on Linux, Windows, and macOS.

## 2. Goals

1. **One-click play to the call.** Click a pad → audio goes out the virtual mic,
   not your speakers only.
2. **Cross-platform.** Windows, macOS, Linux from one Rust codebase.
3. **Local library.** Import/scan folders of audio; search, tag, favorites.
4. **Simultaneous + controllable.** Overlapping pads, per-pad volume, stop,
   fade, loop.
5. **Monitor toggle.** Optionally *also* hear what you're sending (to your real
   headset) without it looping back into the mic.
6. **Zero-surprise audio.** Correct device targeting, sane latency (<150 ms
   trigger-to-sound on the virtual mic).
7. **Easy setup.** Detect the virtual cable, or on Linux create it; show a
   Discord settings checklist that actually prevents Discord from mangling the
   audio.

## 3. Non-goals (v1)

- Not a DAW: no multi-track editing, effects chains, or pitch/tempo.
- Not a hosted service; no accounts, no cloud sync.
- Not a Discord bot / server-side audio injection.
- No DRM'd service ripping (Spotify/Apple Music). The user supplies files.
- No mobile.

## 4. Personas

- **P (primary):** Discord user on a laptop, plays memes/clips/sound effects
  into hangouts. Wants hotkeys while a game has focus.
- **S (secondary):** Streamer/tabletop GM who needs ambience/music beds on a
  loop with volume control.
- **D (developer/this user):** already has a virtual mic + a working pattern,
  wants a proper UI and cross-platform distribution.

## 5. User stories

**MVP**
- US-1  As P, I can point the app at a folder and see every sound as a pad.
- US-2  As P, I can click a pad and the sound plays into the call.
- US-3  As P, I can pick which output device (virtual mic) to send to, and it's
  remembered.
- US-4  As P, I can stop ALL playback instantly.
- US-5  As P, I can preview a sound locally without sending it.
- US-6  As P, I can see whether a supported virtual cable is installed and get
  setup instructions if not.
- US-7  As P, I get a checklist of Discord settings to disable (noise
  suppression etc.) so my sound isn't eaten.
- US-8  As P, I hear nothing weird on my own side: monitor is off by default,
  and my real mic is never echoed into the call.

**v1**
- US-9  Multiple pads overlap; each has its own volume and mute.
- US-10 Global hotkeys per pad (works while another app has focus).
- US-11 Loop a pad (ambience/music bed) with fade in/out.
- US-12 Search + categories/tabs + favorites.
- US-13 Drag-and-drop files onto the window to import.
- US-14 Persisted library, pad layout, and settings.

**Later**
- US-15 Play a URL/stream (YouTube etc.) through the mic via a bundled
  `yt-dlp` sidecar.
- US-16 Route *another app's* audio (browser, game) into the mic (the
  `song_to_mic.py` idea, cross-platform).
- US-17 Output to multiple destination devices at once (mic + monitor + OBS).
- US-18 Record the mixed bus to a file.
- US-19 MIDI pad controller / OSC triggers.
- US-20 Waveform view + trim/normalize.

## 6. Feature tiers (summary)

| Tier  | Theme            | Contents                                             |
|-------|------------------|------------------------------------------------------|
| MVP   | "It plays"       | library scan, pads, play-to-device, stop-all, device picker, cable detect+guide, Discord checklist, local preview |
| v1    | "It's usable"    | overlap, per-pad gain, loop, hotkeys, search/tags, dnd import, persistence, tray |
| Later | "It's powerful"  | URL/stream, app-capture, multi-output, record, MIDI, waveform/trim |

## 7. Key risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Linux device targeting: `cpal` uses ALSA and picking one PipeWire node by name is awkward; `PIPEWIRE_NODE` is process-global | Backend trait: on Linux drain the mix bus to `pw-play --target <node>`; or use `pipewire-rs`; document it (see DECISIONS ADR-4) |
| R2 | Windows/macOS virtual cable isn't installable by us | Detect + guide; ship links; make "no cable" a friendly onboarding state |
| R3 | Discord DSP destroys music (Krisp noise suppression, AEC, AGC) | First-run Discord checklist; verify with a test tone; docs |
| R4 | Global hotkeys on Linux Wayland are restricted | In-app shortcuts always work; global hotkeys best-effort via X11/portal; document limitation |
| R5 | Tauri Linux webview packaging (webkit2gtk, AppImage libs) | CI matrix build; keep frontend dependency-light |
| R6 | Latency from decode-on-trigger | Pre-decode hot pads / cache decoded PCM; stream long files |

## 8. Open questions (need answers before coding)

1. **UI toolkit:** Tauri (web frontend) vs pure-Rust `egui`/`iced`? (Recommend
   Tauri — see ADR-2.)
2. **"Play online":** for MVP, local files only, or is URL/stream playback
   (YouTube etc.) a must-have?
3. **OS priority:** build/dev on Linux first, but which of Windows/macOS must
   ship in the first release? (Affects CI + how hard we fight `cpal` device
   targeting.)
4. **Global hotkeys:** required for MVP or fine in v1?
5. **Virtual mic naming:** reuse the existing `AI Mic` / `AI-Mic-Sink` nodes, or
   create a dedicated `Soundboard Mic`? (Reusing means Discord input is already
   configured.)
6. **Repo layout:** is `/home/user/tool/discord-soundboard` the right home, or
   should this live as a subfolder of the Python repo?
