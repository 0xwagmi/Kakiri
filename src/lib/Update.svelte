<script lang="ts">
  /**
   * "A new version is out." Shown once per release: dismissing remembers the
   * version, so nobody is nagged on every launch, but the next one still appears.
   */
  import { openUrl } from "@tauri-apps/plugin-opener";

  import CatMark from "./CatMark.svelte";

  export let version = "";
  export let notes = "";
  export let url = "";
  export let onLater: () => void = () => {};

  let leaving = false;
  let busy = false;

  async function get() {
    if (!url || busy) return;
    busy = true;
    try {
      await openUrl(url);
    } catch {
      /* the browser is on its own */
    }
    busy = false;
    leave();
  }

  function leave() {
    if (leaving) return;
    leaving = true;
    setTimeout(onLater, 140);
  }
</script>

<div
  class="scrim"
  class:leaving
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && leave()}
  onkeydown={(e) => e.key === "Escape" && leave()}
>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="card" role="dialog" tabindex="-1" aria-labelledby="update-title">
    <CatMark size={56} />
    <h2 id="update-title">Kakiri {version} is out</h2>
    <p class="lead">A newer build than the one you are running. It takes a minute.</p>
    {#if notes}<p class="notes">{notes}</p>{/if}
    <div class="acts">
      <button class="ghost" onclick={leave}>Later</button>
      <button class="primary" onclick={() => void get()} disabled={busy}>
        {busy ? "Opening…" : "Get the update"}
      </button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: rgb(0 0 0 / 0.55);
    backdrop-filter: blur(3px);
    display: grid;
    place-items: center;
    animation: fade 160ms ease;
  }
  .scrim.leaving { animation: fade 140ms ease reverse forwards; }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }

  .card {
    width: min(400px, calc(100vw - 40px));
    padding: 22px 24px 18px;
    border-radius: 16px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 22px 60px rgb(0 0 0 / 0.5);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    animation: rise 180ms cubic-bezier(0.2, 0.7, 0.3, 1);
  }
  @keyframes rise {
    from { transform: translateY(6px) scale(0.99); opacity: 0; }
    to { transform: none; opacity: 1; }
  }

  h2 {
    margin: 8px 0 0;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--text);
  }
  .lead { margin: 0; font-size: 12.5px; line-height: 1.5; color: var(--text-2); }
  .notes {
    margin: 2px 0 0;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text-3);
    max-height: 66px;
    overflow: hidden;
  }

  .acts {
    display: flex;
    gap: 8px;
    align-self: flex-end;
    margin-top: 14px;
  }
  .ghost,
  .primary {
    padding: 7px 14px;
    border-radius: 9px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
  }
  .ghost {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-2);
  }
  .ghost:hover { color: var(--text); border-color: var(--accent); }
  .primary {
    border: 0;
    background: linear-gradient(180deg, var(--accent-2, #a48bff), var(--accent, #8b7bff));
    color: #fff;
  }
  .primary:hover:not(:disabled) { filter: brightness(1.08); }
  .primary:disabled { opacity: 0.6; cursor: default; }
  .ghost:focus-visible,
  .primary:focus-visible { outline: 2px solid var(--accent-2, #a48bff); outline-offset: 2px; }
</style>
