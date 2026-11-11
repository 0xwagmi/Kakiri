<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Play, Square, Repeat, Plus, X, Music } from "$lib/icons";

  type Sound = {
    id: string;
    path: string;
    name: string;
    title: string | null;
    artist: string | null;
    duration_secs: number | null;
  };
  type Clip = {
    id: string;
    path: string;
    name: string;
    track: number;
    start: number;
    dur: number;
    gain: number;
  };
  type Status = { playing: boolean; position: number; length: number };

  export let sounds: Sound[] = [];
  export let storageKey = "sb.timeline";

  const TRACKS = 6;
  const SNAP = 0.1; // seconds
  const MIN_LEN = 1; // seconds of empty timeline to keep usable
  const LS = storageKey;
  const MIN_PPS = 2; // ≈ 7 minutes across a wide window
  const MAX_PPS = 400;

  const clampPps = (v: number) => Math.min(MAX_PPS, Math.max(MIN_PPS, v));
  // The zoom slider is logarithmic: low end (whole songs) gets as much travel
  // as the high end (millisecond nudging), otherwise it is unusable.
  const ppsFromSlider = (v: number) => clampPps(MIN_PPS * Math.pow(MAX_PPS / MIN_PPS, v / 1000));
  const sliderFromPps = (p: number) => (1000 * Math.log(p / MIN_PPS)) / Math.log(MAX_PPS / MIN_PPS);

  let clips: Clip[] = [];
  let looped = false;
  let pps = 40; // pixels per second
  let position = 0;
  let playing = false;
  let length = 0;
  let search = "";
  let selId: string | null = null;
  let toast = "";
  let drag: { id: string; dx: number; track: number } | null = null;
  let poll: ReturnType<typeof setInterval> | undefined;
  let laneEl: HTMLDivElement;
  let laneW = 900;
  let nextId = 1;

  const durOf = (s: Sound) => Math.max(0.25, s.duration_secs ?? 1);
  const label = (s: Sound) => s.title ?? s.name;

  const shown = () => {
    const q = search.trim().toLowerCase();
    const list = q ? sounds.filter((s) => label(s).toLowerCase().includes(q)) : sounds;
    // the same file can be reachable from two roots (a folder added twice, or a
    // library that was moved): keyed each blocks throw on duplicate paths
    const seen = new Set<string>();
    return list.filter((s) => (seen.has(s.path) ? false : (seen.add(s.path), true))).slice(0, 200);
  };

  /** End of the last clip (what the ruler/scroll width is based on). */
  $: contentEnd = clips.reduce((m, c) => Math.max(m, c.start + c.dur), 0);
  /** Visible span: always covers the whole lane, so you can click anywhere. */
  $: need = Math.max(contentEnd + 4, laneW / pps + 2);
  $: span = Math.max(12, Math.ceil(need / 5) * 5);

  // Tick spacing adapts to the zoom so labels never collide and lines stay
  // useful: the smallest step that still leaves room for a label wins.
  const STEPS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800];
  // NOTE: keep `pps` visible in the block — Svelte only tracks identifiers it
  // can see, so reading it inside a helper would leave the ruler stale.
  let majorStep = 5;
  let minorStep = 1;
  $: {
    const p = pps;
    const major = STEPS.find((st) => st * p >= 56) ?? 1800;
    const minor = STEPS.find((st) => st * p >= 9) ?? 1800;
    majorStep = major;
    minorStep = Math.min(major, minor);
  }
  $: ticks = buildTicks(span, minorStep, majorStep);

  function buildTicks(end: number, minor: number, major: number) {
    const out: { t: number; major: boolean }[] = [];
    const n = Math.min(4000, Math.floor(end / minor) + 1);
    for (let i = 0; i < n; i++) {
      const t = Math.round(i * minor * 1000) / 1000;
      out.push({ t, major: Math.abs(t / major - Math.round(t / major)) < 1e-6 });
    }
    return out;
  }

  /** 0:05 / 1:30 / 12:00 for minutes, 5s or 0.5 for the fine zoom. */
  function fmtTime(t: number, step: number) {
    if (step < 1) return t.toFixed(1);
    const m = Math.floor(t / 60);
    const sec = Math.round(t % 60);
    return m > 0 ? `${m}:${String(sec).padStart(2, "0")}` : `${sec}s`;
  }

  /** 1:23.4 for the transport (minutes are easier to read than 500s). */
  function fmtClock(t: number) {
    const m = Math.floor(t / 60);
    const sec = t % 60;
    return m > 0 ? `${m}:${sec.toFixed(1).padStart(4, "0")}` : `${t.toFixed(1)}s`;
  }

  /** Zoom so the whole timeline fits the lane (3-minute songs, no scrolling). */
  function fit() {
    const end = Math.max(contentEnd, 4);
    pps = clampPps((laneW - 12) / end);
    save();
  }

  function onWheel(ev: WheelEvent) {
    if (!(ev.ctrlKey || ev.metaKey)) return;
    ev.preventDefault();
    const rect = laneEl.getBoundingClientRect();
    const at = (ev.clientX - rect.left + laneEl.scrollLeft) / pps;
    pps = clampPps(pps * Math.exp(-ev.deltaY * 0.0025));
    save();
    // keep the time under the cursor pinned while zooming
    requestAnimationFrame(() => {
      laneEl.scrollLeft = Math.max(0, at * pps - (ev.clientX - rect.left));
    });
  }

  function flash(msg: string) {
    toast = msg;
    setTimeout(() => {
      if (toast === msg) toast = "";
    }, 2200);
  }

  function load() {
    try {
      const raw = localStorage.getItem(LS);
      if (!raw) return;
      const data = JSON.parse(raw);
      if (Array.isArray(data.clips)) {
        clips = data.clips.map((c: Clip) => ({ ...c, gain: c.gain ?? 1 }));
        nextId = clips.reduce((m, c) => Math.max(m, Number(c.id) || 0), 0) + 1;
      }
      if (typeof data.looped === "boolean") looped = data.looped;
      if (typeof data.pps === "number") pps = clampPps(data.pps);
    } catch {
      /* ignore corrupt local state */
    }
  }

  function save() {
    localStorage.setItem(LS, JSON.stringify({ clips, looped, pps }));
  }

  /** Snap to the grid, kept to milliseconds so saved values stay clean. */
  const snap = (t: number) => Math.max(0, Math.round(Math.round(t / SNAP) * SNAP * 1000) / 1000);

  /** First track where a clip of `dur` at `start` wouldn't overlap anything. */
  function freeTrack(start: number, dur: number) {
    for (let i = 0; i < TRACKS; i++) {
      const hit = clips.some(
        (c) => c.track === i && start < c.start + c.dur && c.start < start + dur,
      );
      if (!hit) return i;
    }
    return 0; // everything is busy → stack on the top track
  }

  async function addClip(s: Sound) {
    const start = snap(Math.max(position, 0));
    const dur = durOf(s);
    const clip: Clip = {
      id: String(nextId++),
      path: s.path,
      name: label(s),
      track: freeTrack(start, dur),
      start,
      dur,
      gain: 1,
    };
    clips = [...clips, clip];
    save();
    // Drop it into the timeline that is already playing: nothing restarts and
    // the sounds that are sounding right now keep going.
    if (!playing) return;
    try {
      const st = await invoke<Status>("timeline_add_clip", { clip: payloadOf(clip) });
      length = st.length;
    } catch (e) {
      flash(String(e));
    }
  }

  async function removeClip(id: string) {
    clips = clips.filter((c) => c.id !== id);
    if (selId === id) selId = null;
    save();
    if (!playing) return;
    try {
      const st = await invoke<Status>("timeline_remove_clip", { id });
      length = st.length;
    } catch {
      /* the timeline already stopped */
    }
  }

  function clearAll() {
    if (!clips.length) return;
    clips = [];
    selId = null;
    save();
    stop();
  }

  function moveTo(clip: Clip, start: number, track: number) {
    clip.start = snap(start);
    clip.track = Math.max(0, Math.min(TRACKS - 1, track));
    clips = clips; // trigger reactivity
    save();
  }

  // ---- transport -----------------------------------------------------------

  const payloadOf = (c: Clip) => ({
    id: c.id,
    path: c.path,
    track: c.track,
    start: c.start,
    gain: c.gain,
  });

  function payload() {
    return clips.map(payloadOf);
  }

  async function play() {
    if (!clips.length) {
      flash("Add some sounds first");
      return;
    }
    if (playing) {
      stop();
      return;
    }
    try {
      const st = await invoke<Status>("timeline_play", { clips: payload(), looped });
      playing = st.playing;
      length = st.length;
      position = 0;
      startPolling();
    } catch (e) {
      flash(String(e));
    }
  }

  async function stop() {
    try {
      await invoke("timeline_stop");
    } catch {
      /* engine may already be gone */
    }
    playing = false;
    position = 0;
    stopPolling();
  }

  function startPolling() {
    stopPolling();
    poll = setInterval(async () => {
      try {
        const st = await invoke<Status>("timeline_status");
        position = st.position;
        length = st.length;
        followPlayhead(st.position);
        if (!st.playing) {
          playing = false;
          stopPolling();
        }
      } catch {
        stopPolling();
      }
    }, 60);
  }

  /** Keep the playhead on screen — a 3-minute clip scrolls far out of view. */
  function followPlayhead(pos: number) {
    const px = pos * pps;
    const left = laneEl.scrollLeft;
    const right = left + laneEl.clientWidth;
    if (px < left || px > right - 24) {
      laneEl.scrollLeft = Math.max(0, px - laneEl.clientWidth * 0.12);
    }
  }

  function stopPolling() {
    if (poll) clearInterval(poll);
    poll = undefined;
  }

  function toggleLoop() {
    looped = !looped;
    save();
    // Live: the playhead keeps running, the loop just starts/stops wrapping.
    if (playing) invoke("timeline_set_loop", { looped }).catch(() => {});
  }

  // ---- mouse ---------------------------------------------------------------

  function lanePoint(ev: PointerEvent) {
    const r = laneEl.getBoundingClientRect();
    return { x: ev.clientX - r.left + laneEl.scrollLeft, y: ev.clientY - r.top + laneEl.scrollTop };
  }

  const ROW_H = 34;

  function onClipDown(ev: PointerEvent, c: Clip) {
    ev.preventDefault();
    selId = c.id;
    const p = lanePoint(ev);
    drag = { id: c.id, dx: p.x - c.start * pps, track: c.track };
    try {
      (ev.currentTarget as HTMLElement).setPointerCapture(ev.pointerId);
    } catch {
      /* capture is a nicety; dragging still works without it */
    }
  }

  function onLaneMove(ev: PointerEvent) {
    if (!drag) return;
    const c = clips.find((x) => x.id === drag!.id);
    if (!c) return;
    const p = lanePoint(ev);
    const track = Math.floor(p.y / ROW_H);
    moveTo(c, (p.x - drag.dx) / pps, Number.isFinite(track) ? track : c.track);
  }

  function onLaneUp() {
    if (!drag) return;
    const moved = clips.find((c) => c.id === drag!.id);
    drag = null;
    if (moved && playing) {
      // Replace in place: the player keeps the clip's started flag, so a clip
      // that is already sounding is not triggered a second time.
      invoke("timeline_add_clip", { clip: payloadOf(moved) }).catch(() => {});
    }
  }

  async function scrub(ev: PointerEvent) {
    if (drag) return;
    const p = lanePoint(ev);
    const at = Math.max(0, p.x / pps);
    position = at;
    if (!playing) return;
    // Restart from the clicked position: clips that already ended are skipped,
    // a clip spanning the playhead resumes part-way in.
    try {
      stopPolling();
      const st = await invoke<Status>("timeline_play", {
        clips: payload(),
        looped,
        from: at,
      });
      playing = st.playing;
      length = st.length;
      startPolling();
    } catch (e) {
      flash(String(e));
    }
  }

  function onKey(ev: KeyboardEvent) {
    const t = ev.target as HTMLElement;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
    if (ev.code === "Space") {
      ev.preventDefault();
      play();
    } else if (ev.key === "Delete" || ev.key === "Backspace") {
      if (selId) {
        ev.preventDefault();
        removeClip(selId);
      }
    } else if (ev.key === "Escape") {
      selId = null;
    }
  }

  let ro: ResizeObserver | undefined;

  onMount(() => {
    load();
    window.addEventListener("keydown", onKey);
    // Keep the ruler at least as wide as the viewport so scrubbing works on
    // the empty area to the right of the last clip.
    ro = new ResizeObserver(([e]) => {
      if (e) laneW = e.contentRect.width;
    });
    ro.observe(laneEl);
    laneEl.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKey);
      laneEl.removeEventListener("wheel", onWheel);
      ro?.disconnect();
    };
  });

  onDestroy(() => {
    stopPolling();
    if (playing) invoke("timeline_stop").catch(() => {});
  });
