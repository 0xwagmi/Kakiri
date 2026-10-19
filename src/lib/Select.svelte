<script lang="ts">
  import { ChevronDown } from "$lib/icons";

  type Option = { value: string; label: string };

  export let value = "";
  export let options: Option[] = [];
  export let placeholder = "…";
  export let title = "";
  export let width = "190px";
  export let onchange: (v: string) => void = () => {};

  let open = false;
  let root: HTMLDivElement;

  function choose(v: string) {
    open = false;
    value = v;
    onchange(v);
  }

  function onDocClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") open = false;
  }

  $: currentLabel = options.find((o) => o.value === value)?.label ?? placeholder;
</script>

<svelte:window onclick={onDocClick} onkeydown={onKey} />

<div class="sel" bind:this={root} style="width:{width}">
  <button type="button" class="ctl" class:open {title} onclick={() => (open = !open)}>
    <span class="lbl">{currentLabel}</span>
    <span class="chev"><ChevronDown size={13} /></span>
  </button>
  {#if open}
    <ul class="menu">
      {#each options as o (o.value)}
        <li>
          <button
            type="button"
            class="item"
            class:active={o.value === value}
            onclick={() => choose(o.value)}>{o.label}</button
          >
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .sel {
    position: relative;
    display: inline-block;
  }
  .ctl {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 10px;
    min-height: 32px;
    font-size: 12px;
    cursor: pointer;
    text-align: left;
  }
  .ctl:hover,
  .ctl.open {
    border-color: var(--accent);
  }
  .lbl {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    color: var(--muted);
    font-size: 10px;
  }
  .menu {
    position: absolute;
    z-index: 50;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    max-height: 320px;
    overflow-y: auto;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.55);
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    color: var(--text-2);
    border: 0;
    border-radius: 6px;
    padding: 6px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .item:hover {
    background: var(--panel-3);
  }
  .item.active {
    background: var(--accent-dim);
    color: #fff;
  }
</style>
