<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import SoundCard from "$lib/SoundCard.svelte";
  import Pads from "$lib/Pads.svelte";
  import Select from "$lib/Select.svelte";
  import { Mic, Headphones, Music, Square, TriangleAlert, X, Plus, Repeat } from "$lib/icons";

  type Sound = {
    id: string;
    path: string;
    name: string;
    ext: string;
    duration_secs: number | null;
  };
  type Folder = {
    name: string;
    path: string;
    children: Folder[];
    sounds: Sound[];
    total_sounds: number;
  };
  type Node = { class: string; name: string; description: string; virtual_mic: boolean };
  type Device = { index: number; name: string; channels: number; rate: number };
  type MicStatus = { present: boolean; source: string; sink: string; can_create: boolean };
  type Voice = {
    id: number;
    label: string;
    progress: number;
    gain: number;
    looped: boolean;
    route: string;
  };

  const LS_ROOTS = "sb.roots";
  const LS_PREFS = "sb.prefs";

  let roots: string[] = [];
  let folders: Folder[] = [];
  let selected: Folder | null = null;
  let mic: MicStatus | null = null;
  let nodes: Node[] = [];
  let outputs: Device[] = [];
  let target = "Soundboard-Sink";
  let monitorOn = false;
  let monitorTarget = "";
  let micMix = false;
  let micSource = "";
  let micGain = 1.0;
  let retrigger = "layer";

  const retriggerOpts = [
    { value: "layer", label: "Layer" },
    { value: "choke", label: "Choke" },
    { value: "toggle", label: "Toggle" },
  ];
  let loop = false;
  let masterGain = 0.85;
  let search = "";
  let view: "library" | "pads" = "library";
  let voices: Voice[] = [];
  let scrub: { id: number; pos: number } | null = null;
  let gainEdit: Record<number, number> = {};
  let busy = false;
  let toast = "";
  let poll: ReturnType<typeof setInterval>;

  function flash(msg: string) {
    toast = msg;
    setTimeout(() => {
      if (toast === msg) toast = "";
    }, 2500);
  }

  function savePrefs() {
    localStorage.setItem(
      LS_PREFS,
      JSON.stringify({ target, monitorOn, monitorTarget, loop, masterGain, micMix, micSource, micGain, retrigger }),
    );
  }

  function flatten(fs: Folder[], depth = 0, acc: { f: Folder; depth: number }[] = []) {
    for (const f of fs) {
      acc.push({ f, depth });
      flatten(f.children, depth + 1, acc);
    }
    return acc;
  }

  function allSounds(fs: Folder[], acc: Sound[] = []) {
    for (const f of fs) {
      acc.push(...f.sounds);
      allSounds(f.children, acc);
    }
    return acc;
  }

  function fmtDur(s: number | null) {
    if (s == null) return "";
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return m > 0 ? `${m}:${String(sec).padStart(2, "0")}` : `${s.toFixed(1)}s`;
  }

  function findFolder(fs: Folder[], path: string): Folder | null {
    for (const f of fs) {
      if (f.path === path) return f;
      const hit = findFolder(f.children, path);
      if (hit) return hit;
    }
    return null;
  }

  const sinks = () => nodes.filter((n) => n.class === "Audio/Sink");
  const sources = () => nodes.filter((n) => n.class.startsWith("Audio/Source"));

  /** Best guess for the voice source: a processed one (Easy Effects) if present. */
  function preferredSource(): string {
    const s = sources();
    const ee = s.find((n) => /easy effects/i.test(n.description || "") || /ee_|easyeffects/i.test(n.name));
    return (ee ?? s[0])?.name ?? "";
  }

  function labelFor(n: Node) {
    if (n.name === "Soundboard-Sink") return "Soundboard Mic (Discord)";
    return `${n.description || n.name}${n.virtual_mic ? " ★" : ""}`;
  }

  async function refreshLibrary(keepSelection = true) {
    const prev = selected?.path;
    folders = await invoke<Folder[]>("library_scan", { roots });
    selected = (keepSelection && prev && findFolder(folders, prev)) || folders[0] || null;
  }

  async function refreshDevices() {
    mic = await invoke<MicStatus>("virtual_mic_status");
    const d = await invoke<{ outputs: Device[]; nodes: Node[] }>("list_devices");
    nodes = d.nodes;
    outputs = d.outputs;
  }

  async function addFolder() {
    const picked = await open({ directory: true, multiple: true });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    for (const p of list) if (!roots.includes(p)) roots.push(p);
    localStorage.setItem(LS_ROOTS, JSON.stringify(roots));
    busy = true;
    try {
      await refreshLibrary(false);
      flash(`Added ${list.length} folder${list.length > 1 ? "s" : ""}`);
    } finally {
      busy = false;
    }
  }

  async function removeRoot(path: string) {
    roots = roots.filter((r) => r !== path);
    localStorage.setItem(LS_ROOTS, JSON.stringify(roots));
    await refreshLibrary();
  }

  async function play(s: Sound) {
    try {
      const id = await invoke<number | null>("play_sound", {
        path: s.path,
        gain: masterGain,
        looped: loop,
        route: monitorOn ? "both" : "mic",
        retrigger,
      });
      if (id == null && retrigger === "toggle") flash(`stopped ${s.name}`);
    } catch (e) {
      flash(String(e));
    }
  }

  /** Play a sound into the call by path (used by pads / hotkeys). */
  async function playPath(path: string, _label: string) {
    try {
      await invoke("play_sound", {
        path,
        gain: masterGain,
        looped: loop,
        route: "mic",
        retrigger,
      });
    } catch (e) {
      flash(String(e));
    }
  }

  /** Play only to your own output — the call never hears it. */
  async function preview(s: Sound) {
    try {
      if (!monitorOn) {
        monitorOn = true;
        await invoke("set_monitor", { enabled: true, target: monitorTarget || null });
      }
      await invoke("play_sound", {
        path: s.path,
        gain: masterGain,
        looped: false,
        route: "monitor",
      });
    } catch (e) {
      flash(String(e));
    }
  }

  async function stopAll() {
    await invoke("stop_all");
  }

  async function stopVoice(id: number) {
    await invoke("stop_voice", { id });
  }

  async function seekVoice(id: number, position: number) {
    try {
      await invoke("seek_voice", { id, position });
    } catch (e) {
      flash(String(e));
    }
  }

  async function setVoiceGain(id: number, gain: number) {
    try {
      await invoke("set_voice_gain", { id, gain });
    } catch (e) {
      flash(String(e));
    }
  }

  async function applyTarget() {
    savePrefs();
    try {
      await invoke("set_output_target", { target });
      flash(`Mic output → ${target || "default sink"}`);
    } catch (e) {
      flash(String(e));
    }
  }

  async function useSoundboardMic() {
    target = "Soundboard-Sink";
    await applyTarget();
  }

  async function toggleMonitor() {
    monitorOn = !monitorOn;
    savePrefs();
    try {
      await invoke("set_monitor", { enabled: monitorOn, target: monitorTarget || null });
      flash(monitorOn ? "Monitor on" : "Monitor off");
    } catch (e) {
      flash(String(e));
    }
  }

  async function applyMonitorTarget() {
    savePrefs();
    if (monitorOn) {
      try {
        await invoke("set_monitor", { enabled: true, target: monitorTarget || null });
      } catch (e) {
        flash(String(e));
      }
    }
  }

  /** Toggle mixing your live voice into the virtual mic (app does the mix). */
  async function toggleMic() {
    micMix = !micMix;
    if (micMix && !micSource) micSource = preferredSource();
    savePrefs();
    try {
      const active = await invoke<boolean>("set_input", {
        source: micMix ? micSource : null,
        gain: micGain,
      });
      flash(active ? "Mic mixed in" : "Mic off");
    } catch (e) {
      flash(String(e));
    }
  }

  async function applyMic() {
    savePrefs();
    if (!micMix) return;
    try {
      await invoke("set_input", { source: micSource || null, gain: micGain });
    } catch (e) {
      flash(String(e));
    }
  }

  async function createMic() {
    busy = true;
    try {
      mic = await invoke<MicStatus>("create_virtual_mic");
      await refreshDevices();
      flash("Virtual mic ready");
    } catch (e) {
      flash(String(e));
    } finally {
      busy = false;
    }
  }

  async function tone() {
    try {
      await invoke("play_test_tone", { route: monitorOn && !mic?.present ? "monitor" : "mic" });
      flash("Test tone sent");
    } catch (e) {
      flash(String(e));
    }
  }

  async function pollVoices() {
    try {
      voices = await invoke<Voice[]>("active_voices");
    } catch {
      /* engine busy */
    }
  }

  onMount(async () => {
    const storedPrefs = localStorage.getItem(LS_PREFS);
    if (storedPrefs) {
      try {
        const p = JSON.parse(storedPrefs);
        target = p.target ?? target;
        monitorOn = p.monitorOn ?? monitorOn;
        monitorTarget = p.monitorTarget ?? monitorTarget;
        loop = p.loop ?? loop;
        masterGain = p.masterGain ?? masterGain;
        micMix = p.micMix ?? micMix;
        micSource = p.micSource ?? micSource;
        micGain = p.micGain ?? micGain;
        retrigger = p.retrigger ?? retrigger;
      } catch {
        /* ignore */
      }
    }
    const stored = localStorage.getItem(LS_ROOTS);
    if (stored) {
      try {
        roots = JSON.parse(stored);
      } catch {
        /* ignore */
      }
    }
    if (!roots.length) roots = await invoke<string[]>("default_roots");
    localStorage.setItem(LS_ROOTS, JSON.stringify(roots));
    await refreshDevices();
    try {
      await invoke("set_output_target", { target });
    } catch {
      /* no mic yet */
    }
    if (micMix) {
      if (!micSource) micSource = preferredSource();
      if (micSource) {
        savePrefs();
        try {
          await invoke("set_input", { source: micSource, gain: micGain });
        } catch {
          /* source gone */
        }
      }
    }
    if (monitorOn) {
      try {
        await invoke("set_monitor", { enabled: true, target: monitorTarget || null });
      } catch {
        /* monitor device gone */
      }
    }
    if (roots.length) await refreshLibrary(false);
    poll = setInterval(pollVoices, 400);
    return () => clearInterval(poll);
  });

  $: allLibSounds = allSounds(folders);
  $: navItems = flatten(folders);
  $: targetNode = nodes.find((n) => n.name === target);
  $: targetOk = !!targetNode?.virtual_mic;
  $: sinkOpts = [
    { value: "", label: "default sink" },
    ...sinks().map((n) => ({ value: n.name, label: labelFor(n) })),
  ];
  $: monitorOpts = [
    { value: "", label: "default output" },
    ...sinks().map((n) => ({ value: n.name, label: n.description || n.name })),
  ];
  $: sourceOpts = [
    { value: "", label: "choose source…" },
    ...sources().map((n) => ({ value: n.name, label: n.description || n.name })),
  ];
  $: searchHits = search.trim()
    ? allSounds(folders).filter((s) => {
        const q = search.trim().toLowerCase();
        return s.name.toLowerCase().includes(q) || s.ext.includes(q);
      })
    : [];
