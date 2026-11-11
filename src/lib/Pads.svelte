<script lang="ts">
  import { onMount } from "svelte";
    import Select from "$lib/Select.svelte";
  import { readPads, writePads, registerHotkeys, defaultPads, PAD_COUNT, type Pad } from "$lib/pads";
  import { Pencil, Check } from "$lib/icons";

  type Sound = {
    id: string;
    path: string;
    name: string;
    ext: string;
    duration_secs: number | null;
  };

  export let sounds: Sound[] = [];
  export let play: (path: string, label: string) => void;


  let pads: Pad[] = [];
  let editMode = false;
  let toast = "";

  $: padSoundOpts = [
    { value: "", label: "— empty —" },
    ...sounds.map((s) => ({ value: s.path, label: s.name })),
  ];


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
    pads = await registerHotkeys(pads);
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
    pads = readPads();
    void writePads(pads);
    void syncHotkeys();
  });
</script>

<div class="pads">
  <div class="head">
    <button class="ghost" class:on={editMode} onclick={() => (editMode = !editMode)}>
      {#if editMode}<Check size={14} /> Done{:else}<Pencil size={14} /> Edit pads{/if}
    </button>
      <span class="hint">Click to play. Edit to assign a sound and a hotkey.</span>
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
          <button class="clr" onclick={() => clearPad(i)} title="Empty this pad">Clear</button>
        {:else}
          <button class="hit" onclick={() => trigger(p)} title={p.soundPath ?? "empty pad"}>
              {#if p.soundPath}
                <span class="n">{p.name}</span>
                <span class="foot">
                  {#if p.hotkey}<span class="hk">{p.hotkey}</span>{/if}
                </span>
              {:else}
                <span class="vacant">empty</span>
              {/if}
          </button>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .pads {
    flex: 1;
    overflow-y: auto;
    padding: 18px 20px 22px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
    flex-wrap: wrap;
  }
  .hint {
    color: var(--muted);
    font-size: 11.5px;
  }
  .toast { margin-left: auto; color: var(--ok); font-size: 11.5px; }

  /* one soft surface per pad: no border noise, hover lifts it a hair */
  .grid {
    display: grid;
    gap: 12px;
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
  .pad {
    position: relative;
    min-height: 86px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: 14px;
    background: var(--panel-2);
    transition: background 140ms ease, transform 140ms ease;
  }
  .pad::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    pointer-events: none;
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.04);
  }
  .pad:hover { background: var(--panel-3); }
  .pad:has(.hit:active) { transform: translateY(1px); }

  .pad.filled {
    background:
      linear-gradient(180deg, rgb(139 123 255 / 0.1), rgb(139 123 255 / 0.03)),
      var(--panel-2);
  }
  .pad.filled::after {
    box-shadow:
      inset 0 0 0 1px rgb(139 123 255 / 0.26),
      inset 0 1px 0 rgb(255 255 255 / 0.05);
  }
  .pad.err::after { box-shadow: inset 0 0 0 1px var(--danger); }
  .pad.edit { min-height: 122px; }

  .hit {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 8px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .n {
    font-size: 12.5px;
    font-weight: 500;
    letter-spacing: -0.01em;
    line-height: 1.32;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .hit:hover .n { color: var(--accent-2, #a48bff); }

  .vacant {
    font-size: 11px;
    color: var(--muted);
    opacity: 0.5;
    letter-spacing: 0.03em;
  }
  .foot { display: flex; min-height: 17px; }
  .hk {
    font-size: 10px;
    color: var(--text-2, var(--muted));
    background: rgb(255 255 255 / 0.05);
    border-radius: 5px;
    padding: 2px 6px;
    font-variant-numeric: tabular-nums;
  }

  /* edit mode: same surface, the fields get their own quiet edge */
  .hotkey {
    width: 100%;
    box-sizing: border-box;
    min-height: 30px;
    display: flex;
    align-items: center;
    padding: 5px 9px;
    border-radius: 8px;
    border: 1px dashed var(--border);
    background: transparent;
    color: var(--muted);
    font-size: 11.5px;
    cursor: text;
    outline: none;
    transition: border-color 120ms ease, color 120ms ease;
  }
  .hotkey:hover { color: var(--text-2, var(--muted)); }
  .hotkey.set {
    color: var(--text);
    border-style: solid;
    border-color: rgb(139 123 255 / 0.45);
  }
  .hotkey:focus {
    border-style: solid;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-ring);
  }
  .pad.err .hotkey { border-color: var(--danger); }

  .clr {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 11px;
    cursor: pointer;
  }
  .clr:hover { color: var(--text); }

  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: transparent;
    color: var(--text-2);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    transition: border-color 120ms ease, color 120ms ease, background 120ms ease;
  }
  .ghost:hover { color: var(--text); border-color: var(--accent); }
  .ghost.on {
    background: rgb(139 123 255 / 0.14);
    border-color: rgb(139 123 255 / 0.4);
    color: var(--accent-2, #a48bff);
  }
</style>