</script>

<div class="tl">
  <aside class="picker">
    <div class="pick-head">
      <input placeholder="Filter sounds…" bind:value={search} />
    </div>
    <div class="pick-list">
      {#each shown() as s (s.path)}
        <button class="pick" onclick={() => addClip(s)} title="Add to the timeline">
          <Music size={13} />
          <span class="pn">{label(s)}</span>
          <span class="add"><Plus size={13} /></span>
        </button>
      {/each}
      {#if !shown().length}
        <p class="empty">No sounds. Add a folder in the Library tab.</p>
      {/if}
    </div>
  </aside>

  <section class="arrange">
    <div class="transport">
      <button class="play" onclick={play} title={playing ? "Stop" : "Play (Space)"}>
        {#if playing}<Square size={14} fill="currentColor" />{:else}<Play size={14} fill="currentColor" />{/if}
      </button>
      <button class="ghost" class:on={looped} onclick={toggleLoop} title="Loop the timeline">
        <Repeat size={13} />
      </button>
      <span class="time">{fmtClock(position)}{length ? ` / ${fmtClock(length)}` : ""}</span>
      <span class="spacer"></span>
      <button class="ghost" onclick={fit} disabled={!clips.length} title="Zoom out to show everything">Fit</button>
      <label class="zoom">
        Zoom
        <input
          type="range"
          min="0"
          max="1000"
          step="1"
          value={sliderFromPps(pps)}
          oninput={(e) => {
            pps = ppsFromSlider(Number(e.currentTarget.value));
          }}
          onchange={save}
          title="Ctrl + scroll on the timeline also zooms"
        />
      </label>
      <button class="ghost" onclick={clearAll} disabled={!clips.length}>Clear</button>
      {#if toast}<span class="ok">{toast}</span>{/if}
    </div>

    <div class="lane" bind:this={laneEl} onpointermove={onLaneMove} onpointerup={onLaneUp}
         onpointerleave={onLaneUp} role="presentation">
      <div class="inner" style="width:{span * pps}px">
        <div class="ruler" onpointerdown={scrub} role="presentation">
          {#each ticks as tk (tk.t)}
            <span class="tick" class:major={tk.major} style="left:{tk.t * pps}px">
              <i></i>{#if tk.major}<em>{fmtTime(tk.t, majorStep)}</em>{/if}
            </span>
          {/each}
        </div>

        <div class="rows" style="height:{TRACKS * ROW_H}px" onpointerdown={scrub} role="presentation">
          {#each Array(TRACKS) as _, i (i)}
            <div class="row" class:alt={i % 2 === 1} style="top:{i * ROW_H}px;height:{ROW_H}px"></div>
          {/each}

          {#each clips as c (c.id)}
            <div
              class="clip"
              class:sel={selId === c.id}
              class:narrow={c.dur * pps < 34}
              style="left:{c.start * pps}px;top:{c.track * ROW_H + 3}px;width:{Math.max(
                14,
                c.dur * pps - 2,
              )}px;height:{ROW_H - 6}px"
              onpointerdown={(e) => onClipDown(e, c)}
              role="button"
              tabindex="0"
              title={`${c.name} · drag to move, double-click to remove`}
              ondblclick={() => removeClip(c.id)}
            >
              <span class="cn">{c.name}</span>
              <button
                class="del"
                onpointerdown={(e) => e.stopPropagation()}
                onclick={(e) => {
                  e.stopPropagation();
                  removeClip(c.id);
                }}
                title="Remove"
              >
                <X size={10} />
              </button>
            </div>
          {/each}

          <div class="playhead" style="left:{position * pps}px"></div>
        </div>
      </div>
    </div>
  </section>
</div>

<style>
  .tl {
    flex: 1;
    display: flex;
    min-height: 0;
    min-width: 0;
  }

  /* ---- sound picker: a quiet list, no boxed-in feeling ---- */
  .picker {
    width: 220px;
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding-right: 6px;
  }
  .pick-head { padding: 12px 10px 8px 14px; }
  .pick-head input {
    width: 100%;
    box-sizing: border-box;
    background: var(--panel-2);
    border: 1px solid transparent;
    border-radius: 9px;
    color: var(--text);
    padding: 7px 10px;
    font-size: 12px;
    outline: none;
    transition: border-color 140ms ease, background 140ms ease;
  }
  .pick-head input:focus {
    border-color: rgb(139 123 255 / 0.45);
    background: var(--panel-3);
  }
  .pick-head input::placeholder { color: var(--text-3, var(--muted)); }

  .pick-list { flex: 1; overflow-y: auto; padding: 0 6px 10px; }
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 8px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--text-2, var(--muted));
    text-align: left;
    font-size: 12px;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }
  .pick:hover { background: var(--panel-2); color: var(--text); }
  .pick .pn { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick .add { opacity: 0; color: var(--accent-2, #a48bff); transition: opacity 120ms ease; }
  .pick:hover .add { opacity: 1; }
  .empty {
    color: var(--text-3, var(--muted));
    font-size: 11.5px;
    line-height: 1.5;
    padding: 12px 10px;
    margin: 0;
  }

  /* ---- arranger ---- */
  .arrange { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }

  .transport {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
  }
  .play {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 50%;
    cursor: pointer;
    color: #fff;
    background: linear-gradient(180deg, var(--accent-2, #a48bff), var(--accent, #8b7bff));
    box-shadow: 0 2px 10px rgb(139 123 255 / 0.35);
    transition: transform 120ms ease, box-shadow 120ms ease;
  }
  .play:hover { box-shadow: 0 3px 14px rgb(139 123 255 / 0.5); }
  .play:active { transform: translateY(1px); }

  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 11px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: transparent;
    color: var(--text-2, var(--muted));
    font-size: 11.5px;
    cursor: pointer;
    transition: border-color 120ms ease, color 120ms ease;
  }
  .ghost:hover:not(:disabled) { color: var(--text); border-color: var(--accent); }
  .ghost.on {
    color: var(--accent-2, #a48bff);
    border-color: rgb(139 123 255 / 0.45);
    background: rgb(139 123 255 / 0.1);
  }
  .ghost:disabled { opacity: 0.4; cursor: default; }

  .time {
    font-size: 11.5px;
    color: var(--text-3, var(--muted));
    font-variant-numeric: tabular-nums;
    min-width: 96px;
    letter-spacing: 0.01em;
  }
  .spacer { flex: 1; }
  .zoom {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--text-3, var(--muted));
  }
  .ok { color: var(--ok); font-size: 11.5px; }

  /* a slim, quiet slider */
  .zoom input {
    width: 120px;
    appearance: none;
    height: 3px;
    border-radius: 2px;
    background: var(--border);
    outline: none;
  }
  .zoom input::-webkit-slider-thumb {
    appearance: none;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--accent, #8b7bff);
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.4);
    cursor: pointer;
  }

  /* ---- the lanes ---- */
  .lane { flex: 1; overflow: auto; min-height: 0; position: relative; }
  .inner { position: relative; }

  .ruler {
    position: relative;
    height: 26px;
    cursor: pointer;
    user-select: none;
  }
  .tick { position: absolute; top: 0; bottom: 0; }
  .tick i {
    position: absolute;
    left: 0;
    top: 18px;
    bottom: 0;
    width: 1px;
    background: rgb(255 255 255 / 0.05);
  }
  .tick.major i { top: 12px; background: rgb(255 255 255 / 0.11); }
  .tick em {
    position: absolute;
    left: 4px;
    top: 2px;
    font-style: normal;
    font-size: 9px;
    color: var(--text-3, var(--muted));
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
  }

  .rows { position: relative; }
  .row { position: absolute; left: 0; right: 0; box-shadow: inset 0 -1px 0 rgb(255 255 255 / 0.04); }
  .row.alt { background: rgb(255 255 255 / 0.015); }

  /* clips: soft, no hard border, readable at any zoom */
  .clip {
    position: absolute;
    display: flex;
    align-items: center;
    padding: 0 16px 0 9px;
    border: 0;
    border-radius: 7px;
    background:
      linear-gradient(180deg, rgb(139 123 255 / 0.32), rgb(139 123 255 / 0.2)),
      var(--panel-2);
    box-shadow: inset 0 0 0 1px rgb(139 123 255 / 0.28);
    color: var(--text);
    cursor: grab;
    overflow: hidden;
    touch-action: none;
    transition: box-shadow 120ms ease, background 120ms ease;
  }
  .clip:hover { background: linear-gradient(180deg, rgb(139 123 255 / 0.4), rgb(139 123 255 / 0.26)), var(--panel-2); }
  .clip.sel {
    box-shadow:
      inset 0 0 0 1px var(--accent-2, #a48bff),
      0 0 0 2px rgb(139 92 246 / 0.3);
  }
  .clip:active { cursor: grabbing; }
  .clip.narrow { padding: 0; border-radius: 5px; }
  .clip.narrow .cn { display: none; }
  .clip .cn {
    font-size: 10.5px;
    letter-spacing: -0.005em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .clip .del {
    position: absolute;
    right: 2px;
    top: 2px;
    display: grid;
    place-items: center;
    padding: 1px;
    border: 0;
    background: none;
    color: var(--text-3, var(--muted));
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease, color 120ms ease;
  }
  .clip:hover .del { opacity: 1; }
  .clip .del:hover { color: var(--danger); }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgb(255 255 255 / 0.45);
    pointer-events: none;
  }
</style>
