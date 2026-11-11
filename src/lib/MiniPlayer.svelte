<script lang="ts">
  /**
   * One bar for the whole app: what is playing right now, and the few controls
   * worth reaching from any view. Online has its own full player with a queue, so
   * this only appears while you are somewhere else.
   */
  import { Bookmark, Music, Pause, Play, SkipBack, SkipForward, Square } from "$lib/icons";

  export let track: { title: string; artist: string; artwork: string | null } | null = null;
  export let count = 0;
  export let label = "";
  export let progress = 0;
  export let paused = false;
  export let saved = false;

  export let onToggle: () => void = () => {};
  export let onStop: () => void = () => {};
  export let onSeek: (fraction: number) => void = () => {};
  export let onOpen: () => void = () => {};
  /** local tracks have a queue; an Online track keeps its own next/prev */
  export let onPrev: (() => void) | null = null;
  export let onNext: (() => void) | null = null;

  function seek(ev: MouseEvent) {
    const el = ev.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    onSeek(Math.min(1, Math.max(0, (ev.clientX - rect.left) / rect.width)));
  }
</script>

<div class="mini">
  <button class="who" onclick={onOpen} title="Show in Online">
    <span class="art">
      {#if track?.artwork}
        <img
          src={track.artwork}
          alt=""
          onerror={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
        />
      {:else}
        <Music size={16} />
      {/if}
    </span>
    <span class="meta">
      <span class="title">{track ? track.title : `${count} ${count === 1 ? "sound" : "sounds"} playing`}</span>
      <span class="sub">{track ? track.artist || "Online" : label}</span>
    </span>
  </button>

  {#if track}
    <span class="bar" onclick={seek} role="presentation">
      <span class="fill" style="width:{Math.max(0, Math.min(100, progress * 100))}%"></span>
    </span>
  {/if}

  <div class="acts">
    {#if track && saved}
      <span class="kept" title="In your library"><Bookmark size={13} fill="currentColor" /></span>
    {/if}
    {#if onPrev}
      <button class="ctl" onclick={onPrev} title="Previous"><SkipBack size={13} /></button>
    {/if}
    {#if track}
      <button class="ctl" onclick={onToggle} title={paused ? "Resume" : "Pause"}>
        {#if paused}<Play size={14} />{:else}<Pause size={14} />{/if}
      </button>
    {/if}
    {#if onNext}
      <button class="ctl" onclick={onNext} title="Next"><SkipForward size={13} /></button>
    {/if}
    <button class="ctl stop" onclick={onStop} title="Stop">
      <Square size={12} fill="currentColor" stroke-width={0} />
    </button>
  </div>
</div>

<style>
  /* a floating pane of glass: the blur pulls colour from whatever is behind it,
     the inner highlight gives the top edge something to catch */
  .mini {
    position: relative;
    flex: none;
    height: 52px;
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 0 12px 10px;
    padding: 0 12px 0 10px;
    border-radius: 15px;
    background: linear-gradient(180deg, rgb(255 255 255 / 0.075), rgb(255 255 255 / 0.028));
    border: 1px solid rgb(255 255 255 / 0.09);
    -webkit-backdrop-filter: blur(22px) saturate(170%);
    backdrop-filter: blur(22px) saturate(170%);
    box-shadow:
      inset 0 1px 0 rgb(255 255 255 / 0.14),
      inset 0 -1px 0 rgb(0 0 0 / 0.18),
      0 8px 24px rgb(0 0 0 / 0.28);
    overflow: hidden;
    animation: fade 160ms ease;
  }
  @keyframes fade {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .who {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 0 1 260px;
    border: 0;
    background: none;
    padding: 4px 6px;
    border-radius: 8px;
    text-align: left;
    cursor: pointer;
    color: var(--text);
  }
  .who:hover { background: var(--panel-3, rgb(255 255 255 / 0.06)); }

  .art {
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 6px;
    overflow: hidden;
    display: grid;
    place-items: center;
    background: var(--panel-3, rgb(255 255 255 / 0.06));
    color: var(--text-3);
  }
  .art img { width: 100%; height: 100%; object-fit: cover; }

  .meta { display: flex; flex-direction: column; min-width: 0; }
  .title {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bar {
    flex: 1;
    height: 3px;
    min-width: 80px;
    border-radius: 2px;
    background: var(--panel-3, rgb(255 255 255 / 0.1));
    overflow: hidden;
    cursor: pointer;
  }
  .fill { display: block; height: 100%; background: var(--accent, #8b7bff); }

  .acts { display: flex; align-items: center; gap: 6px; flex: none; }
  .kept { color: var(--accent, #8b7bff); display: inline-flex; padding-right: 2px; }

  .ctl {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: var(--panel-3, rgb(255 255 255 / 0.06));
    color: var(--text);
    cursor: pointer;
  }
  .ctl:hover { background: var(--panel-4, rgb(255 255 255 / 0.12)); }
  .ctl.stop { color: var(--text-2); }

  @media (prefers-reduced-motion: reduce) {
    .mini { animation: none; }
  }
</style>
