<script lang="ts">
  import { Check } from "$lib/icons";
  import { THEMES } from "$lib/themes";

  export let theme: string;
  export let onTheme: (t: string) => void;
  export let onClearCache: () => void;

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
    <h2>Maintenance</h2>
    <div class="row">
      <button class="ghost" onclick={clearCache}>Clear decoded audio + cover caches</button>
      {#if cleared}<span class="ok">{cleared}</span>{/if}
    </div>
    <p class="muted">Frees memory used for decoded PCM and embedded cover art.</p>
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
