<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Headphones, Music } from "$lib/icons";

  type Sound = {
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
    gap: 12px;
    padding: 8px 10px;
    border-radius: 10px;
    color: var(--text);
    cursor: pointer;
    min-height: 64px;
    transition: background-color 0.12s ease;
  }
  .card:hover {
    background: var(--panel-2);
  }
  .card:focus-visible {
    background: var(--panel-2);
    outline: none;
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .thumb {
    width: 46px;
    height: 46px;
    flex: none;
    border-radius: 9px;
    display: grid;
    place-items: center;
    overflow: hidden;
    color: var(--accent);
    background: var(--panel-3);
    background: color-mix(in srgb, var(--accent) 13%, transparent);
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
    gap: 3px;
    min-width: 0;
    padding-right: 28px;
  }
  .name {
    font-size: 13px;
    line-height: 1.25;
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
    top: 8px;
    right: 8px;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: 999px;
    background: var(--panel-3);
    color: var(--text-2);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.12s ease,
      color 0.12s ease;
  }
  .card:hover .prev,
  .card:focus-within .prev {
    opacity: 1;
  }
  .prev:hover {
    color: var(--accent-2);
  }
</style>
