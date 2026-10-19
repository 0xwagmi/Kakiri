<script lang="ts">
  import { Check } from "$lib/icons";
  import { THEMES } from "$lib/themes";

  export let theme: string;
  export let onTheme: (t: string) => void;
  export let onClearCache: () => void;
  export let mic: MicStatus | null = null;
  export let micBusy = false;
  export let onInstallMic: () => void = () => {};
  export let onRemoveMic: () => void = () => {};

  type MicStatus = {
    present: boolean;
    source: string;
    sink: string;
    persistent: boolean;
    can_manage: boolean;
  };

  let cleared = "";

  function clearCache() {
    onClearCache();
    cleared = "Caches cleared";
    setTimeout(() => {
      if (cleared === "Caches cleared") cleared = "";
    }, 2000);
  }
</script>

<div class="settings">
  <section>
    <h2>Appearance</h2>
    <p class="muted">Theme</p>
    <div class="themes">
      {#each THEMES as t (t.id)}
        <button class="theme" class:active={theme === t.id} onclick={() => onTheme(t.id)}>
          <span class="swatches">
            {#each t.swatch as c}<span class="dot" style="background:{c}"></span>{/each}
          </span>
          <span class="tname">{t.name}</span>
          {#if theme === t.id}<span class="tick"><Check size={13} /></span>{/if}
        </button>
      {/each}
    </div>
  </section>

  <section>
    <h2>Virtual microphone</h2>
    {#if !mic}
      <p class="muted">Checking…</p>
    {:else if !mic.can_manage}
      <p class="muted">
        Kakiri can't create a virtual microphone on this platform. Install
        <b>VB-CABLE</b> (Windows) or <b>BlackHole</b> (macOS), then choose
        <code>CABLE Input</code> / <code>BlackHole 2ch</code> as the output.
      </p>
    {:else}
      <p class="muted">
        {#if mic.present}Running{:else}Not running{/if} ·
        {#if mic.persistent}persists across restarts{:else}temporary (stops with Kakiri){/if}.<br />
        Discord should use <b>{mic.source}</b> as its input device.
      </p>
      <div class="row">
        {#if mic.persistent}
          <button class="ghost" onclick={onRemoveMic} disabled={micBusy}>
            Remove persistent mic
          </button>
        {:else}
          <button onclick={onInstallMic} disabled={micBusy}>Keep across restarts</button>
        {/if}
      </div>
      <p class="muted">
        Keeping it installs a PipeWire config, so the mic exists from login on and
        Discord always has a valid input — even while Kakiri is closed. Removing it
        takes effect once PipeWire restarts.
      </p>
    {/if}
  </section>

  <section>
    <h2>Maintenance</h2>
    <div class="row">
      <button class="ghost" onclick={clearCache}>Clear cover-art cache</button>
      {#if cleared}<span class="ok">{cleared}</span>{/if}
    </div>
    <p class="muted">Frees memory used for embedded cover art.</p>
  </section>

  <section>
    <h2>About</h2>
    <p class="muted">
      <b>Kakiri Soundboard</b> — early alpha.<br />
      Audio: Linux (native PipeWire) · Windows/macOS (cpal).<br />
      MIT licensed.
    </p>
  </section>
</div>

<style>
  .settings {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    max-width: 640px;
  }
  section { margin-bottom: 22px; }
  h2 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
    margin: 0 0 8px;
  }
  .themes { display: flex; gap: 10px; flex-wrap: wrap; }
  .theme {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px 14px;
    cursor: pointer;
    color: var(--text);
    min-width: 120px;
  }
  .theme.active { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-ring); }
  .swatches { display: flex; gap: 4px; }
  .dot { width: 18px; height: 18px; border-radius: 50%; border: 1px solid rgba(255, 255, 255, 0.12); }
  .tname { font-size: 13px; }
  .tick { position: absolute; top: 8px; right: 8px; color: var(--accent-2); }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .ok { color: var(--ok); font-size: 12px; }
  .muted { color: var(--muted); font-size: 12px; line-height: 1.5; }
</style>
