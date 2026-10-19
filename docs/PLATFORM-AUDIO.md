# PLATFORM AUDIO & DISCORD SETUP

The app plays the mix into a **virtual microphone**. How that mic exists
differs per OS. This is the part users get stuck on, so onboarding matters.

---

## Linux (PipeWire)

The app can create the virtual mic itself — no user install.

Create a sink whose monitor is exposed as a source:

```bash
pw-loopback \
  --capture-props='node.name=Soundboard-Sink media.class=Audio/Sink' \
  --playback-props='node.name=Soundboard-Source media.class=Audio/Source node.description="Soundboard Mic"'
```

- The app plays the mix into the **`Soundboard-Sink`** node
  (`pw-play --target Soundboard-Sink`).
- Discord input = **"Soundboard Mic"** (`Soundboard-Source`).
- Lifecycle: not persistent — dies on reboot/logout. The app starts it on
  launch (and on demand) and kills it on exit. Same model as the existing
  `recreate_ai_mic.sh`.
- Reuse option: the existing `AI-Mic-Sink` / `AI Mic` pair from the Python
  project can be selected instead.

Prereqs: a running PipeWire session (`pipewire`, `wireplumber`,
`pipewire-pulse` all standard on Ubuntu 22.10+/Fedora 34+). `pw-play`,
`pw-loopback`, `pw-dump`, `wpctl` come with `pipewire-bin`.

**Known sharp edge:** `cpal`/PortAudio speak ALSA on Linux and can't easily
target one PipeWire node by name → we route via `pw-play --target` (ADR-4).

---

## Windows

The app **cannot** install a virtual cable (kernel driver, licensing). User
installs one:

| Option | Cost | Devices | Notes |
|--------|------|---------|-------|
| **VB-Audio VB-CABLE** | Free (donationware) | Play to `CABLE Input`, Discord records `CABLE Output` | Simplest; recommended |
| **VB-Audio VoiceMeeter** | Free | Virtual inputs | Overkill but flexible mixing |
| **Virtual Audio Cable (VAC)** | Paid | `Line 1` | Reliable, paid |
| (built-in) "Stereo Mix" | Free | — | Only if sound card exposes it; often missing |

App detects by matching output device names: `CABLE Input`, `VoiceMeeter
Input`, `Line 1`.

Steps shown to the user:

1. Install VB-CABLE, **reboot** (driver install).
2. Sound settings → set `CABLE Input` as a playback device (do **not** set it
   as default).
3. In the app, pick `CABLE Input` as the output device.
4. Discord → User Settings → Voice & Video → **Input Device = `CABLE Output`**.

---

## macOS

| Option | Cost | Notes |
|--------|------|-------|
| **BlackHole 2ch** | Free (open source) | Recommended. `brew install blackhole-2ch` |
| BlackHole 16ch | Free | For multi-channel routing |
| Loopback (Rogue Amoeba) | Paid | GUI routing, very capable |
| Soundflower | Free | Legacy, unreliable on modern macOS |

App detects `BlackHole 2ch` / `BlackHole 16ch` / `Loopback Audio`.

Steps:

1. `brew install blackhole-2ch` (or download the installer).
2. In the app, pick `BlackHole 2ch` as output.
3. Discord → Voice & Video → **Input Device = `BlackHole 2ch`**.
4. To *also* hear it yourself: **Audio MIDI Setup → Create Multi-Output
   Device** containing BlackHole + your headphones. Or use the app's built-in
   **Monitor** (which feeds your headphones separately without going through
   BlackHole) — simpler and avoids echo.

macOS may ask for **Microphone permission** for the app (Discord recording the
virtual device is not a mic prompt from us, but BlackHole install can trigger
privacy prompts). Grant when asked.

---

## Discord settings that WILL break your sound

Discord applies speech DSP that assumes a human voice. It chews up music and
sound effects. In **User Settings → Voice & Video**:

| Setting | Set to | Why |
|---------|--------|-----|
| Input Device | your virtual mic | so it hears the app, not your real mic |
| **Noise Suppression** (Krisp) | **OFF** | removes music as "noise"; the #1 culprit |
| **Echo Cancellation** | **OFF** | cancels the virtual mic against output |
| **Automatic Gain Control** | **OFF** | pumps/ducks volume |
| **Voice Isolation** (if present) | **OFF** | same as noise suppression |
| Advanced Voice Activity / sensitivity | manual, low threshold | so quiet clips aren't gated |
| Input Volume | ~100% | |
| Output Device | your real headphones | not the virtual cable |

> Side effect of disabling AEC: if you *also* use your real mic at the same
> time, Discord may echo. This is why the app defaults **Monitor = off** and
> recommends the virtual mic as the *only* input in that call, or a headset.

The app's onboarding screen should present exactly this checklist and offer a
**Test tone** button ("play test tone → confirm you hear it in Discord's Mic
Test").

---

## Other apps

- **OBS:** add an "Application Audio Capture" or a second `AudioOutput` device
  (Later feature, US-17).
- **VoiceMeeter users:** route into a VoiceMeeter virtual input and configure
  there; the app just needs to emit to that device.
