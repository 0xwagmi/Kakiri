# Changelog

All notable changes to Kakiri are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.6]

### Added

- **Online mode**, a fourth view next to Library/Pads/Timeline. Two panes: playlists
  and stations on the left, tracks on the right.
  - **Trending and search** work with no account, from a key-less catalogue that
    also supplies artwork and track length.
  - **Last.fm** as the personal layer: your loved tracks and tag stations
    (`synthwave`, `lofi`, `phonk`…). Needs your own API key; the shared secret is
    not used and should stay private.
  - **Saved** playlist, read from the library's `Online` folder rather than
    remembered by the UI.
- **Play a link without saving it**, from Import: resolves the stream and plays it,
  useful for a one-off.
- **Track cache**: everything streamed is kept on disk, so replays start instantly
  and keep working offline. Coverage is tracked per 64 KiB block (the container
  index sits at the end of the file, so "enough bytes" is not "complete"). Capped
  at 100 MB per track and 1.5 GB total, evicted least-recently-used.
- **Background prefetch** keeps filling the cache while a track plays *or is
  paused*, so a short song finishes caching even if you stop.
- **Save** writes the track into `Music/Kakiri/Online` as a real library file, with
  a small metadata sidecar (artist, title, cover) beside it. Saving a cached track
  is a local copy, not a second download, and the button shows the cache filling
  with real KB.
- **Delete saves** with selection and a confirmation dialog — one row or many at
  once — then both the Saved list and the Library rescan.
- **Player bar**: pause/resume that keeps the position, click-to-seek on the
  progress line, shuffle, repeat, auto-advance through the queue, and a
  **Recently played** list.
- **Settings → Online** for the Last.fm key and username, hidden by default with an
  eye toggle. The key is kept out of `sb.prefs` and stored in Kakiri's own data
  folder, obfuscated, in a file only your user can read. **Known limitation:** with
  a fixed salt in the source this is obfuscation, not encryption — anyone who can
  read that file as you can decode it. Moving it to the system keyring is planned.

### Changed

- Online playback is **exclusive**: clicking several rows quickly leaves one voice,
  and the latest click wins instead of every resolve starting its own stream.
- **First-play latency roughly halved**: one yt-dlp launch instead of two
  (search and resolve together, ~10 s → ~5.5 s), and the HTTP reader starts with a
  32 KB window instead of pulling 512 KB to read a 16-byte header (probe 2.7 s →
  0.44 s).
- Saved rows show the catalogue's artist/title and cover instead of a guess from
  the file name.

### Fixed

- **Covers were blank in the packaged app.** The content security policy only
  allowed `self`/`data:` images, so every remote cover was silently blocked —
  visible only in a real build, never in `tauri dev`. Cover hosts are allowed now,
  and an image that still fails hides itself instead of leaving an empty tile.
- Clicking a track played the **previously selected** one (`current` was read one
  flush before the assignment that changed it).
- Deleting a save left its "saved" mark behind, so the track could not be
  downloaded again.
- Saved artist and title were **swapped** for files named `Title - Artist`.
- Recently played could point at a deleted file and refuse to play; it now
  remembers the track, and falls back to streaming if a local file vanished.
- Last.fm credentials were saved but not restored, so the fields looked empty after
  a restart and the first keystroke overwrote the key.

## [0.1.5]

### Added

- **Noise reduction (RNNoise)** for the live mic, in Settings. `nnnoiseless`, the
  pure-Rust rnnoise port, runs on samples the engine already captures: ~1 % of a
  core and one 480-sample frame (10 ms) of added latency, replacing an external
  processor chain that cost a separate process *and* an extra graph hop.
  A gate closes to **digital silence** between words (hangover ~20 ms) instead of
  leaving residual hiss, and a wet/dry mix keeps some natural body in the voice.
  Measured on a fixed white-noise input: −3.3 dB with the gate off, −16.2 dB with
  it on. Turn Discord's own suppression off afterwards — it would gate the sounds
  you play. Tuning knobs, for A/B tests without rebuilding: `KAKIRI_WET_DB`,
  `KAKIRI_VAD`, `KAKIRI_VAD_THRES`, `KAKIRI_RELEASE_MS`.
  A DeepFilterNet engine was measured and dropped: 44 % of a core on a laptop
  against RNNoise's 1 %, for a quality difference Discord's 64 kbps Opus would
  largely erase.
- **The companion speaks.** Status lines ("Added music", a decode error, "stopped
  FEIN") appear as a bubble above her head with the action button inside it,
  instead of a line in the far corner. When she is switched off in Settings the
  old bottom line still renders.

## [0.1.4] — Windows

### Added

- **Windows build.** Built natively on `windows-latest` and shipped as a
  `*_x64-setup.exe` next to the Linux packages. Windows has no way for a
  user-space app to create an audio device, so it needs **VB-CABLE**: set
  `Send to` to `CABLE Input` and pick `CABLE Output` in Discord. Untested on real
  hardware — this build exists so it can be tested.

### Changed

- **Accent discipline.** The accent used to do five jobs at once (logo, active
  tab, Monitor, selected folder, every artwork tile), so nothing read as
  important. Now one filled button — Import — carries it, active toggles get a
  tint and a hairline, the selected folder is a neutral surface with a 2px accent
  edge, and artwork tiles are neutral.
- `SHA256SUMS` is generated once, from the published assets, so it cannot drift
  from what was actually shipped.

### Notes

- `ci.yml` compiles both platforms on every push to `dev`. Windows had never been
  compiled before this release; a tag was the first place a break would show up.
- Version traps: all four version files must be bumped together.

## [0.1.3] — companion

### Added

- **Companion**: a small resident in the bottom-right corner. She waves once at
  launch, idles in two alternating loops, dances while something plays and
  switches to listening with headphones during longer playback. Quiet for five
  minutes and she sits down, fifteen and she is asleep; with an empty library she
  points at the grid.
- `scripts/companion-assets.sh` turns a flat-backdrop clip into the animated
  WebP loops: backdrop colour and crop box are detected, so new footage needs no
  config, letterbox bars and borders are trimmed by comparing edge strips to the
  backdrop, and each clip is scaled against a shared reference height so she does
  not change size between states. Eight loops, 4.4 MB.
- A **Companion** switch in Settings to hide her; the component unmounts, so the
  animation tick stops and the decoded frames are released.

### Notes

- The download grows from 7.9 MB to 12.2 MB. That is entirely the eight loops:
  animated WebP is already compressed, so it does not shrink inside the package.

## [0.1.2] — tray icon

### Added

- **Tray icon** (white cat, works with GNOME's appindicator extension): left
  click toggles the window, the menu has Show / Quit. Closing the window now
  hides Kakiri instead of quitting, so a soundboard keeps working mid-call;
  Quit lives in the tray so the app can never be left running with no way back.

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

[0.1.5]: https://github.com/0xwagmi/Kakiri/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/0xwagmi/Kakiri/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/0xwagmi/Kakiri/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/0xwagmi/Kakiri/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/0xwagmi/Kakiri/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/0xwagmi/Kakiri/releases/tag/v0.1.0
