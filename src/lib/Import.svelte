<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { X, Upload, Link2, Download, Check, TriangleAlert, FileAudio } from "$lib/icons";

  type Imported = { path: string; name: string; duration_secs: number };

  /** Folder new files land in (the library folder being browsed). */
  export let dest = "";
  /** Every folder the user may pick as the destination. */
  export let folders: string[] = [];
  /** Files dropped on the window, handed over by the parent. */
  export let pending: string[] = [];
  export let onConsumed: () => void = () => {};
  export let onDone: (s: Imported) => void = () => {};
  export let onClose: () => void = () => {};

  type Job = {
    id: string;
    label: string;
    pct: number; // negative = unknown
    stage: string;
    state: "running" | "done" | "error";
    error?: string;
  };

  let tab: "file" | "link" = "file";
  let folder = dest;
  let url = "";
  let jobs: Job[] = [];
  let hovering = false;
  let seq = 0;
  let ffmpeg = true;
  let ytdlp: string | null = null;
  let ytdlpStale = false;
  let updating = false;
  let probing = false;
  let probe: { title: string; duration_secs: number } | null = null;
  let probeError = "";
  let clipOn = false;
  let clipFrom = "0:00";
  let clipTo = "";
  let extraFolder = ""; // created on demand when the library is still empty

  const base = (p: string) => p.split(/[\\/]/).pop() ?? p;
  $: destination = folder || dest || folders[0] || extraFolder;
  $: running = jobs.some((j) => j.state === "running");

  function addJob(label: string): Job {
    const job: Job = {
      id: `job${++seq}`,
      label,
      pct: 0,
      stage: "starting",
      state: "running",
    };
    jobs = [...jobs, job];
    return job;
  }

  function settle(id: string, patch: Partial<Job>) {
    jobs = jobs.map((j) => (j.id === id ? { ...j, ...patch } : j));
  }

  async function importFile(path: string) {
    const job = addJob(base(path));
    try {
      const got = await invoke<Imported>("import_file", {
        job: job.id,
        src: path,
        dest: destination,
      });
      settle(job.id, { state: "done", pct: 1, stage: "done" });
      onDone(got);
    } catch (e) {
      settle(job.id, { state: "error", stage: "", error: String(e) });
    }
  }

  async function importUrl() {
    const link = url.trim();
    if (!link) return;
    const from = clipOn ? parseClock(clipFrom) : null;
    const to = clipOn ? parseClock(clipTo) : null;
    if (clipOn && (from == null || to == null || to <= from)) {
      probeError = "The clip range looks wrong — use m:ss, with the end after the start.";
      return;
    }
    const job = addJob(
      clipOn
        ? `${probe?.title ?? link} (${fmtClock(from!)}–${fmtClock(to!)})`.slice(0, 48)
        : (probe?.title ?? link.replace(/^https?:\/\//, "")).slice(0, 48),
    );
    url = "";
    probe = null;
    try {
      const got = await invoke<Imported>("import_url", {
        job: job.id,
        url: link,
        dest: destination,
        start: clipOn ? from : null,
        end: clipOn ? to : null,
      });
      settle(job.id, { state: "done", pct: 1, stage: "done" });
      onDone(got);
    } catch (e) {
      settle(job.id, { state: "error", stage: "", error: String(e) });
    }
  }

  const fmtClock = (t: number) => {
    const m = Math.floor(t / 60);
    const sec = Math.floor(t % 60);
    return `${m}:${String(sec).padStart(2, "0")}`;
  };

  /** "1:05" / "65" / "1:05.5" → seconds */
  function parseClock(text: string): number | null {
    const t = text.trim();
    if (!t) return null;
    const parts = t.split(":");
    if (parts.length > 2) return null;
    const nums = parts.map((p) => Number(p.replace(",", ".")));
    if (nums.some((n) => !Number.isFinite(n) || n < 0)) return null;
    return parts.length === 2 ? nums[0] * 60 + nums[1] : nums[0];
  }

  async function doProbe() {
    const link = url.trim();
    probe = null;
    probeError = "";
    if (!link || !ytdlp) return;
    probing = true;
    try {
      const info = await invoke<{ title: string; duration_secs: number }>("import_probe", { url: link });
      probe = info;
      // Default to a short drop rather than the whole track — that is what a
      // soundboard wants, and "whole file" is one untick away.
      clipOn = info.duration_secs > 30;
      if (info.duration_secs > 0) clipTo = fmtClock(Math.min(15, info.duration_secs));
    } catch (e) {
      probeError = String(e);
    } finally {
      probing = false;
    }
  }

  async function setupDownloader() {
    probeError = "";
    updating = true;
    const job = addJob(ytdlp ? `Downloader update (was ${ytdlp})` : "Downloader (yt-dlp)");
    try {
      const version = await invoke<string>("import_ensure_ytdlp", { job: job.id });
      ytdlp = version;
      ytdlpStale = false;
      settle(job.id, { state: "done", pct: 1, stage: "done" });
      flashLink(`Downloader ${version} ready — paste the link again.`);
    } catch (e) {
      settle(job.id, { state: "error", stage: "", error: String(e) });
    } finally {
      updating = false;
    }
  }

  async function cancelJob(job: Job) {
    try {
      await invoke("import_cancel", { job: job.id });
    } catch {
      /* already gone */
    }
  }

  let linkNote = "";
  function flashLink(msg: string) {
    linkNote = msg;
    setTimeout(() => {
      if (linkNote === msg) linkNote = "";
    }, 4000);
  }

  async function pickFiles() {
    const picked = await open({
      multiple: true,
      directory: false,
      title: "Add audio or video files",
      filters: [
        {
          name: "Audio and video",
          extensions: [
            "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "aiff",
            "mp4", "m4v", "mov", "webm", "mkv", "avi",
          ],
        },
      ],
    });
    if (!picked) return;
    for (const p of Array.isArray(picked) ? picked : [picked]) await importFile(p);
  }

  function onDrop(ev: DragEvent) {
    ev.preventDefault();
    hovering = false;
    const files = Array.from(ev.dataTransfer?.files ?? []);
    for (const f of files) {
      // Tauri gives real paths on the File object; plain browsers don't.
      const path = (f as File & { path?: string }).path;
      if (path) void importFile(path);
    }
  }

  onMount(() => {
    let un: Promise<() => void> | undefined;
    try {
      un = listen<{ job: string; pct: number; stage: string }>("import-progress", (e) => {
        const { job, pct, stage } = e.payload;
        jobs = jobs.map((j) => (j.id === job && j.state === "running" ? { ...j, pct, stage } : j));
      });
    } catch {
      /* not inside Tauri (plain browser preview) */
    }
    invoke<{ ffmpeg: boolean; ytdlp: string | null; ytdlp_stale: boolean }>("import_tools")
      .then((t) => {
        ffmpeg = t.ffmpeg;
        ytdlp = t.ytdlp;
        ytdlpStale = t.ytdlp_stale;
      })
      .catch(() => {});
    if (!dest && !folders.length) {
      invoke<string>("import_default_folder")
        .then((p) => {
          extraFolder = p;
          folder = p;
        })
        .catch(() => {});
    }
    return () => {
      void un?.then((f) => f());
    };
  });

  $: if (pending.length) {
    const list = [...pending];
    onConsumed();
    for (const p of list) void importFile(p);
  }
</script>

<div class="scrim" role="presentation" onclick={(e) => e.target === e.currentTarget && onClose()}>
  <div class="card">
    <header>
      <h2>Add sounds</h2>
      <button class="icon" onclick={onClose} title="Close"><X size={15} /></button>
    </header>

    <div class="tabs">
      <button class="tab" class:on={tab === "file"} onclick={() => (tab = "file")}>
        <FileAudio size={13} /> Files
      </button>
      <button class="tab" class:on={tab === "link"} onclick={() => (tab = "link")}>
        <Link2 size={13} /> Link
      </button>
    </div>

    {#if tab === "file"}
      <div
        class="drop"
        class:hovering={hovering}
        role="button"
        tabindex="0"
        onclick={pickFiles}
        onkeydown={(e) => (e.key === "Enter" || e.key === " ") && pickFiles()}
        ondragover={(e) => {
          e.preventDefault();
          hovering = true;
        }}
        ondragleave={() => (hovering = false)}
        ondrop={onDrop}
      >
        <Upload size={20} />
        <b>Drop files here</b>
        <span>or click to choose — audio is copied, video has its audio pulled out</span>
      </div>
      {#if !ffmpeg}
        <p class="warn">
          <TriangleAlert size={13} />
          ffmpeg not found, so video files can't be converted. Install it
          (<code>sudo apt install ffmpeg</code> / <code>brew install ffmpeg</code> /
          <code>winget install ffmpeg</code>) — audio files work either way.
        </p>
      {/if}
    {:else}
      <div class="row">
        <input
          class="url"
          placeholder="Paste a sound or video link (mp3, mp4, YouTube, TikTok…)"
          bind:value={url}
          onkeydown={(e) => e.key === "Enter" && (probe ? importUrl() : doProbe())}
          oninput={() => {
            probe = null;
            probeError = "";
          }}
        />
        <button class="primary" onclick={importUrl} disabled={!url.trim()}>
          <Download size={13} /> Get
        </button>
      </div>

      {#if probing}
        <p class="hint">Looking the link up…</p>
      {:else if probe}
        <div class="found">
          <div class="ftitle" title={probe.title}>{probe.title}</div>
          <div class="fsub">{probe.duration_secs > 0 ? fmtClock(probe.duration_secs) : "unknown length"}</div>
        </div>
        {#if probe.duration_secs > 30}
          <label class="clip">
            <input type="checkbox" bind:checked={clipOn} />
            <span>Only take a clip</span>
            <input class="t" bind:value={clipFrom} placeholder="0:00" disabled={!clipOn} />
            <span class="dash">→</span>
            <input class="t" bind:value={clipTo} placeholder="0:10" disabled={!clipOn} />
          </label>
          {#if clipOn && !ffmpeg}
            <p class="warn"><TriangleAlert size={13} /> Cutting a clip needs ffmpeg.</p>
          {/if}
        {/if}
      {:else if url.trim() && ytdlp}
        <button class="peek" onclick={doProbe}>Look it up first</button>
      {/if}

      {#if probeError}
        <p class="warn"><TriangleAlert size={13} /> {probeError}</p>
      {/if}

      {#if linkNote}
        <p class="hint">{linkNote}</p>
      {/if}

      {#if !ytdlp}
        <div class="need-tool">
          <span>
            <b>Page links need the downloader.</b>
            One-time 17–39 MB download (yt-dlp) — turns YouTube/TikTok links into audio.
          </span>
          <button class="primary small" onclick={setupDownloader} disabled={updating}>Set up</button>
        </div>
      {:else if ytdlpStale}
        <div class="need-tool warnrow">
          <span>
            <b>Downloader {ytdlp} is out of date.</b>
            Sites change constantly — YouTube and TikTok links usually fail on an old copy.
          </span>
          <button class="primary small" onclick={setupDownloader} disabled={updating}>Update</button>
        </div>
      {:else}
        <p class="hint">
          Downloader {ytdlp}. Direct audio/video file links work without it.
          <button class="linkish" onclick={setupDownloader} disabled={updating}>update</button>
        </p>
      {/if}
    {/if}

    <label class="dest">
      <span>Save to</span>
      <select bind:value={folder}>
        {#if dest}<option value={dest}>{dest}</option>{/if}
        {#if extraFolder && extraFolder !== dest}
          <option value={extraFolder}>{extraFolder}</option>
        {/if}
        {#each folders.filter((f) => f !== dest) as f (f)}
          <option value={f}>{f}</option>
        {/each}
      </select>
    </label>

    {#if jobs.length}
      <ul class="jobs">
        {#each jobs as j (j.id)}
          <li class:err={j.state === "error"} class:ok={j.state === "done"}>
            <div class="jtop">
              <span class="jname" title={j.label}>{j.label}</span>
              {#if j.state === "done"}
                <Check size={13} />
              {:else if j.state === "error"}
                <TriangleAlert size={13} />
              {:else}
                <span class="pct">
                  {j.pct < 0 || j.pct === 0 ? j.stage : `${Math.round(j.pct * 100)}%`}
                </span>
              {/if}
            </div>
            {#if j.state === "running"}
              <div class="bar"><i style="width:{j.pct < 0 ? 100 : j.pct * 100}%;opacity:{j.pct < 0 ? 0.35 : 1}"></i></div>
              <button class="stop" onclick={() => cancelJob(j)}>Cancel</button>
            {/if}
            {#if j.error}
              <p class="jerr">{j.error}</p>
              {#if /out of date|refused the download/i.test(j.error) && ytdlp}
                <button class="primary small fix" onclick={setupDownloader} disabled={updating}>
                  Update downloader
                </button>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    <footer>
      <span class="muted">{running ? "Working…" : ""}</span>
      <button class="ghost" onclick={onClose}>{running ? "Hide" : "Done"}</button>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(6, 4, 12, 0.6);
    backdrop-filter: blur(2px);
    display: grid;
    place-items: center;
    z-index: 50;
  }
  .card {
    width: 520px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    overflow: auto;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.45);
    padding: 14px 16px 12px;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  h2 { margin: 0; font-size: 14px; font-weight: 600; letter-spacing: 0.01em; }
  .icon {
    border: 0;
    background: none;
    color: var(--muted);
    cursor: pointer;
    padding: 3px;
    display: grid;
    place-items: center;
    border-radius: 5px;
  }
  .icon:hover { color: var(--text); background: var(--panel-2); }

  .tabs { display: flex; gap: 4px; margin-bottom: 10px; }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 12px;
    padding: 5px 9px;
    border-radius: 6px;
    cursor: pointer;
  }
  .tab:hover { color: var(--text); background: var(--panel-2); }
  .tab.on { color: var(--text); background: color-mix(in srgb, var(--accent) 22%, var(--panel-2)); }

  .drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 24px 12px;
    border-radius: 9px;
    border: 1px dashed var(--border);
    background: var(--panel-2);
    color: var(--muted);
    cursor: pointer;
    text-align: center;
    transition: border-color 0.12s, background 0.12s, color 0.12s;
  }
  .drop b { color: var(--text); font-size: 13px; font-weight: 550; }
  .drop span { font-size: 11px; }
  .drop:hover, .drop.hovering {
    border-color: var(--accent);
    color: var(--text-2);
    background: color-mix(in srgb, var(--accent) 10%, var(--panel-2));
  }

  .row { display: flex; gap: 6px; }
  .url {
    flex: 1;
    min-width: 0;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 12px;
    padding: 7px 9px;
  }
  .url:focus { outline: none; border-color: var(--accent); }
  .primary {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 0;
    border-radius: 7px;
    padding: 0 12px;
    font-size: 12px;
    color: #fff;
    background: var(--accent);
    cursor: pointer;
  }
  .primary:disabled { opacity: 0.45; cursor: default; }
  .hint { color: var(--muted); font-size: 11px; margin: 7px 2px 0; }

  .found {
    margin-top: 8px;
    padding: 7px 9px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--accent) 10%, var(--panel-2));
  }
  .ftitle { font-size: 12px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .fsub { font-size: 10.5px; color: var(--muted); margin-top: 2px; }
  .clip { display: flex; align-items: center; gap: 6px; margin-top: 8px; font-size: 11px; color: var(--text-2); }
  .clip input[type="checkbox"] { accent-color: var(--accent); }
  .clip .t {
    width: 52px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 5px;
    color: var(--text);
    font-size: 11px;
    padding: 3px 6px;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  .clip .t:disabled { opacity: 0.4; }
  .dash { color: var(--muted); }
  .peek {
    margin-top: 7px;
    border: 0;
    background: none;
    color: var(--accent-2, var(--accent));
    font-size: 11px;
    cursor: pointer;
    padding: 0;
  }
  .peek:hover { text-decoration: underline; }
  .need-tool {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
    padding: 9px 10px;
    border-radius: 8px;
    background: var(--panel-2);
    font-size: 11px;
    color: var(--muted);
    line-height: 1.45;
  }
  .need-tool b { color: var(--text-2); font-weight: 550; }
  .primary.small { padding: 5px 12px; flex: none; }
  .warn {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--warn, #d8a13a);
    font-size: 11px;
    margin: 8px 2px 0;
    line-height: 1.45;
  }
  .warn code { font-size: 10.5px; background: var(--panel-3); padding: 1px 4px; border-radius: 4px; }

  .dest {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    font-size: 11px;
    color: var(--muted);
  }
  .dest select {
    flex: 1;
    min-width: 0;
    background: var(--panel-2);
    color: var(--text-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 5px 6px;
    font-size: 11px;
  }

  .jobs { list-style: none; margin: 12px 0 0; padding: 0; display: flex; flex-direction: column; gap: 7px; }
  .jobs li { background: var(--panel-2); border-radius: 7px; padding: 7px 9px; }
  .jobs li.ok { background: color-mix(in srgb, var(--ok, #3fb950) 12%, var(--panel-2)); }
  .jobs li.err { background: color-mix(in srgb, var(--danger, #f85149) 12%, var(--panel-2)); }
  .jtop { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .jname { font-size: 11.5px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pct { font-size: 10.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .bar { height: 3px; border-radius: 2px; background: var(--panel-3); margin-top: 6px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); transition: width 0.15s linear; }
  .jerr { margin: 5px 0 0; font-size: 10.5px; color: var(--danger, #f85149); line-height: 1.4; }
  .stop {
    margin-top: 5px;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 10.5px;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
  }
  .stop:hover { color: var(--danger, #f85149); }
  .linkish {
    border: 0;
    background: none;
    color: var(--accent-2, var(--accent));
    font-size: 11px;
    cursor: pointer;
    padding: 0 0 0 2px;
  }
  .linkish:hover { text-decoration: underline; }
  .fix { margin-top: 7px; padding: 4px 10px; }
  .warnrow { background: color-mix(in srgb, var(--warn, #d8a13a) 14%, var(--panel-2)); }
  .warnrow b { color: var(--text); }
  .ok .jname { color: var(--text); }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 12px;
  }
  .muted { color: var(--muted); font-size: 11px; }
  .ghost {
    border: 0;
    background: none;
    color: var(--text-2);
    font-size: 12px;
    padding: 5px 10px;
    border-radius: 6px;
    cursor: pointer;
  }
  .ghost:hover { background: var(--panel-2); color: var(--text); }
</style>