</script>

<main>
  <header class="topbar">
    <div class="brand">
      <img class="logo" src="/icon.png" alt="Kakiri" />
      <div class="titles">
        <strong>Kakiri Soundboard</strong>
        <span class="sub">{mic?.present ? mic.source : "no virtual mic"}</span>
      </div>
      <span class="dot" class:ok={mic?.present}></span>
    </div>

    <div class="views">
      <button class="ghost" class:on={view === "library"} onclick={() => (view = "library")}
        >Library</button
      >
      <button class="ghost" class:on={view === "pads"} onclick={() => (view = "pads")}>Pads</button>
    </div>

    <div class="controls">
      <label class="seg">
        Send to
        <Select
          value={target}
          options={sinkOpts}
          onchange={(v) => {
            target = v;
            applyTarget();
          }}
          title="Where soundboard audio is sent (your virtual mic)"
        />
      </label>

      <button class="ghost" class:on={monitorOn} onclick={toggleMonitor} title="Hear the mix yourself">
        <Headphones size={14} /> Monitor
      </button>
      {#if monitorOn}
        <Select
          value={monitorTarget}
          options={monitorOpts}
          onchange={(v) => {
            monitorTarget = v;
            applyMonitorTarget();
          }}
          title="Monitor device"
          width="165px"
        />
      {/if}

      <button class="ghost" class:on={micMix} onclick={toggleMic} title="Mix your microphone into the virtual mic (talk + play sounds at once)">
        <Mic size={14} /> Mic
      </button>
      {#if micMix}
        <Select
          value={micSource}
          options={sourceOpts}
          onchange={(v) => {
            micSource = v;
            applyMic();
          }}
          title="Voice source"
          width="185px"
        />
        <input
          type="range"
          min="0"
          max="2"
          step="0.01"
          bind:value={micGain}
          onchange={applyMic}
          title="Mic level"
        />
      {/if}

      <label class="seg" title="Volume">
        Vol
        <input type="range" min="0" max="1" step="0.01" bind:value={masterGain} onchange={savePrefs} />
      </label>

      <button class="ghost" class:on={loop} onclick={() => { loop = !loop; savePrefs(); }} title="Loop played sounds" aria-label="Loop">
        <Repeat size={15} />
      </button>
      <Select
        value={retrigger}
        options={retriggerOpts}
        onchange={(v) => {
          retrigger = v;
          savePrefs();
        }}
        title="What happens when you trigger the same sound again: Layer / Choke (restart) / Toggle (play-stop)"
        width="116px"
      />
      <button class="ghost" onclick={tone}>Test tone</button>
      {#if !mic?.present && mic?.can_create}
        <button onclick={createMic} disabled={busy}>Create mic</button>
      {/if}
      <button class="stop" onclick={stopAll} disabled={!voices.length}><Square size={12} fill="currentColor" /> Stop all</button>
    </div>
  </header>

  {#if !mic?.present}
    <div class="banner">
      No virtual mic yet.
      {#if mic?.can_create}
        <button class="link" onclick={createMic}>Create “Soundboard Mic”</button>
      {:else}
        Install VB-CABLE (Windows) or <code>brew install blackhole-2ch</code> (macOS), then pick it as output.
      {/if}
      Then Discord → Voice &amp; Video → Input Device → it, and turn <b>off</b> Noise Suppression / Echo Cancellation / AGC.
      You can still <b>preview</b> sounds locally with <Headphones size={13} />.
    </div>
  {/if}

  {#if mic?.present && !targetOk}
    <div class="banner warn">
      <TriangleAlert size={13} /> Sounds are going to <b>{targetNode ? labelFor(targetNode) : "default sink"}</b>, which is
      <b>not your virtual mic</b> — Discord won't hear them.
      <button class="link" onclick={useSoundboardMic}>Send to Soundboard Mic</button>
    </div>
  {/if}

  {#if view === "library"}
  <div class="layout">
    <aside class="sidebar">
      <div class="side-head">
        <span>Playlists</span>
        <button class="link" onclick={addFolder}><Plus size={13} /> Add folder</button>
      </div>
      {#if !navItems.length}
        <p class="muted pad">No folders yet. Add the folder that holds your sounds.</p>
      {/if}
      {#each navItems as item}
        <div
          class="nav"
          class:active={selected?.path === item.f.path}
          style="padding-left: {10 + item.depth * 14}px"
          onclick={() => (selected = item.f)}
          onkeydown={(e) => e.key === "Enter" && (selected = item.f)}
          role="button"
          tabindex="0"
        >
          <span class="nav-name">{item.depth === 0 ? "📁" : "📂"} {item.f.name}</span>
          <span class="count">{item.f.total_sounds}</span>
          {#if item.depth === 0}
            <button
              class="x"
              title="Remove"
              onclick={(e) => {
                e.stopPropagation();
                removeRoot(item.f.path);
              }}><X size={13} /></button>
          {/if}
        </div>
      {/each}
    </aside>

    <section class="content">
      <div class="content-head">
        <input class="search" placeholder="Search all sounds…" bind:value={search} />
        {#if selected && !search.trim()}
          <span class="muted ellipsis">{selected.path}</span>
        {/if}
      </div>

      {#if search.trim()}
        <h3>{searchHits.length} result{searchHits.length === 1 ? "" : "s"}</h3>
        <div class="grid">
          {#each searchHits as s (s.id)}
            <SoundCard {s} {play} {preview} {fmtDur} />
          {/each}
        </div>
      {:else if selected}
        {#if selected.children.length}
          <h3>Folders</h3>
          <div class="chips">
            {#each selected.children as c (c.path)}
              <button class="chip" onclick={() => (selected = c)}>
                📂 {c.name} <span class="count">{c.total_sounds}</span>
              </button>
            {/each}
          </div>
        {/if}

        <h3>{selected.name} · {selected.sounds.length} sounds</h3>
        {#if !selected.sounds.length}
          <p class="muted pad">No sounds directly in this folder — try a subfolder above.</p>
        {/if}
        <div class="grid">
          {#each selected.sounds as s (s.id)}
            <SoundCard {s} {play} {preview} {fmtDur} />
          {/each}
        </div>
      {:else}
        <p class="muted pad">Add a folder to get started.</p>
      {/if}
    </section>
  </div>
  {:else}
    <Pads sounds={allLibSounds} play={playPath} />
  {/if}

  <footer class="nowbar">
    <span class="muted">Playing {voices.length}</span>
    {#each voices as v (v.id)}
      <span class="now" class:mon={v.route === "monitor"}>
        {#if v.route === "monitor"}<Headphones size={12} />{:else if v.looped}<Repeat size={12} />{:else}<Music size={11} />{/if}
        <span class="lbl">{v.label}</span>
        <input
          class="seek"
          type="range"
          min="0"
          max="1"
          step="0.001"
          title="Seek"
          value={scrub?.id === v.id ? scrub.pos : v.progress}
          onpointerdown={() => (scrub = { id: v.id, pos: v.progress })}
          oninput={(e) => {
            const p = Number(e.currentTarget.value);
            scrub = { id: v.id, pos: p };
            seekVoice(v.id, p);
          }}
          onpointerup={() => (scrub = null)}
          onchange={() => (scrub = null)}
        />
        <input
          class="gain"
          type="range"
          min="0"
          max="2"
          step="0.01"
          title="Volume"
          value={gainEdit[v.id] ?? v.gain}
          oninput={(e) => {
            const g = Number(e.currentTarget.value);
            gainEdit[v.id] = g;
            setVoiceGain(v.id, g);
          }}
        />
        <button class="x" onclick={() => stopVoice(v.id)}><X size={12} /></button>
      </span>
    {/each}
    {#if toast}<span class="toast">{toast}</span>{/if}
  </footer>
</main>

<style>
  :global(:root) {
    --bg: #0e0b16;
    --panel: #171226;
    --panel-2: #1f1833;
    --panel-3: #271f3d;
    --border: #332a4d;
    --text: #ece9f7;
    --text-2: #c9c3de;
    --muted: #9a93b5;
    --accent: #8b5cf6;
    --accent-2: #b79bff;
    --accent-dim: #2e2350;
    --accent-ring: rgba(139, 92, 246, 0.35);
    --ok: #7ddca0;
    --danger: #e0565b;
  }
  :global(svg.lucide) { vertical-align: -0.15em; }
  :global(body) {
    margin: 0;
    height: 100vh;
    overflow: hidden;
    font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
    background: var(--bg);
    color: var(--text);
  }
  main { display: flex; flex-direction: column; height: 100vh; }

  .topbar {
    display: flex; align-items: center; gap: 14px; flex-wrap: wrap;
    padding: 10px 16px; background: var(--panel); border-bottom: 1px solid var(--border);
  }
  .brand { display: flex; align-items: center; gap: 10px; }
  .brand .logo { width: 40px; height: 40px; border-radius: 11px; display: block; }
  .brand .titles { display: flex; flex-direction: column; line-height: 1.15; }
  .brand strong { font-size: 14px; letter-spacing: 0.01em; }
  .brand .sub { color: var(--muted); font-size: 11px; }
  .views { display: flex; gap: 6px; margin-left: 8px; }
  .views button { padding: 5px 10px; font-size: 12px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--danger); }
  .dot.ok { background: var(--ok); }
  .controls { margin-left: auto; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .seg { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); }
  .search {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 10px;
  }
  input[type="range"] {
    width: 82px;
    height: 4px;
    appearance: none;
    -webkit-appearance: none;
    background: var(--border);
    border-radius: 2px;
    outline: none;
  }
  input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    -webkit-appearance: none;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--accent-2);
    cursor: pointer;
  }

  button {
    background: linear-gradient(180deg, var(--accent-2), var(--accent));
    color: #fff; border: 0; border-radius: 9px;
    padding: 7px 12px; font-size: 13px; cursor: pointer;
    transition: filter .15s ease;
  }
  button:hover:not(:disabled) { filter: brightness(1.08); }
  button:disabled { opacity: .45; cursor: default; }
  button.ghost { background: var(--panel-3); color: var(--text-2); }
  button.ghost.on { background: var(--accent); color: #fff; }
  button.stop { background: var(--danger); }
  button.link { background: none; color: var(--accent-2); padding: 0; }
  button.x { background: none; color: var(--muted); padding: 0 4px; }

  .banner {
    background: #2a2417; color: #f5d98b; font-size: 12.5px;
    padding: 8px 16px; border-bottom: 1px solid #3a3320;
  }
  .banner.warn { background: #3a1d1d; color: #f2a6a6; border-bottom-color: #5a2a2a; }
  .banner code { background: var(--bg); padding: 1px 5px; border-radius: 4px; }

  .layout { flex: 1; display: flex; min-height: 0; }
  .sidebar {
    width: 260px; flex: none; overflow-y: auto;
    border-right: 1px solid var(--border); padding: 8px 0;
  }
  .side-head {
    display: flex; justify-content: space-between; align-items: center;
    padding: 4px 12px 10px; font-size: 11px; letter-spacing: .08em;
    text-transform: uppercase; color: var(--muted);
  }
  .nav {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 10px; font-size: 13px; cursor: pointer; border-radius: 6px; margin: 0 6px;
  }
  .nav:hover { background: var(--panel-2); }
  .nav.active { background: var(--accent-dim); }
  :global(.nav-name) { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  :global(.count) { font-size: 11px; color: var(--muted); }

  .content { flex: 1; overflow-y: auto; padding: 14px 18px; min-width: 0; }
  .content-head { display: flex; align-items: center; gap: 12px; margin-bottom: 10px; }
  .search { width: 260px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  h3 { font-size: 12px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); margin: 16px 0 8px; }

  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { background: var(--panel-3); color: var(--text-2); }
  .grid {
    display: grid; gap: 10px;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  }
  .pad { padding: 4px 0; }

  .nowbar {
    display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
    padding: 8px 16px; background: var(--panel); border-top: 1px solid var(--border); min-height: 42px;
  }
  .now {
    display: inline-flex; align-items: center; gap: 6px; font-size: 12px;
    background: var(--panel-3); border-radius: 999px; padding: 3px 6px 3px 10px;
  }
  .now.mon { background: var(--accent-dim); }
  .now .lbl { max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .now input[type="range"] { width: 72px; flex: none; }
  .now input.gain { width: 54px; flex: none; opacity: .85; }
  .toast { margin-left: auto; color: var(--ok); font-size: 12px; }
  .muted { color: var(--muted); font-size: 12px; }
</style>
