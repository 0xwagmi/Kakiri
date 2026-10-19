<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Headphones, Music } from "$lib/icons";

  export type Sound = {
    id: string;
    path: string;
    name: string;
    title: string | null;
    artist: string | null;
    ext: string;
    duration_secs: number | null;
    has_cover: boolean;
  };

  export let s: Sound;
  export let play: (s: Sound) => void;
  export let preview: (s: Sound) => void;
  export let fmtDur: (d: number | null) => string;

  let cover: string | null = null;

  onMount(async () => {
    if (!s.has_cover) return;
    try {
      cover = await invoke<string | null>("artwork", { path: s.path });
    } catch {
      /* ignore a missing/broken picture */
    }
  });

  $: label = s.title ?? s.name;
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
    }}><Headphones size={14} /></button
  >
  <div class="thumb">
    {#if cover}<img src={cover} alt="" />{:else}<Music size={22} />{/if}
  </div>
  <div class="info">
    <span class="name">{label}</span>
    <span class="meta">
      {#if s.artist}{s.artist} · {/if}{s.ext.toUpperCase()}{s.duration_secs != null
        ? " · " + fmtDur(s.duration_secs)
        : ""}
    </span>
  </div>
</div>

<style>
  .card {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px;
    text-align: left;
    color: var(--text);
    cursor: pointer;
    min-height: 66px;
  }
  .card:hover {
    border-color: var(--accent);
    background: var(--panel-2);
  }
  .thumb {
    width: 44px;
    height: 44px;
    flex: none;
    border-radius: 8px;
    background: var(--bg);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
    overflow: hidden;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 11px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
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
  .card:hover .prev {
    opacity: 1;
  }
  .prev:hover {
    color: var(--accent-2);
  }
</style>
