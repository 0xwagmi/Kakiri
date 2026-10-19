# UI

## Screens

1. **Board** (main) — grid of pads.
2. **Library** — list/search of all sounds; drag onto pads.
3. **Settings** — audio device, bus format, monitor, cache, library folders.
4. **Setup / Onboarding** — virtual-cable detect + Discord checklist + test tone.

---

## 1. Board (main window)

```
┌───────────────────────────────────────────────────────────────────┐
│  Soundboard            [ Search…            ]   [⚙] [🎧 Monitor ▢] │
├───────────────────────────────────────────────────────────────────┤
│  Tabs:  All | Memes | Music | FX | Favs | +                       │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐           │
│  │ airhorn│ │ bruh   │ │ applause│ │  ▶ lo-fi │ │  drop  │         │
│  │  🔁 ▢  │ │        │ │        │ │  loop ▢ │ │        │         │
│  │  _▁▂▃_ │ │  _▁▃_  │ │  _▁▂_  │ │  _▁▂▃_ │ │  _▂▃_  │         │
│  └────────┘ └────────┘ └────────┘ └────────┘ └────────┘           │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐           │
│  │  +     │ │        │ │        │ │        │ │        │           │
│  └────────┘ └────────┘ └────────┘ └────────┘ └────────┘           │
├───────────────────────────────────────────────────────────────────┤
│  Out: Soundboard Mic ▾   Now: airhorn ▍▍▍▍░  [STOP ALL]   ♪− ♪+    │
└───────────────────────────────────────────────────────────────────┘
```

Pad anatomy:
- Title (from tags or filename), tiny waveform strip.
- Hover: ▶ preview (local only), ✎ edit, ✕ clear.
- Right-click / settings: assign file, hotkey, gain, loop, color, category.
- Active pads pulse with the level; click again to stop (or "toggle" mode).

Bottom bar:
- **Output device** dropdown (virtual mics highlighted).
- **Now playing** mini-list with per-voice stop and a level bar.
- **STOP ALL** (big, or `Esc`).
- **Monitor** toggle (hear it yourself) + master volume.
- Master **mute to mic** (panic) — cuts the virtual-mic bus but keeps monitor.

---

## 2. Library

```
┌───────────────────────────────────────────────────────────────┐
│  [ Search… ]   Tag:▾   ★ Favs   [+ Add folder]  [Import files] │
├───────────────────────────────────────────────────────────────┤
│  ♪ airhorn.mp3        Memes   0:02   ★   ▶  +Pad  🗑           │
│  ♪ applause.wav       FX      0:06       ▶  +Pad  🗑           │
│  ♪ lofi-loop.mp3      Music   12:40  ★   ▶  +Pad  🗑           │
│  …                                                             │
├───────────────────────────────────────────────────────────────┤
│  Drag a row onto a pad to assign. Double-click = preview.      │
└───────────────────────────────────────────────────────────────┘
```

---

## 3. Settings

```
Audio
  Output device (to mic)   [ Soundboard Mic ▾ ]  (Test)
  Monitor device           [ Headphones ▾ ]      (Test)
  Monitor enabled          [ ▢ ]
  Bus sample rate          [ 48000 ▾ ]   Channels [ Stereo ▾ ]
  Trigger latency target   [ Low ▾ ]  (~64–128 frames)

Library
  Folders                  [ ~/Sounds ✕ ] [ + Add ]
  Rescan now
  Cache decoded audio      [ ▣ ]   Clear cache
  Cache size: 412 MB

Interface
  Theme                    [ Dark ▾ ]
  Start minimized to tray  [ ▣ ]
  Global hotkeys enabled   [ ▣ ]   (Wayland: limited)

Virtual mic
  Status:  ● Soundboard Mic ready (Linux, managed by app)
           or  ○ Not found → [ Setup guide ]
```

---

## 4. Onboarding / Setup

Step 1 — choose your platform path:
- Linux: "Create Soundboard Mic" button (runs `pw-loopback`), status turns
  green.
- Windows: "VB-CABLE not detected → Download" link + steps.
- macOS: "BlackHole not detected → `brew install blackhole-2ch`" + steps.

Step 2 — Discord checklist (checkboxes the user ticks):
```
Discord → User Settings → Voice & Video
  [ ] Input Device = Soundboard Mic / CABLE Output / BlackHole 2ch
  [ ] Noise Suppression = OFF
  [ ] Echo Cancellation = OFF
  [ ] Automatic Gain Control = OFF
  [ ] Voice Isolation = OFF
  [ ] Output Device = your headphones
        (click  [ Play test tone ]  then Discord's  [ Mic Test ])
```

Step 3 — done → Board.

---

## Interaction details / decisions

- **Trigger:** left-click = play (or toggle if "latching" set). `Esc` = stop all.
- **Overlap:** pads overlap by default; there is always a "stop all".
- **Preview vs Send:** preview routes locally only; sending routes to the
  virtual-mic bus. Clearly colored differently so you don't accidentally blast
  the call.
- **Feedback:** when a pad is live, show it on the pad and in the "Now" bar;
  peak meters update ~15–20×/s (not per-frame).
- **Drag-drop:** OS files → import to library; library rows → pads.
- **Accessibility:** pads are focusable, keyboard-triggerable, labeled.
