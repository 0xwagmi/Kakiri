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
    return list.slice(0, 200);
  };

  /** End of the last clip (what the ruler/scroll width is based on). */
  $: contentEnd = clips.reduce((m, c) => Math.max(m, c.start + c.dur), 0);
  /** Visible span: always covers the whole lane, so you can click anywhere. */
  $: need = Math.max(contentEnd + 4, laneW / pps + 2);
  $: span = Math.max(12, Math.ceil(need / 5) * 5);
  $: ruler = Array.from({ length: Math.floor(span) + 1 }, (_, i) => i);

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
      if (typeof data.pps === "number") pps = data.pps;
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

  function addClip(s: Sound) {
    const start = snap(Math.max(position, 0));
    const dur = durOf(s);
    clips = [
      ...clips,
      {
        id: String(nextId++),
        path: s.path,
        name: label(s),
        track: freeTrack(start, dur),
        start,
        dur,
        gain: 1,
      },
    ];
    save();
    syncIfPlaying();
  }

  function removeClip(id: string) {
    clips = clips.filter((c) => c.id !== id);
    if (selId === id) selId = null;
    save();
    syncIfPlaying();
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

  function payload() {
    return clips.map((c) => ({
      id: c.id,
      path: c.path,
      track: c.track,
      start: c.start,
      gain: c.gain,
    }));
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

  /** Re-start playback from the top so edits are heard without a manual stop. */
  async function syncIfPlaying() {
    if (!playing) return;
    if (!clips.length) {
      stop();
      return;
    }
    try {
      await invoke<Status>("timeline_play", { clips: payload(), looped });
    } catch (e) {
      flash(String(e));
    }
  }

  function startPolling() {
    stopPolling();
    poll = setInterval(async () => {
      try {
        const st = await invoke<Status>("timeline_status");
        position = st.position;
        length = st.length;
        if (!st.playing) {
          playing = false;
          stopPolling();
        }
      } catch {
        stopPolling();
      }
    }, 60);
  }

  function stopPolling() {
    if (poll) clearInterval(poll);
    poll = undefined;
  }

  function toggleLoop() {
    looped = !looped;
    save();
    syncIfPlaying();
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
    drag = null;
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
    return () => {
      window.removeEventListener("keydown", onKey);
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
      <span class="time">{position.toFixed(1)}s{length ? ` / ${length.toFixed(1)}s` : ""}</span>
      <span class="spacer"></span>
      <label class="zoom">
        Zoom
        <input type="range" min="12" max="200" step="2" bind:value={pps} onchange={save} />
      </label>
      <button class="ghost" onclick={clearAll} disabled={!clips.length}>Clear</button>
      {#if toast}<span class="ok">{toast}</span>{/if}
    </div>

    <div class="lane" bind:this={laneEl} onpointermove={onLaneMove} onpointerup={onLaneUp}
         onpointerleave={onLaneUp} role="presentation">
      <div class="inner" style="width:{span * pps}px">
        <div class="ruler" onpointerdown={scrub} role="presentation">
          {#each ruler as t (t)}
            <span class="tick" style="left:{t * pps}px">
              <i></i><em>{t % 5 === 0 ? `${t}s` : ""}</em>
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
              style="left:{c.start * pps}px;top:{c.track * ROW_H + 3}px;width:{Math.max(
                14,
                c.dur * pps - 2,
              )}px;height:{ROW_H - 6}px"
              onpointerdown={(e) => onClipDown(e, c)}
              role="button"
              tabindex="0"
              title={`${c.name} — drag to move, double-click to remove`}
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

    {#if !clips.length}
      <p class="hint">Pick sounds on the left to stack them on the timeline — overlapping clips mix together.</p>
    {/if}
  </section>
</div>

<style>
  .tl {
    flex: 1;
    display: flex;
    min-height: 0;
    min-width: 0;
  }
  /* sound picker */
  .picker {
    width: 216px;
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
  }
  .pick-head { padding: 8px; }
  .pick-head input {
    width: 100%;
    box-sizing: border-box;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    padding: 6px 9px;
    font-size: 12px;
  }
  .pick-list { flex: 1; overflow-y: auto; padding: 0 4px 8px; }
  .pick {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    background: none;
    border: 0;
    color: var(--text-2);
    text-align: left;
    padding: 6px 8px;
    border-radius: 7px;
    cursor: pointer;
    font-size: 12px;
  }
  .pick:hover { background: var(--panel-2); color: var(--text); }
  .pick .pn { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick .add { opacity: 0; color: var(--accent-2); }
  .pick:hover .add { opacity: 1; }
  .empty { color: var(--muted); font-size: 12px; padding: 10px; margin: 0; }

  /* arranger */
  .arrange { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .transport {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .play {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 999px;
    border: 0;
    cursor: pointer;
    color: #fff;
    background: linear-gradient(180deg, var(--accent-2), var(--accent));
  }
  .ghost {
    background: var(--panel-3);
    color: var(--text-2);
    border: 0;
    border-radius: 7px;
    padding: 6px 9px;
    cursor: pointer;
    display: inline-flex;
  }
  .ghost.on { color: var(--accent-2); box-shadow: inset 0 0 0 1px var(--accent); }
  .ghost:disabled { opacity: 0.45; cursor: default; }
  .time { font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; min-width: 92px; }
  .spacer { flex: 1; }
  .zoom { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--muted); }
  .zoom input { width: 90px; }
  .ok { color: var(--ok); font-size: 11px; }

  .lane { flex: 1; overflow: auto; min-height: 0; position: relative; }
  .inner { position: relative; }
  .ruler {
    position: relative;
    height: 22px;
    border-bottom: 1px solid var(--border);
    cursor: pointer;
    user-select: none;
  }
  .tick { position: absolute; top: 0; bottom: 0; }
  .tick i { position: absolute; left: 0; top: 12px; bottom: 0; width: 1px; background: var(--border); }
  .tick em {
    position: absolute;
    left: 3px;
    top: 3px;
    font-style: normal;
    font-size: 9px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .rows { position: relative; }
  .row { position: absolute; left: 0; right: 0; border-bottom: 1px solid rgba(127, 127, 127, 0.1); }
  .row.alt { background: rgba(127, 127, 127, 0.045); }
  .clip {
    position: absolute;
    border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 26%, var(--panel-2));
    border: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
    color: var(--text);
    display: flex;
    align-items: center;
    padding: 0 16px 0 8px;
    cursor: grab;
    overflow: hidden;
    touch-action: none;
  }
  .clip.sel { border-color: var(--accent-2); box-shadow: 0 0 0 2px var(--accent-ring); }
  .clip:active { cursor: grabbing; }
  .clip .cn {
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .clip .del {
    position: absolute;
    right: 2px;
    top: 2px;
    border: 0;
    background: none;
    color: var(--muted);
    cursor: pointer;
    padding: 1px;
    display: grid;
    place-items: center;
    opacity: 0;
  }
  .clip:hover .del { opacity: 1; }
  .clip .del:hover { color: var(--danger); }
  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--accent-2);
    pointer-events: none;
  }
  .hint { color: var(--muted); font-size: 12px; padding: 10px 14px; margin: 0; }
</style>
