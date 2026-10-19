<script lang="ts">
  import { Headphones, Play } from "$lib/icons";

  export type Sound = {
    id: string;
    path: string;
    name: string;
    ext: string;
    duration_secs: number | null;
  };

  export let s: Sound;
  export let play: (s: Sound) => void;
  export let preview: (s: Sound) => void;
  export let fmtDur: (d: number | null) => string;
</script>

<div
  class="card"
  role="button"
  tabindex="0"
  title="Play into the call"
  onclick={() => play(s)}
  onkeydown={(e) => e.key === "Enter" && play(s)}
>
  <button
    class="prev"
    title="Preview locally — not sent to the call"
    onclick={(e) => {
      e.stopPropagation();
      preview(s);
    }}><Headphones size={14} /></button>
  <span class="play"><Play size={15} /></span>
  <span class="name">{s.name}</span>
  <span class="meta">{s.ext.toUpperCase()}{s.duration_secs != null ? " · " + fmtDur(s.duration_secs) : ""}</span>
</div>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px;
    text-align: left;
    color: var(--text);
    min-height: 74px;
    cursor: pointer;
  }
  .card:hover { border-color: var(--accent); background: var(--panel-2); }
  .play { font-size: 15px; color: var(--accent-2); }
  .name {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .meta { font-size: 11px; color: var(--muted); }
  .prev {
    position: absolute;
    top: 6px;
    right: 6px;
    background: none;
    color: var(--muted);
    border: 0;
    padding: 2px 4px;
    cursor: pointer;
    opacity: 0;
    font-size: 13px;
  }
  .card:hover .prev { opacity: 1; }
  .prev:hover { color: var(--accent-2); }
</style>
