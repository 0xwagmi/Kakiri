<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import Select from "$lib/Select.svelte";
  import { Pencil, Check } from "$lib/icons";

  type Sound = {
    id: string;
    path: string;
    name: string;
    ext: string;
    duration_secs: number | null;
  };
  type Pad = {
    id: number;
    soundPath: string | null;
    name: string;
    hotkey: string;
    canonical: string | null;
    error: boolean;
  };

  export let sounds: Sound[] = [];
  export let play: (path: string, label: string) => void;

  const LS_PADS = "sb.pads";
  const PAD_COUNT = 16;

  let pads: Pad[] = [];
  let editMode = false;
  let toast = "";
  let unlisten: (() => void) | undefined;

  $: padSoundOpts = [
    { value: "", label: "— empty —" },
    ...sounds.map((s) => ({ value: s.path, label: s.name })),
  ];

  function defaultPads(): Pad[] {
    return Array.from({ length: PAD_COUNT }, (_, i) => ({
      id: i,
      soundPath: null,
      name: "",
      hotkey: "",
      canonical: null,
      error: false,
    }));
  }

  function flash(m: string) {
    toast = m;
    setTimeout(() => {
      if (toast === m) toast = "";
    }, 2500);
  }

  function nameFor(path: string | null): string {
    if (!path) return "";
    return sounds.find((s) => s.path === path)?.name ?? path.split("/").pop() ?? "";
  }

  async function syncHotkeys() {
    const res = await invoke<string[]>("set_hotkeys", {
      accelerators: pads.map((p) => p.hotkey || ""),
    });
    pads = pads.map((p, i) => ({
      ...p,
      canonical: res[i] || null,
      error: !!p.hotkey.trim() && !res[i],
    }));
    localStorage.setItem(LS_PADS, JSON.stringify(pads));
    const bad = pads.filter((p) => p.error).length;
    if (bad) flash(`${bad} hotkey${bad > 1 ? "s" : ""} not recognized`);
  }

  function onPick(i: number) {
    pads[i].name = nameFor(pads[i].soundPath);
    pads = [...pads];
    syncHotkeys();
  }

  function clearPad(i: number) {
    pads[i] = { ...pads[i], soundPath: null, name: "" };
    pads = [...pads];
    syncHotkeys();
  }

  function trigger(p: Pad) {
    if (p.soundPath) play(p.soundPath, p.name || nameFor(p.soundPath));
  }

  /** Map a KeyboardEvent.code to a token the Tauri shortcut parser accepts. */
  function keyToken(code: string): string | null {
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^F([1-9]|1[0-2])$/.test(code)) return code;
    if (/^Numpad[0-9]$/.test(code)) return "Num" + code.slice(-1);
    const map: Record<string, string> = {
      Space: "Space",
      Enter: "Enter",
      NumpadEnter: "Enter",
      Tab: "Tab",
      Minus: "-",
      Equal: "=",
      BracketLeft: "[",
      BracketRight: "]",
      Backslash: "\\",
      Semicolon: ";",
      Quote: "'",
      Comma: ",",
      Period: ".",
      Slash: "/",
      Backquote: "`",
      ArrowUp: "Up",
      ArrowDown: "Down",
      ArrowLeft: "Left",
      ArrowRight: "Right",
      Home: "Home",
      End: "End",
      PageUp: "PageUp",
      PageDown: "PageDown",
      Insert: "Insert",
      Delete: "Delete",
    };
    return map[code] ?? null;
  }

  /** Capture the key combo the user presses instead of making them type it. */
  function captureHotkey(e: KeyboardEvent, i: number) {
    e.preventDefault();
    if (e.key === "Escape" || e.key === "Backspace" || e.key === "Delete") {
      pads[i].hotkey = "";
    } else {
      const token = keyToken(e.code);
      if (!token) return; // modifier-only press → keep waiting
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("Ctrl");
      if (e.altKey) mods.push("Alt");
      if (e.shiftKey) mods.push("Shift");
      if (e.metaKey) mods.push("Super");
      pads[i].hotkey = [...mods, token].join("+");
    }
    pads = [...pads];
    syncHotkeys();
  }

  onMount(() => {
    void (async () => {
    const stored = localStorage.getItem(LS_PADS);
    if (stored) {
      try {
        const parsed = JSON.parse(stored);
        if (Array.isArray(parsed) && parsed.length) pads = parsed;
      } catch {
        /* ignore */
      }
    }
    if (!pads.length) pads = defaultPads();
    await syncHotkeys();
    unlisten = await listen<string>("hotkey", (e) => {
      const pad = pads.find((p) => p.canonical && p.canonical === e.payload);
      if (pad) trigger(pad);
    });
    })();
    return () => unlisten?.();
  });
