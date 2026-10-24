# Changelog

All notable changes to Kakiri are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] — desktop polish (Linux alpha)

Everything a reviewer would poke at in a first release. No audio-path changes.

### Added

- **Single-instance guard**: launching Kakiri while it is already running raises
  the existing window instead of starting a second engine. Two engines used to
  write the same virtual sink and the same settings file.
- **File logging** to `~/.local/share/com.kakiri.soundboard/logs/Kakiri.log` with
  a panic hook, so a bug report comes with context: version, backend, bus
  format, virtual-mic status and output target are logged at startup.
  Third-party crates log errors only (symphonia logs a warning per probed file).
- **AppStream metadata** (`/usr/share/metainfo`), so software centres can show
  Kakiri with a description and category.
- `pipewire` added to the package dependencies.

### Changed

- **Content Security Policy** instead of `null`: no inline script, `data:` only
  for images/media, `connect-src` limited to the app and its IPC origins.
- The desktop entry is now `Name=Kakiri` with
  `Categories=AudioVideo;Audio;Mixer;` (it was "Kakiri Soundboard" and an empty
  category, which filed it under "Other" in some launchers).
- Release assets are named `Kakiri_<version>_amd64.deb` rather than
  `Kakiri Soundboard_…`, and `SHA256SUMS` lists plain file names so
  `sha256sum -c SHA256SUMS` works for files downloaded side by side.
- The release workflow builds on **Ubuntu 24.04**: `libspa-sys` needs PipeWire 1.0
  headers, so a 22.04 build was never runnable anyway. Release notes now state
  the real requirement.
- `scripts/run.sh` and the `app`/`serve` npm scripts are gone. Development is
  `npm run tauri dev`; installing is `./scripts/install.sh`.

### Fixed

- Package `Depends` duplicated webkit/gtk, and the deb `Description` was empty.

## [0.1.0] — first public build (Linux alpha)

First release. Kakiri plays sounds through a virtual microphone so Discord (or
any other app) hears them as your voice, and mixes your real microphone in so
you can still talk.

### Added

- **Audio engine** (`crates/audio-core`): mix bus with per-voice gain, seek,
  retriggering modes (layer / choke / toggle), soft limiter, and a master gain
  applied at render time so it also covers sounds that start later.
- **Linux output** via native PipeWire (ADR-4), with a `pw-play` CLI fallback
  (`SOUNDBOARD_PW_BACKEND=cli`) and a `cpal` backend written for Windows/macOS.
- **Virtual microphone**: a daemon-owned `libpipewire-module-loopback` drop-in
  (`~/.config/pipewire/pipewire.conf.d/90-kakiri-soundboard.conf`) that survives
  app exit and reboot, auto-created at launch. Shows up in Discord as
  "Soundboard Mic".
- **Live voice input** captured by the app itself and mixed into the virtual mic
  — no out-of-band `pw-link` wiring.
- **Streaming playback**: per-voice decoder thread feeding a lock-free SPSC ring
  with a stateful Catmull-Rom resampler. First audio on a 4½-minute file went
  from 6.16 s to 8 ms, memory from ~104 MB to ~0.8 MB per voice, 0 dropouts.
- **Library**: folder tree with counts, cross-library search, tags and embedded
  cover art, borderless sound cards, pad grid with global hotkeys.
- **Timeline arranger**: stack sounds as clips on 6 tracks, drag to move/track,
  scrub, loop, zoom from 2 to 400 px/s with a Fit button, adaptive ruler,
  ctrl+scroll zoom, playhead following. Clips are ordinary voices, so
  overlapping clips mix. Editable while it plays: adding, removing, moving a clip
  or toggling loop never restarts playback.
- **Import**: drop files on the window, pick files, or paste a link. Audio is
  copied untouched; video has its audio extracted with ffmpeg; page links
  (YouTube/TikTok…) go through yt-dlp with an optional clip range
  (`--download-sections`) so a 3-minute video becomes a 6-second sound. The
  downloader can be installed/updated from the panel.
- **Install tooling**: `scripts/install.sh` (standalone or `--dev`),
  `scripts/uninstall.sh` (with `--purge`) and a `kakiri` command with desktop
  entry and icons.
- **Release automation**: tag-driven GitHub Actions workflow publishing
  `.deb` / `.rpm` / `.AppImage` with checksums.

### Fixed

- Timeline was silent whenever the monitor output was enabled: the audio writer
  held the output mutex across a blocking `push()`, which stalled the scheduler.
  Outputs are now owned by the writer thread and swapped over a channel.
- Files using AAC profiles the decoder cannot handle (HE-AAC/HE-AACv2, which
  every media player plays) failed with "aac too complex". Imports are now
  decode-verified and re-encoded to AAC-LC when needed, and existing library
  files offer a one-click Convert.
- An age-restricted link was reported as "the downloader is out of date", which
  led to pointless 40 MB updates. Errors now carry the action they need
  (update / sign-in), and updates check GitHub first so an up-to-date downloader
  is never downloaded again.
- Master volume ignored anything that started after it was changed.
- Icons shipped with a 50 % opaque black backdrop instead of transparency.

### Known limitations

- **Linux only.** The Windows/macOS audio path (`cpal` + VB-CABLE/BlackHole) is
  implemented but has not been run on real hardware yet.
- Global hotkeys are best-effort on Wayland; in-app shortcuts always work.
- Requires **PipeWire 1.0+** (Ubuntu 24.04+ / Fedora 39+ / Arch; tested on Ubuntu
  24.04 with PipeWire 1.0.5). Ubuntu 22.04 ships PipeWire 0.3.48 and cannot run
  it; the build links glibc 2.39.
- No tray menu or single-instance guard yet.
- `ffmpeg` and `yt-dlp` are optional, detected on `PATH`; the app offers to fetch
  yt-dlp itself.

[0.1.1]: https://github.com/0xwagmi/Kakiri/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/0xwagmi/Kakiri/releases/tag/v0.1.0
