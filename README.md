<p align="center">
  <img src="static/icon.png" width="140" height="140" alt="Wagmify" />
</p>

<h1 align="center">Wagmify</h1>

<p align="center"><em>Soundboard and music player that Discord hears as your microphone.</em></p>

<p align="center">
  <img alt="status" src="https://img.shields.io/badge/status-early%20alpha-orange" />
  <img alt="audio" src="https://img.shields.io/badge/audio-Linux%20(PipeWire)-informational" />
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue" />
</p>

Wagmify plays audio into a virtual microphone, so everyone in the call hears it as
your voice. Your own mic still works at the same time: talk and fire a sound effect
over yourself, and nobody needs to know which came from where.

It started as a soundboard. It is now two things in one window:

**Soundboard.** Your folders of clips, a 16-pad grid with global hotkeys, and a
timeline for arranging a few of them. Click to send to the call, hold the headphone
button to hear it yourself first.

**Player.** Search a track and play it into the call, the way Spotify does it in your
presence. Tracks stream, get cached as they play, and save into your music library
when you want to keep one. If you have a Last.fm key, your loved tracks and tag
stations show up too. Music is in the Online tab, next to the soundboard.

Some smaller things: a bar at the bottom keeps the current track reachable from any
tab. Optional Discord presence shows what you are playing, sent to Discord on your
machine and nowhere else. On launch it checks the releases page and tells you when
a new version is out, once.

> **Early alpha.** The Linux / PipeWire path is what I run and test. Windows and
> macOS build and have a `cpal` backend, but their audio has not been tested on real
> hardware yet, so treat them as unfinished.

## Platforms

| Platform | Runs | Audio | Virtual mic |
|---|:--:|---|---|
| Linux (PipeWire) | yes | native `pipewire-rs` | created for you (`pw-loopback`) |
| Windows | yes | `cpal` (WASAPI) | you install VB-CABLE |
| macOS | yes | `cpal` (CoreAudio) | you install BlackHole |

Every OS needs a virtual cable for this to work, and only Linux lets a program
create one. On Windows and macOS you install the free driver once, Wagmify finds it
and routes to it. The Discord side is one setting: pick the virtual device as your
input, and turn off Noise Suppression, Echo Cancellation and Automatic Gain, which
would otherwise chew up the audio. Per-platform details are in
[`docs/PLATFORM-AUDIO.md`](docs/PLATFORM-AUDIO.md).

## Install

Linux, no root, from the latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/0xwagmi/Wagmify/main/scripts/install-release.sh | bash
```

It downloads the `.deb` from the newest release, checks it against `SHA256SUMS`, and
puts the binary, icons and menu entry in `~/.local`. `--version v0.2.0` pins a
release; `--appimage` uses the AppImage instead, which is what you want off Debian.

Windows: take `Wagmify_<version>_x64-setup.exe` from the releases page.

From source, on any of the three:

```bash
npm install
npm run tauri dev        # development window, rebuilds on save
./scripts/install.sh     # packaged build into ~/.local, adds the wagmify command
```

`./scripts/uninstall.sh` removes it again, `--purge` takes the settings and
downloaded tools with it.

Closing the window leaves Wagmify in the tray, since a soundboard is most useful
while its window is shut. On GNOME the tray needs the AppIndicator extension, which
Ubuntu ships enabled.

## Requirements

- Linux: PipeWire 1.0 or newer. Everything else is bundled.
- Music search needs `yt-dlp`, which the import panel downloads into the app's data
  folder on first use. Nothing else is installed system-wide.
- Last.fm is optional. Create an application at `last.fm/api`, paste the API key in
  Settings, and tag stations and loved tracks appear. No key, no Last.fm, everything
  else still works.

## How it is built

Rust for the audio, Tauri 2 for the window, Svelte for the interface. Decoding is
`symphonia`, pure Rust, so there is no ffmpeg to install. On Linux the engine talks
PipeWire directly through `pipewire-rs`. The result is one binary with the UI baked
in, no runtime to install.

```
crates/audio-core/     the audio engine: decode, mix, stream, cache, denoise
src-tauri/             the app: commands, library, Online, Discord, updates
src/                   Svelte frontend
docs/                  architecture, decisions, platform notes, roadmap
```

The engine keeps one persistent virtual mic, mixes every voice into it and applies
its own limiter, so five sounds at once do not clip. Streamed music and local files
go through the same mixer. There is an optional RNNoise gate on the mic path for
people whose real mic hisses between words.

## Tests and releases

```bash
cargo test --workspace     # engine, cache, matching, presence, updates
npm run check              # svelte-check
```

Releases are cut by CI from a tag, never by hand. Bumping the version, tagging and
pushing `vX.Y.Z` builds Linux and Windows, runs the tests, checks that the tag
matches the app version, and attaches the installers plus `SHA256SUMS`. `0.x` tags
are published as pre-releases.

## License

MIT, see [LICENSE](LICENSE). The interface and icon art are part of the repository.