</script>

<div class="pads">
  <div class="head">
    <button class="ghost" class:on={editMode} onclick={() => (editMode = !editMode)}>
      {#if editMode}<Check size={14} /> Done{:else}<Pencil size={14} /> Edit pads{/if}
    </button>
    <span class="muted">
      Click a pad to send it to the call. In edit mode, give it a sound and a global hotkey
      (works even when the game has focus, e.g. <code>Ctrl+Alt+1</code>).
    </span>
    {#if toast}<span class="toast">{toast}</span>{/if}
  </div>

  <div class="grid">
    {#each pads as p, i (p.id)}
      <div class="pad" class:filled={!!p.soundPath} class:err={p.error} class:edit={editMode}>
        {#if editMode}
          <Select
            value={p.soundPath ?? ""}
            options={padSoundOpts}
            placeholder="— empty —"
            width="100%"
            onchange={(v) => {
              pads[i].soundPath = v || null;
              onPick(i);
            }}
          />
          <div
            class="hotkey"
            class:set={!!p.hotkey}
            tabindex="0"
            role="textbox"
            title="Click, then press the key combo (Esc / Backspace clears)"
            onkeydown={(e) => captureHotkey(e, i)}
          >
            {p.hotkey || "click, then press keys"}
          </div>
          <button class="clr" onclick={() => clearPad(i)}>clear</button>
        {:else}
          <button class="hit" onclick={() => trigger(p)} title={p.soundPath ?? "empty pad"}>
            <span class="n">{p.name || "—"}</span>
            {#if p.hotkey}<span class="hk">{p.hotkey}</span>{/if}
          </button>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .pads { flex: 1; overflow-y: auto; padding: 14px 18px; }
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; flex-wrap: wrap; }
  .grid {
    display: grid;
    gap: 10px;
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
  .pad {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    min-height: 84px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
  }
  .pad.filled { border-color: var(--accent); background: var(--accent-dim); }
  .pad.err { border-color: var(--danger); }
  .pad.edit { min-height: 118px; }

  .hit {
    flex: 1;
    background: none;
    border: 0;
    color: var(--text);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
    padding: 0;
    cursor: pointer;
    text-align: left;
  }
  .hit:hover .n { color: var(--accent-2); }
  .n { font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .hk {
    font-size: 10px;
    color: var(--muted);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 1px 6px;
  }

  .hotkey {
    width: 100%;
    box-sizing: border-box;
    min-height: 29px;
    display: flex;
    align-items: center;
    background: var(--bg);
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 5px 8px;
    font-size: 12px;
    cursor: text;
    outline: none;
  }
  .hotkey.set { color: var(--text); border-color: var(--accent); }
  .hotkey:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-ring); }
  .pad.err .hotkey { border-color: var(--danger); }
  .clr { background: none; color: var(--muted); padding: 2px 0; font-size: 11px; }

  button.ghost { background: var(--panel-3); color: var(--text-2); }
  button.ghost.on { background: var(--accent); color: #fff; }
  button {
    border: 0;
    border-radius: 8px;
    padding: 7px 12px;
    font-size: 13px;
    cursor: pointer;
  }
  code { background: var(--bg); padding: 1px 5px; border-radius: 4px; }
  .toast { margin-left: auto; color: var(--ok); font-size: 12px; }
  .muted { color: var(--muted); font-size: 12px; }
</style>
