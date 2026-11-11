<script lang="ts">
  import { Check, Eye, EyeOff } from "$lib/icons";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  import { CircleHelp } from "$lib/icons";
  import { THEMES } from "$lib/themes";

  export let theme: string;
  export let onTheme: (t: string) => void;
  export let onClearCache: () => void;
  export let mic: MicStatus | null = null;
  export let micBusy = false;
  export let onInstallMic: () => void = () => {};
  export let onRemoveMic: () => void = () => {};
  export let companion = true;
  export let onCompanion: (v: boolean) => void = () => {};
  export let discord = false;
  export let onDiscord: (v: boolean) => void = () => {};
  export let discordOk = true;
  export let lastfmKey = "";
  export let lastfmUser = "";
  export let onLastfm: (key: string, user: string) => void = () => {};
  export let denoise = false;
  export let onDenoise: (v: boolean) => void = () => {};

  type MicStatus = {
    present: boolean;
    source: string;
    sink: string;
    persistent: boolean;
    can_manage: boolean;
  };

  let cleared = "";
  // credentials start hidden; the eye reveals them for a moment of checking
  let showKey = false;
  let showUser = false;
  let secretState = "";
  /** the "?" toggles: explanation on demand instead of a wall of text */
  let helpOnline = false;
  let helpDiscord = false;

  // the key is a credential: it lives in the backend's private store, not in prefs
  onMount(async () => {
    try {
      const key = (await invoke<string | null>("secret_get", { name: "lastfm.key" })) ?? "";
      const user = (await invoke<string | null>("secret_get", { name: "lastfm.user" })) ?? "";
      if (key || user) onLastfm(key, user || lastfmUser);
      secretState = key ? "stored" : "";
    } catch {
      /* no backend (browser preview) */
    }
  });

  async function saveSecret(key: string, user: string) {
    try {
      await invoke("secret_set", { name: "lastfm.key", value: key });
      await invoke("secret_set", { name: "lastfm.user", value: user });
      secretState = key ? "stored" : "";
    } catch {
      /* ignore */
    }
  }

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
    <h2>
      Online
      <button
        class="help"
        class:on={helpOnline}
        aria-expanded={helpOnline}
        title="What the Last.fm key is for"
        onclick={() => (helpOnline = !helpOnline)}
        >
          <CircleHelp size={13} />
        </button>
    </h2>
    {#if helpOnline}
      <p class="muted">
        Charts and search work without an account. A key adds your loved tracks and tag stations.
        Make one at
        <button class="inline" onclick={() => openUrl("https://www.last.fm/api/account/create")}
          >last.fm/api</button
        > and paste it here. Only the <em>API key</em> is needed, not the shared secret.
      </p>
      <p class="muted small">
        The key sits in its own scrambled file in Wagmify's folder, not in your settings. There is no
        system keyring yet, so treat it like any other secret on this machine.
      </p>
    {/if}
    <div class="fields">
        <label>
          <span>Last.fm API key</span>
          <div class="secret">
            <input
              type={showKey ? "text" : "password"}
              placeholder="paste your 32-character API key"
              autocomplete="off"
              spellcheck="false"
              value={lastfmKey}
              oninput={(e) => { const v = e.currentTarget.value; onLastfm(v, lastfmUser); void saveSecret(v, lastfmUser); }}
            />
            <button
              class="reveal"
              onclick={() => (showKey = !showKey)}
              title={showKey ? "Hide API key" : "Show API key"}
              aria-label={showKey ? "Hide API key" : "Show API key"}
            >
              {#if showKey}<EyeOff size={13} />{:else}<Eye size={13} />{/if}
            </button>
          </div>
        </label>
        <label>
          <span>Last.fm username</span>
          <div class="secret">
            <input
              type={showUser ? "text" : "password"}
              placeholder="your profile name, for loved tracks"
              autocomplete="off"
              spellcheck="false"
              value={lastfmUser}
              oninput={(e) => { const v = e.currentTarget.value; onLastfm(lastfmKey, v); void saveSecret(lastfmKey, v); }}
            />
            <button
              class="reveal"
              onclick={() => (showUser = !showUser)}
              title={showUser ? "Hide username" : "Show username"}
              aria-label={showUser ? "Hide username" : "Show username"}
            >
              {#if showUser}<EyeOff size={13} />{:else}<Eye size={13} />{/if}
            </button>
          </div>
        </label>
    </div>
  </section>

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
    <h2>Companion</h2>
    <div class="row">
      <button
        class="toggle"
        class:on={companion}
        role="switch"
        aria-checked={companion}
        onclick={() => onCompanion(!companion)}
      >
        <span class="track"><span class="knob"></span></span>
        <span>{companion ? "Shown" : "Hidden"}</span>
      </button>
    </div>
    <p class="muted">She idles in the corner and reacts to playback. Purely cosmetic.</p>
  </section>

  <section>
    <h2>
      Discord
      <button
        class="help"
        class:on={helpDiscord}
        aria-expanded={helpDiscord}
        title="What this shows"
        onclick={() => (helpDiscord = !helpDiscord)}
        >
          <CircleHelp size={13} />
        </button>
    </h2>
    <div class="row">
      <button
        class="toggle"
        class:on={discord}
        role="switch"
        aria-checked={discord}
        onclick={() => onDiscord(!discord)}
      >
        <span class="track"><span class="knob"></span></span>
        <span>{discord ? "Shown in your profile" : "Off"}</span>
      </button>
    </div>
    {#if discord && !discordOk}
      <p class="muted">Discord is closed, so nothing is showing. It appears once Discord is open.</p>
    {:else if helpDiscord}
      <p class="muted">
        Your Discord profile shows the track you are playing. Wagmify talks to Discord on this
        machine only. No account is linked and nothing is sent anywhere else.
      </p>
    {/if}
  </section>

  <section>
    <h2>Virtual microphone</h2>
    <div class="row" style="margin-top:10px">
      <button
        class="toggle"
        class:on={denoise}
        role="switch"
        aria-checked={denoise}
        onclick={() => onDenoise(!denoise)}
      >
        <span class="track"><span class="knob"></span></span>
        <span>Noise reduction (RNNoise)</span>
      </button>
    </div>
    <p class="muted">
      Cleans your voice before it reaches the virtual mic, so Discord's own suppression can stay
      off. It would otherwise gate the sounds you play.
    </p>
    {#if !mic}
      <p class="muted">Checking…</p>
    {:else if !mic.can_manage}
      <p class="muted">
        Wagmify can't create a virtual microphone on this platform. Install
        <b>VB-CABLE</b> (Windows) or <b>BlackHole</b> (macOS), then choose
        <code>CABLE Input</code> / <code>BlackHole 2ch</code> as the output.
      </p>
    {:else}
      <p class="muted">
        {#if mic.present}Running{:else}Not running{/if} ·
        {#if mic.persistent}persists across restarts{:else}temporary (stops with Wagmify){/if}.<br />
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
        Discord always has a valid input, even while Wagmify is closed. Removing it
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
      <b>Wagmify Soundboard</b>, early alpha.<br />
      Audio: Linux (native PipeWire) · Windows/macOS (cpal).<br />
      MIT licensed.
    </p>
  </section>
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 420px;
  }
  .fields label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .fields input {
    background: var(--panel-2);
    border: 0;
    border-radius: 7px;
    color: var(--text);
    font-size: 12.5px;
    padding: 7px 10px;
    outline: none;
  }
  .fields input:focus { box-shadow: inset 0 0 0 1px var(--accent-dim); }
  .secret { position: relative; display: flex; align-items: center; }
  .secret input { flex: 1 1 auto; padding-right: 32px; }
  .secret input::placeholder { letter-spacing: 0; }
  .reveal {
    position: absolute;
    right: 4px;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--text-3);
    cursor: pointer;
  }
  .reveal:hover { background: var(--panel-3); color: var(--text); }
  .inline {
    border: 0;
    background: none;
    color: var(--accent, #8b7bff);
    font: inherit;
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
  }

  .settings {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
  }
  /* Constrain the reading width, but let the scroll container stay full-bleed
     so its scrollbar sits at the window edge, not mid-screen. */
  section {
    max-width: 640px;
    margin-bottom: 22px;
  }
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
  /* borderless switch: only the knob and the track colour move */
  .toggle {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 13px;
    cursor: pointer;
  }
  .toggle.on { color: var(--text); }
  .track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 999px;
    background: var(--border);
    transition: background 140ms ease;
  }
  .toggle.on .track { background: var(--accent); }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    transition: transform 140ms ease;
  }
  .toggle.on .knob { transform: translateX(14px); }
  .toggle:focus-visible { outline: 2px solid var(--accent-2); outline-offset: 4px; border-radius: 6px; }
  .ok { color: var(--ok); font-size: 12px; }
  .muted.small { font-size: 11px; line-height: 1.5; opacity: 0.9; }

  /* an explanation on request: a quiet "?" next to the heading */
  .help {
    width: 16px;
    height: 16px;
    margin-left: 6px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 600;
    line-height: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    vertical-align: middle;
    cursor: pointer;
    transition: color 120ms ease, background 120ms ease;
  }
  .help:hover { color: var(--text); background: var(--panel-3, rgb(255 255 255 / 0.06)); }
  .help.on {
    color: var(--accent, #8b7bff);
    background: color-mix(in srgb, var(--accent, #8b7bff) 16%, transparent);
  }
  .muted { color: var(--muted); font-size: 12px; line-height: 1.5; }
</style>
