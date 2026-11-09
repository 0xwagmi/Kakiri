<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    Search,
    Play,
    Pause,
    Square,
    SkipBack,
    SkipForward,
    X,
    TriangleAlert,
    Heart,
    Radio,
    RefreshCw,
    Music,
    ListMusic,
    Download,
    Check,
    Trash2,
    History,
    Shuffle,
    Repeat,
  } from "$lib/icons";

  type Track = {
    id: string;
    title: string;
    artist: string;
    artwork: string | null;
    duration: number; // 0 = unknown
    source: string;
    /** local file, for saved tracks */
    path?: string;
    /** saved before metadata was stored beside it: artist/title came from a name */
    legacy?: boolean;
  };

  type Collection = {
    kind: "trending" | "loved" | "recent" | "saved" | "tag" | "search";
    tag?: string;
  };

  /** let the page open Settings from here, so the key is findable */
  export let onSettings: () => void = () => {};
  /** tell the app a library file appeared or went away, so it can rescan */
  export let onChanged: () => void = () => {};

  const LS_PREFS = "sb.prefs";
  const LS_RECENT = "sb.recent";
  /** decoration only: covers for saves made before covers were written beside the file */
  const LS_COVERS = "sb.covers";

  /** Outside the Tauri shell (a browser tab) there is no IPC, and every
      `invoke` would throw. Say so plainly instead of failing at the first call. */
  const inTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in (window as unknown as object);

  const STATIONS = ["synthwave", "lofi", "phonk", "city pop", "dnb", "hyperpop"];

  let lfKey = "";
  let lfUser = "";

  let collection: Collection = { kind: "trending" };
  let rows: Track[] = [];
  let loading = true;
  let error = "";
  let query = "";
  let searchActive = false;

  let recent: Track[] = [];
  /** tracks in the library's Online folder — the disk is the source of truth */
  let saved: Track[] = [];
  let savingKey: string | null = null;
  /** rows ticked in the Saved list */
  let selected: string[] = [];
  /** pending deletion, waiting for confirmation */
  let pendingDelete: Track[] | null = null;
  let notice = "";
  let covers: Record<string, string> = {};

  // queue
  let queue: Track[] = [];
  let qIndex = -1;
  let voiceId: number | null = null;
  let starting = false;
  /** bumped by stop()/newer plays, so a slow resolve that lands late is thrown away */
  let seq = 0;
  /** a track clicked while one is still resolving: only the last one survives */
  let requested: Track | null = null;
  let shuffle = false;
  let repeat = false;
  let paused = false;
  let progress = 0; // 0..1 of the current track
  /** how much of the current track is on disk, so Save can show it filling */
  let cachedBytes = 0;
  let totalBytes = 0;
  let currentCached = false;
  let poll: ReturnType<typeof setInterval> | null = null;

  let current: Track | null = null;
  $: current = qIndex >= 0 ? (queue[qIndex] ?? null) : null;
  $: hero = rows[0] ?? null;
  $: cards = rows.slice(1, 7);

  $: title =
    collection.kind === "trending"
      ? "Trending now"
      : collection.kind === "loved"
        ? "Loved on Last.fm"
        : collection.kind === "recent"
          ? "Recently played"
          : collection.kind === "saved"
            ? "Saved"
          : collection.kind === "tag"
            ? `Station · ${collection.tag}`
            : `Results for “${query.trim()}”`;

  $: elapsed = Math.round(progress * (current?.duration ?? 0));
  $: compact = collection.kind === "saved";
  $: tableRows = compact ? rows : rows.slice(7);
  $: tableBase = compact ? 0 : 7;
  $: allSelected = compact && rows.length > 0 && selected.length === rows.length;
  $: cachePct = currentCached ? 100 : totalBytes ? Math.min(100, (cachedBytes / totalBytes) * 100) : 0;
  const mb = (b: number) => (b >= 1024 * 1024 ? `${(b / 1048576).toFixed(1)} MB` : `${Math.round(b / 1024)} KB`);
  /** Compare loosely: the file name has characters replaced on save, so exact
      string equality would stop recognising a track as already saved. */
  const loose = (v: string) => v.toLowerCase().replace(/[^a-z0-9]+/g, "");
  $: savedKeys = new Set(saved.map((t) => loose(`${t.artist}${t.title}`)));
  const isSaved = (t: Track) => savedKeys.has(loose(`${t.artist}${t.title}`));

  $: subtitle =
    collection.kind === "trending"
      ? "Global charts"
      : collection.kind === "loved"
        ? lfUser
          ? `@${lfUser}`
          : "Last.fm"
        : collection.kind === "saved"
          ? "Saved to your library"
          : collection.kind === "recent"
          ? "From this machine"
          : collection.kind === "tag"
            ? "Last.fm tag station"
            : "Deezer search";

  /** A cover that fails to load (blocked by CSP, offline, expired) disappears
      rather than leaving a broken square. */
  function hideBroken(e: Event) {
    const el = e.currentTarget as HTMLImageElement | null;
    if (el) el.style.display = "none";
  }

  const fmt = (s: number) => {
    if (!s) return "—";
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };

  function readPrefs() {
    try {
      const c = JSON.parse(localStorage.getItem(LS_COVERS) ?? "{}");
      if (c && typeof c === "object") covers = c;
    } catch {
      /* ignore */
    }
    try {
      const p = JSON.parse(localStorage.getItem(LS_PREFS) ?? "{}");
      lfUser = p.lastfmUser ?? "";
    } catch {
      /* ignore */
    }
    try {
      const r = JSON.parse(localStorage.getItem(LS_RECENT) ?? "[]");
      if (Array.isArray(r)) {
        recent = r.slice(0, 40).map((t: Track, i: number) => ({
          id: `recent:${i}`,
          title: t.title,
          artist: t.artist,
          artwork: t.artwork ?? null,
          duration: t.duration ?? 0,
          source: "online", // old entries may point at files that no longer exist
        }));
      }
    } catch {
      /* ignore */
    }
  }

  function remember(t: Track) {
    const key = `${t.artist} — ${t.title}`;
    // Remember what the track *is*, not where a copy happened to live: a saved
    // file can be deleted later, and history should still be playable.
    const entry: Track = {
      id: `recent:${key}`,
      title: t.title,
      artist: t.artist,
      artwork: t.artwork,
      duration: t.duration,
      source: "online",
    };
    recent = [entry, ...recent.filter((r) => `${r.artist} — ${r.title}` !== key)].slice(0, 40);
    try {
      localStorage.setItem(LS_RECENT, JSON.stringify(recent));
    } catch {
      /* storage full */
    }
  }

  function failure(e: unknown): string {
    const msg = String(e);
    if (msg.includes("invoke") || msg.includes("__TAURI")) {
      return "This page is open in a browser — the player only runs inside the Kakiri app.";
    }
    return msg.replace(/^Error:\s*/, "");
  }

  async function open(c: Collection) {
    collection = c;
    searchActive = false;
    error = "";
    if (c.kind === "recent") {
      rows = recent;
      loading = false;
      return;
    }
    if (c.kind === "saved") {
      await refreshSaved();
      rows = saved;
      loading = false;
      return;
    }
    if (c.kind === "loved" && (!lfKey || !lfUser)) {
      rows = [];
      loading = false;
      error = "Add your Last.fm API key and username in Settings first.";
      return;
    }
    if (c.kind === "tag" && !lfKey) {
      rows = [];
      loading = false;
      error = "Add a Last.fm API key in Settings to play tag stations.";
      return;
    }
    if (!inTauri) {
      rows = [];
      loading = false;
      return;
    }
    loading = true;
    try {
      rows =
        c.kind === "trending"
          ? await invoke<Track[]>("online_charts", { limit: 40 })
          : c.kind === "loved"
            ? await invoke<Track[]>("online_loved", { user: lfUser, apiKey: lfKey })
            : await invoke<Track[]>("online_tag", { tag: c.tag, apiKey: lfKey });
      if (!rows.length) error = "Nothing came back for that.";
    } catch (e) {
      error = failure(e);
      rows = [];
    } finally {
      loading = false;
    }
  }

  async function runSearch() {
    const q = query.trim();
    if (!q) return closeSearch();
    if (!inTauri) return;
    loading = true;
    error = "";
    searchActive = true;
    try {
      rows = await invoke<Track[]>("online_search", { query: q, limit: 40 });
      if (!rows.length) error = "Nothing matched that search.";
    } catch (e) {
      error = failure(e);
      rows = [];
    } finally {
      loading = false;
    }
  }

  async function closeSearch() {
    query = "";
    searchActive = false;
    await open(collection);
  }

  async function play(index: number, fromQueue?: Track[]) {
    const list = fromQueue ?? rows;
    const track = list[index];
    if (!track) return;
    queue = [...list];
    qIndex = index;
    await startTrack(track);
  }

  async function playAll(shuffled = false) {
    if (!rows.length) return;
    const list = [...rows];
    if (shuffled) {
      for (let i = list.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [list[i], list[j]] = [list[j], list[i]];
      }
    }
    await play(0, list);
  }

  /** Stop whatever Online is playing, without touching the queue. */
  async function stopVoice() {
    if (voiceId === null) return;
    const id = voiceId;
    voiceId = null;
    try {
      await invoke("stop_voice", { id });
    } catch {
      /* already gone */
    }
  }

  /** Play `t`. The track is passed in rather than read from `current`: a derived
      value lags one flush behind the assignment that triggered it, and reading it
      here played the *previously* selected track instead of the clicked one.

      Online is a player, not a soundboard: clicking five rows must leave exactly
      one voice. Clicks that arrive while a track is still resolving are folded
      into a single "latest wins" request, so they cost one resolve and no extra
      bandwidth — and a resolve that finishes after a newer request is discarded
      instead of starting a second stream. */
  async function startTrack(t: Track) {
    if (starting) {
      requested = t;
      return;
    }
    const mine = ++seq;
    starting = true;
    paused = false;
    progress = 0;
    currentCached = false;
    cachedBytes = 0;
    totalBytes = 0;
    try {
      await stopVoice();
      if (mine !== seq) return;
      remember(t);
      // a saved track is a file on this machine: no network at all — unless the
      // file has been deleted since, in which case fall back to streaming
      let v: { id?: number } = {};
      let local = t.source === "saved" && !!t.path;
      if (local) {
        try {
          const r = await invoke<{ id?: number } | number>("play_sound", {
            path: t.path,
            route: "both",
            retrigger: "choke",
          });
          v = typeof r === "number" ? { id: r } : r;
        } catch {
          local = false; // the file is gone: forget the path and stream it
        }
      }
      if (!local) {
        v = await invoke<{ id: number }>("play_online", {
          artist: t.artist,
          title: t.title,
          duration: t.duration,
        });
      }
      if (mine !== seq) {
        // a newer click or a stop happened while this one was resolving
        if (typeof v.id === "number") {
          try {
            await invoke("stop_voice", { id: v.id });
          } catch {
            /* already gone */
          }
        }
        return;
      }
      if (typeof v.id === "number") voiceId = v.id;
      error = "";
    } catch (e) {
      if (mine === seq) {
        error = failure(e);
        voiceId = null;
      }
    } finally {
      starting = false;
      const queued = requested;
      requested = null;
      if (queued && mine === seq) await startTrack(queued);
    }
  }

  async function next() {
    if (qIndex + 1 >= queue.length) {
      if (repeat && queue.length) {
        qIndex = 0;
        const first = queue[0];
        if (first) return void startTrack(first);
      }
      return stop();
    }
    qIndex += 1;
    const nextTrack = queue[qIndex];
    if (!nextTrack) return;
    await startTrack(nextTrack);
  }

  async function prev() {
    if (qIndex <= 0) return;
    qIndex -= 1;
    const prevTrack = queue[qIndex];
    if (!prevTrack) return;
    await startTrack(prevTrack);
  }

  /** Pause keeps the position: the voice stays in the engine, silent. */
  async function togglePause() {
    if (voiceId === null) return;
    paused = !paused;
    try {
      await invoke("pause_voice", { id: voiceId, paused });
    } catch {
      paused = !paused; // put the UI back if the engine refused
    }
  }

  /** Click anywhere on the progress line to jump there. */
  async function seekAt(ev: MouseEvent) {
    if (voiceId === null || !current) return;
    const el = ev.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (ev.clientX - rect.left) / rect.width));
    progress = f;
    try {
      await invoke("seek_voice", { id: voiceId, position: f });
    } catch {
      /* engine busy; the poll will resync */
    }
  }

  async function stop() {
    paused = false;
    seq += 1; // any resolve still in flight is stale now
    requested = null;
    await stopVoice();
    queue = [];
    qIndex = -1;
  }

  /** Download a row into the library, so it is instant and offline from now on. */
  async function save(t: Track) {
    if (savingKey || isSaved(t)) return;
    const key = `${t.artist}\u0000${t.title}`;
    savingKey = key;
    error = "";
    try {
      const got = await invoke<{ path: string; duration_secs: number }>("save_online", {
        artist: t.artist,
        title: t.title,
        duration: t.duration,
        job: `save-${Date.now()}`,
      });
      const entry: Track = {
        id: `saved:${got.path}`,
        title: t.title,
        artist: t.artist,
        artwork: t.artwork,
        duration: Math.round(got.duration_secs) || t.duration,
        source: "saved",
        path: got.path,
      };
      saved = [entry, ...saved.filter((s) => `${s.artist}\u0000${s.title}` !== key)];
      await refreshSaved();
      notice = `Saved to your library · ${t.artist} — ${t.title}`;
      onChanged();
      setTimeout(() => (notice = ""), 3000);
    } catch (e) {
      error = failure(e);
    } finally {
      savingKey = null;
    }
  }

  const trackKey = (t: Track) => `${t.artist}\u0000${t.title}`;

  /** Re-read the Online folder after anything that might have changed it. */
  async function refreshSaved() {
    if (!inTauri) return;
    try {
      const list = await invoke<
        {
          path: string;
          artist: string;
          title: string;
          duration?: number;
          artwork?: string | null;
        }[]
      >("online_saved");
      saved = list.map((t, i) => ({
        id: `saved:${t.path || i}`,
        title: t.title,
        artist: t.artist,
        // the file's own sidecar first; the local map is only a fallback for
        // saves made before covers were written next to the audio
        artwork: t.artwork ?? covers[`${loose(t.artist)}|${loose(t.title)}`] ?? null,
        duration: t.duration ?? 0,
        source: "saved",
        path: t.path,
      }));
      await fillMissingCovers();
    } catch {
      /* keep what we have */
    }
  }

  /** Saves made before covers were stored beside the file get one lookup each
      (cached locally), so the Saved list never shows a wall of blank tiles. */
  async function fillMissingCovers() {
    if (!inTauri) return;
    let looked = 0;
    let changed = false;
    const next = [...saved];
    for (let i = 0; i < next.length && looked < 12; i++) {
      const t = next[i];
      if (t.artwork && !t.legacy) continue;
      looked += 1;
      try {
        const hit = await invoke<{
          artist: string;
          title: string;
          artwork: string | null;
        } | null>("online_lookup", { artist: t.artist, title: t.title });
        if (!hit) continue;
        // for pre-sidecar files this also fixes a title/artist swap
        next[i] = {
          ...t,
          artist: t.legacy ? hit.artist : t.artist,
          title: t.legacy ? hit.title : t.title,
          artwork: t.artwork ?? hit.artwork,
          legacy: false,
        };
        if (hit.artwork) covers[`${loose(hit.artist)}|${loose(hit.title)}`] = hit.artwork;
        changed = true;
      } catch {
        /* offline: leave the glyph */
      }
    }
    if (changed) {
      saved = next;
      if (collection.kind === "saved") rows = saved;
      try {
        localStorage.setItem(LS_COVERS, JSON.stringify(covers));
      } catch {
        /* ignore */
      }
    }
  }

  function toggleSelect(t: Track) {
    const k = trackKey(t);
    selected = selected.includes(k) ? selected.filter((x) => x !== k) : [...selected, k];
  }

  function toggleSelectAll() {
    selected = allSelected ? [] : rows.map(trackKey);
  }

  /** Ask before deleting: this removes real files. */
  function askDelete(list: Track[]) {
    if (!list.length) return;
    pendingDelete = list;
  }

  async function confirmDelete() {
    const list = pendingDelete ?? [];
    pendingDelete = null;
    selected = [];
    let removed = 0;
    for (const t of list) {
      if (await removeSaved(t)) removed += 1;
    }
    await refreshSaved();
    if (collection.kind === "saved") rows = saved;
    if (removed) {
      notice = removed === 1 ? "Removed from your library" : `Removed ${removed} tracks`;
      onChanged(); // let the Library view rescan
      setTimeout(() => (notice = ""), 2600);
    }
  }

  async function removeSaved(t: Track): Promise<boolean> {
    const key = trackKey(t);
    const entry = saved.find((s) => trackKey(s) === key);
    const path = t.path ?? entry?.path;
    if (!path) {
      error = "Could not tell which file to delete — rescan the Saved list.";
      return false;
    }
    if (path) {
      try {
        await invoke("unsave_online", { path });
      } catch (e) {
        error = failure(e);
        return false;
      }
    }
    // it was playing from that file: stop rather than leave a dangling voice
    if (current?.id === t.id) await stop();
    selected = selected.filter((x) => x !== key);
    return true;
  }

  /** A track is finished when its voice disappears from the engine. */
  function watch() {
    poll = setInterval(async () => {
      if (voiceId === null || starting) return;
      try {
        const voices = await invoke<{ id: number; progress: number }[]>("active_voices");
        const mine = voices.find((v) => v.id === voiceId);
        if (!mine) {
          voiceId = null;
          progress = 0;
          await next();
        } else if (!paused) {
          progress = mine.progress ?? 0;
        }
        if (current) {
          try {
            const st = await invoke<{
              cached: number;
              total: number;
              complete: boolean;
            }>("online_cache_state", { artist: current.artist, title: current.title });
            cachedBytes = st.cached;
            totalBytes = st.total;
            currentCached = st.complete;
          } catch {
            /* keep the previous state */
          }
        }
      } catch {
        /* engine busy; try again next tick */
      }
    }, 700);
  }

  onMount(() => {
    readPrefs();
    // credentials come from the backend's private store
    invoke<string | null>("secret_get", { name: "lastfm.key" })
      .then((k) => {
        lfKey = k ?? "";
        // a loved/tag view opened before the key arrived needs a retry
        if (lfKey && (collection.kind === "loved" || collection.kind === "tag")) {
          void open(collection);
        }
      })
      .catch(() => {});
    void refreshSaved();
    void open(collection);
    watch();
  });

  onDestroy(() => {
    if (poll) clearInterval(poll);
  });
</script>

<div class="online">
  <!-- ---------------- playlists ---------------- -->
  <aside class="rail">
    <div class="rail-group">
      <span class="rail-label">Playlists</span>
      <button
        class="rail-item"
        class:on={!searchActive && collection.kind === "trending"}
        onclick={() => void open({ kind: "trending" })}
      >
        <RefreshCw size={13} /> Trending now
      </button>
      <button
        class="rail-item"
        class:on={!searchActive && collection.kind === "loved"}
        onclick={() => void open({ kind: "loved" })}
      >
        <Heart size={13} /> Loved
      </button>
      <button
        class="rail-item"
        class:on={!searchActive && collection.kind === "saved"}
        onclick={() => void open({ kind: "saved" })}
      >
        <Download size={13} /> Saved
        {#if saved.length}<span class="count">{saved.length}</span>{/if}
      </button>
      <button
        class="rail-item"
        class:on={!searchActive && collection.kind === "recent"}
        onclick={() => void open({ kind: "recent" })}
      >
        <History size={13} /> Recently played
        {#if recent.length}<span class="count">{recent.length}</span>{/if}
      </button>
    </div>

    <div class="rail-group">
      <span class="rail-label">Stations</span>
      {#each STATIONS as tag}
        <button
          class="rail-item"
          class:on={!searchActive && collection.kind === "tag" && collection.tag === tag}
          onclick={() => void open({ kind: "tag", tag })}
        >
          <Radio size={13} /> {tag}
        </button>
      {/each}
    </div>

    {#if !lfKey}
      <p class="rail-hint">
        Charts work as they are.
        <button class="link" onclick={onSettings}>Add a Last.fm key</button> for loved tracks and
        stations.
      </p>
    {/if}
  </aside>

  <!-- ---------------- tracks ---------------- -->
  <main class="pane">
    <div class="bar">
      <div class="field" class:disabled={!inTauri}>
        <Search size={15} />
        <input
          placeholder="Search songs, artists, anything…"
          bind:value={query}
          onkeydown={(e) => e.key === "Enter" && runSearch()}
        />
        {#if searchActive}
          <button class="icon" onclick={() => void closeSearch()} title="Back">
            <X size={13} />
          </button>
        {/if}
      </div>
    </div>

      {#if notice}
        <div class="notice ok"><Check size={13} /><p>{notice}</p></div>
      {/if}

    {#if !inTauri}
      <div class="notice">
        <TriangleAlert size={14} />
        <div>
          <strong>Open this in the Kakiri window, not a browser.</strong>
          <p>The player needs the app's audio engine — a browser tab has none.</p>
        </div>
      </div>
    {/if}

    {#if error}
      <div class="notice soft"><TriangleAlert size={14} /><p>{error}</p></div>
    {/if}

    {#if loading}
      <div class="hero sk"></div>
      <div class="cards">
        {#each Array(6) as _}<div class="card sk"></div>{/each}
      </div>
    {:else if rows.length}
      <div class="head">
        <div class="head-text">
          <h2>{title}</h2>
          <span class="sub">{subtitle} · {rows.length} tracks</span>
        </div>
        <div class="head-actions">
          <button class="ghost" onclick={() => void playAll(false)}>
            <Play size={13} /> Play all
          </button>
          <button class="ghost" class:on={shuffle} onclick={() => { shuffle = !shuffle; void playAll(true); }}>
            <Shuffle size={13} /> Shuffle
          </button>
        </div>
      </div>

      {#if compact && selected.length}
        <div class="sel-bar">
          <span class="sel-count">{selected.length} selected</span>
          <button class="danger" onclick={() => askDelete(rows.filter((t) => selected.includes(trackKey(t))))}>
            <Trash2 size={12} /> Delete
          </button>
          <button class="ghost" onclick={() => (selected = [])}>Cancel</button>
        </div>
      {/if}

      {#if hero && !compact}
        <div class="hero-row">
        <button class="hero" onclick={() => void play(0)}>
          <span class="hero-art">
            {#if hero.artwork}<img src={hero.artwork} alt="" onerror={hideBroken} />{:else}<Music size={30} />{/if}
          </span>
          <span class="hero-meta">
            <span class="hero-kicker">Start here</span>
            <span class="hero-title" title={hero.title}>{hero.title}</span>
            <span class="hero-artist" title={hero.artist}>{hero.artist}</span>
          </span>
          <span class="hero-play"><Play size={18} /></span>
        </button>
        {#if !isSaved(hero) && hero.source !== "saved"}
          <button
            class="hero-save"
            class:ready={current?.id === hero.id && currentCached}
            onclick={() => void save(hero)}
            title={current?.id === hero.id && currentCached
              ? "Already cached — saving is instant"
              : "Save to your library"}
          >
            {#if current?.id === hero.id}
              <span class="hero-fill" style="height:{cachePct}%"></span>
            {/if}
            <Download size={12} /> Save
          </button>
        {/if}
        </div>
      {/if}

      {#if cards.length && !compact}
        <div class="cards">
          {#each cards as t, i (t.id)}
            <button class="card" class:playing={current?.id === t.id} onclick={() => void play(i + 1)}>
              <span class="card-art">
                {#if t.artwork}<img src={t.artwork} alt="" loading="lazy" onerror={hideBroken} />{:else}<Music size={18} />{/if}
                <span class="card-play"><Play size={15} /></span>
              </span>
              <span class="card-title" title={t.title}>{t.title}</span>
              <span class="card-artist" title={t.artist}>{t.artist}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if tableRows.length}
        <div class="table">
          <div class="thead">
            <span class="c-num">
              {#if compact}
                <button class="tick" class:on={allSelected} onclick={toggleSelectAll} title="Select all">
                  {#if allSelected}<Check size={10} />{/if}
                </button>
              {:else}
                #
              {/if}
            </span>
            <span class="c-ttl">Title</span>
            <span class="c-dur">Length</span>
            <span class="c-act"></span>
          </div>
          {#each tableRows as t, i (t.id)}
            <div class="trow" class:playing={current?.id === t.id} class:picked={compact && selected.includes(trackKey(t))}>
              <span class="c-num">
                {#if compact}
                  <button
                    class="tick"
                    class:on={selected.includes(trackKey(t))}
                    onclick={() => toggleSelect(t)}
                    title={selected.includes(trackKey(t)) ? "Unselect" : "Select"}
                  >
                    {#if selected.includes(trackKey(t))}<Check size={10} />{/if}
                  </button>
                {:else if current?.id === t.id}
                  <span class="eq"><i></i><i></i><i></i></span>
                {:else}
                  {i + 8}
                {/if}
              </span>
              <span class="c-ttl">
                <span class="thumb">
                  {#if t.artwork}<img src={t.artwork} alt="" loading="lazy" onerror={hideBroken} />{:else}<Music size={13} />{/if}
                </span>
                <span class="ttl-wrap">
                  <span class="ttl" title={t.title}>{t.title}</span>
                  <span class="artist" title={t.artist}>{t.artist}</span>
                </span>
              </span>
              <span class="c-dur">{fmt(t.duration)}</span>
              <span class="c-act">
                {#if isSaved(t)}
                  <button
                    class="row-save saved"
                    onclick={() => askDelete([t])}
                    title="Remove from your library"
                  >
                    <Trash2 size={12} />
                  </button>
                {:else}
                  <button
                    class="row-save"
                    class:saved={isSaved(t)}
                    onclick={() => void save(t)}
                    disabled={savingKey === `${t.artist}\u0000${t.title}`}
                    title={isSaved(t)
                      ? "Saved to your library"
                      : current?.id === t.id && currentCached
                        ? "Already cached — saving is instant"
                        : "Save to your library"}
                  >
                    {#if isSaved(t)}
                      <Check size={12} />
                    {:else if savingKey === `${t.artist}\u0000${t.title}`}
                      <span class="dot-busy"></span>
                    {:else}
                      <Download size={12} />
                    {/if}
                  </button>
                {/if}
                <button
                  class="row-play"
                  onclick={() => (current?.id === t.id ? void togglePause() : void play(i + tableBase))}
                  title={current?.id === t.id ? (paused ? "Resume" : "Pause") : "Play"}
                >
                  <Play size={13} />
                </button>
              </span>
            </div>
          {/each}
        </div>
      {/if}
    {:else if !error}
      <div class="empty">
        <ListMusic size={22} />
        <p>{collection.kind === "recent" ? "Nothing played yet — it fills up as you listen." : "Nothing to show yet."}</p>
      </div>
    {/if}
  </main>

  {#if pendingDelete}
    <div class="scrim">
      <div class="dialog">
        <h3>
          {pendingDelete.length === 1
            ? "Remove this track?"
            : `Remove ${pendingDelete.length} tracks?`}
        </h3>
        <p class="dlg-body">
          {#if pendingDelete.length === 1}
            <strong>{pendingDelete[0].artist} — {pendingDelete[0].title}</strong>
            will be deleted from <code>Music/Kakiri/Online</code>.
          {:else}
            These {pendingDelete.length} files will be deleted from
            <code>Music/Kakiri/Online</code>:
          {/if}
        </p>
        {#if pendingDelete.length > 1}
          <ul class="dlg-list">
            {#each pendingDelete.slice(0, 8) as t (t.id)}
              <li>{t.artist} — {t.title}</li>
            {/each}
            {#if pendingDelete.length > 8}<li>…and {pendingDelete.length - 8} more</li>{/if}
          </ul>
        {/if}
        <div class="dlg-actions">
          <button class="ghost" onclick={() => (pendingDelete = null)}>Cancel</button>
          <button class="danger" onclick={() => void confirmDelete()}>
            <Trash2 size={12} /> Delete
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if current}
    <div class="now" class:live={!!voiceId && !starting} class:paused>
      <div class="now-track">
        <div class="now-art">
          {#if current.artwork}
            <img src={current.artwork} alt="" onerror={hideBroken} />
          {:else}
            <Music size={16} />
          {/if}
          {#if current.artwork}
            <span class="now-glow" style="background-image:url({current.artwork})"></span>
          {/if}
        </div>
        <div class="now-meta">
          <div class="now-title" title={current.title}>{current.title}</div>
          <div class="now-artist" title={current.artist}>{current.artist}</div>
        </div>
      </div>

      <div class="now-center">
        <div class="now-controls">
          <button class="icon" onclick={() => void prev()} disabled={qIndex <= 0} title="Previous">
            <SkipBack size={14} />
          </button>
          <button
            class="play-btn"
            onclick={() => void togglePause()}
            title={paused ? "Resume" : "Pause"}
            aria-label={paused ? "Resume" : "Pause"}
          >
            {#if starting}
              <span class="spin-ring"></span>
            {:else if paused}
              <Play size={15} />
            {:else}
              <Pause size={15} />
            {/if}
          </button>
          <button class="icon" onclick={() => void next()} title="Next">
            <SkipForward size={14} />
          </button>
          {#if starting}<span class="state">buffering…</span>{/if}
        </div>
        <div class="track-line">
          <span class="time">{fmt(elapsed)}</span>
          <span
            class="seek"
            role="slider"
            tabindex="-1"
            aria-valuenow={Math.round(progress * 100)}
            aria-label="Seek"
            onclick={seekAt}
            onkeydown={(e) => e.key === "Enter" && seekAt(e as unknown as MouseEvent)}
          >
            <span class="seek-fill" style="width:{Math.max(0, Math.min(100, progress * 100))}%"></span>
            <span class="seek-knob" style="left:{Math.max(0, Math.min(100, progress * 100))}%"></span>
          </span>
          <span class="time">{fmt(current.duration)}</span>
        </div>
      </div>

      <div class="now-side">
        {#if !isSaved(current) && current.source !== "saved"}
          <button
            class="now-save"
            class:ready={currentCached}
            onclick={() => void save(current)}
            disabled={savingKey !== null}
            title={currentCached
              ? "Cached — saving is a local copy, no download"
              : "Save to your library (downloading as it plays)"}
          >
            <span class="now-save-fill" style="height:{cachePct}%"></span>
            <span class="now-save-label">
              {#if savingKey !== null}
                <span class="dot-busy"></span> Saving…
              {:else if currentCached}
                <Download size={10} /> Save
                <span class="now-save-size">{mb(totalBytes)}</span>
              {:else}
                <Download size={10} /> Save
                <span class="now-save-size">{mb(cachedBytes)} / {mb(totalBytes)}</span>
              {/if}
            </span>
          </button>
        {:else}
          <button class="now-saved" onclick={() => askDelete([current])} title="Remove from your library">
            <Check size={11} /> Saved
          </button>
        {/if}
        <button class="icon" class:on={shuffle} onclick={() => (shuffle = !shuffle)} title="Shuffle">
          <Shuffle size={13} />
        </button>
        <button class="icon" class:on={repeat} onclick={() => (repeat = !repeat)} title="Repeat">
          <Repeat size={13} />
        </button>
        <button class="icon" onclick={() => void stop()} title="Stop">
          <Square size={12} />
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .online {
    overflow: hidden;
    display: grid;
    grid-template-columns: 204px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    gap: 18px;
    height: 100%;
    padding: 16px 20px 14px;
    max-width: 1120px;
    margin: 0 auto;
    width: 100%;
  }

  /* ---------------- rail ---------------- */
  .rail {
    scrollbar-width: thin;
    scrollbar-color: rgb(255 255 255 / 0.12) transparent;
    grid-row: 1;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding-right: 14px;
    border-right: 1px solid var(--line, rgb(255 255 255 / 0.06));
    overflow-y: auto;
  }
  .rail-group { display: flex; flex-direction: column; gap: 1px; }
  .rail-label {
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-3);
    padding: 0 8px 6px;
  }
  .rail-item {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--text-2);
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
  }
  .rail-item:hover { background: var(--panel-2); color: var(--text); }
  .rail-item.on {
    background: var(--panel-2);
    color: var(--text);
    box-shadow: inset 2px 0 0 var(--accent, #8b7bff);
  }
  .count {
    margin-left: auto;
    font-size: 10.5px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .rail-hint {
    margin: auto 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-3);
    padding: 0 8px;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent, #8b7bff);
    cursor: pointer;
    text-decoration: underline;
  }

  /* ---------------- pane ---------------- */
  .pane {
    scrollbar-width: thin;
    scrollbar-color: rgb(255 255 255 / 0.14) transparent;
    grid-row: 1;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
    overflow-y: auto;
    padding-right: 4px;
  }
  .bar {
    position: sticky;
    top: -16px;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px 0 8px;
    margin: -16px 0 0;
    background: linear-gradient(var(--bg, #0e0b16) 72%, rgb(0 0 0 / 0));
  }
  .field {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    gap: 9px;
    height: 36px;
    padding: 0 11px;
    border-radius: 9px;
    background: var(--panel-2);
    color: var(--text-3);
  }
  .field:focus-within { box-shadow: inset 0 0 0 1px var(--accent-dim); }
  .field.disabled { opacity: 0.6; }
  .field input {
    flex: 1 1 auto;
    min-width: 0;
    border: 0;
    background: none;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
  .field input::placeholder { color: var(--text-3); }

  .notice {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--panel-2);
    font-size: 12.5px;
    color: var(--text-2);
  }
  .notice.soft { background: none; padding: 0 2px; color: var(--warn, #e0a33e); }
  .notice strong { display: block; color: var(--text); font-size: 13px; font-weight: 600; }
  .notice p { margin: 3px 0 0; color: var(--text-3); }

  .head { display: flex; align-items: flex-end; gap: 10px; }
  .head-text { min-width: 0; flex: 1 1 auto; }
  .head h2 {
    margin: 0;
    font-size: 19px;
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub { font-size: 11px; color: var(--text-3); }
  .head-actions { display: flex; gap: 6px; flex: 0 0 auto; }

  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border: 0;
    border-radius: 8px;
    background: var(--panel-2);
    color: var(--text-2);
    font-size: 12px;
    cursor: pointer;
  }
  .ghost:hover { background: var(--panel-3); color: var(--text); }
  .ghost.on { color: var(--text); box-shadow: inset 0 0 0 1px var(--accent-line, rgb(255 255 255 / 0.08)); }

  /* hero */
  .hero-row { display: flex; align-items: stretch; gap: 8px; }
  .hero {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 15px;
    padding: 13px;
    border: 0;
    border-radius: 13px;
    background: linear-gradient(115deg, var(--panel-2), var(--panel-1, var(--panel-2)) 72%);
    text-align: left;
    cursor: pointer;
    color: inherit;
  }
  .hero:hover .hero-play { opacity: 1; transform: scale(1); }
  .hero-art {
    width: 84px;
    height: 84px;
    flex: 0 0 auto;
    border-radius: 9px;
    overflow: hidden;
    background: var(--panel-3);
    display: grid;
    place-items: center;
    color: var(--text-3);
  }
  .hero-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .hero-meta { min-width: 0; flex: 1 1 auto; display: flex; flex-direction: column; gap: 1px; }
  .hero-kicker {
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-3);
    margin-bottom: 4px;
  }
  .hero-title {
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.015em;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hero-artist { font-size: 12.5px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hero-play {
    width: 42px;
    height: 42px;
    flex: 0 0 auto;
    border-radius: 50%;
    background: var(--accent, #8b7bff);
    color: #0e0e12;
    display: grid;
    place-items: center;
    opacity: 0.75;
    transform: scale(0.94);
    transition: opacity 140ms ease, transform 140ms ease;
  }

  /* cards */
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(126px, 1fr));
    gap: 10px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px;
    border: 0;
    border-radius: 10px;
    background: none;
    text-align: left;
    cursor: pointer;
    color: inherit;
  }
  .card:hover, .card.playing { background: var(--panel-2); }
  .card-art {
    position: relative;
    aspect-ratio: 1;
    border-radius: 7px;
    overflow: hidden;
    background: var(--panel-3);
    display: grid;
    place-items: center;
    color: var(--text-3);
    margin-bottom: 5px;
  }
  .card-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .card-play {
    position: absolute;
    right: 7px;
    bottom: 7px;
    width: 29px;
    height: 29px;
    border-radius: 50%;
    background: var(--accent, #8b7bff);
    color: #0e0e12;
    display: grid;
    place-items: center;
    opacity: 0;
    transform: translateY(4px);
    transition: opacity 130ms ease, transform 130ms ease;
  }
  .card:hover .card-play { opacity: 1; transform: translateY(0); }
  .card-title { font-size: 12px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-artist { font-size: 10.5px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* table */
  .table { display: flex; flex-direction: column; }
  .thead, .trow {
    display: grid;
    grid-template-columns: 26px 1fr 52px 34px;
    align-items: center;
    gap: 12px;
  }
  .thead {
    padding: 0 9px 6px;
    font-size: 10px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--text-3);
    border-bottom: 1px solid var(--line, rgb(255 255 255 / 0.06));
  }
  .trow { padding: 5px 9px; border-radius: 8px; }
  .trow:hover, .trow.playing { background: var(--panel-2); }
  .trow.playing .ttl { color: var(--accent, #8b7bff); font-weight: 550; }
  .trow.playing .thumb { box-shadow: 0 0 0 1.5px var(--accent, #8b7bff); }
  .c-num { font-size: 11px; color: var(--text-3); font-variant-numeric: tabular-nums; text-align: right; }
  .c-ttl { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .c-dur { font-size: 11px; color: var(--text-3); font-variant-numeric: tabular-nums; text-align: right; }
  .c-act { display: flex; justify-content: flex-end; }
  .thumb {
    width: 32px;
    height: 32px;
    flex: 0 0 auto;
    border-radius: 5px;
    overflow: hidden;
    background: var(--panel-3);
    display: grid;
    place-items: center;
    color: var(--text-3);
  }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .ttl-wrap { display: flex; flex-direction: column; min-width: 0; }
  .ttl { font-size: 12px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .artist { font-size: 10.5px; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-save {
    border: 0;
    background: none;
    color: var(--text-3);
    width: 25px;
    height: 25px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    cursor: pointer;
    opacity: 0;
  }
  .trow:hover .row-save, .row-save.saved { opacity: 1; }
  .row-save:hover { color: var(--text); box-shadow: inset 0 0 0 1px var(--line, rgb(255 255 255 / 0.1)); }
  .row-save.saved { color: var(--accent, #8b7bff); }
  .row-save:disabled { opacity: 0.6; cursor: default; }
  .dot-busy {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--text-3);
    border-top-color: transparent;
    animation: spin 700ms linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .hero-save {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: 62px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: var(--panel-3);
    color: var(--text-2);
    font-size: 11px;
    cursor: pointer;
  }
  .hero-save:hover { color: var(--text); }
  .hero-save.ready { color: var(--text); box-shadow: inset 0 0 0 1px var(--accent-line, rgb(255 255 255 / 0.1)); }
  .hero-fill {
    position: absolute;
    inset: auto 0 0 0;
    background: linear-gradient(180deg, rgb(139 123 255 / 0.34), rgb(139 123 255 / 0.18));
    transition: height 900ms cubic-bezier(0.3, 0.7, 0.4, 1);
    pointer-events: none;
  }

  .row-play {
    border: 0;
    background: none;
    color: var(--text-2);
    width: 25px;
    height: 25px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    cursor: pointer;
    opacity: 0;
  }
  .trow:hover .row-play { opacity: 1; }
  .row-play:hover { color: var(--text); box-shadow: inset 0 0 0 1px var(--line, rgb(255 255 255 / 0.1)); }

  .eq { display: inline-flex; align-items: flex-end; gap: 2px; height: 10px; }
  .eq i { width: 2px; background: var(--accent, #8b7bff); border-radius: 1px; animation: bounce 900ms ease-in-out infinite; }
  .eq i:nth-child(1) { height: 5px; }
  .eq i:nth-child(2) { height: 9px; animation-delay: 180ms; }
  .eq i:nth-child(3) { height: 6px; animation-delay: 340ms; }
  @keyframes bounce {
    0%, 100% { transform: scaleY(0.45); }
    50% { transform: scaleY(1); }
  }

  .empty { display: grid; place-items: center; gap: 8px; padding: 50px 0; color: var(--text-3); font-size: 12px; }
  .empty p { margin: 0; }

  .sk {
    background: linear-gradient(90deg, var(--panel-2), var(--panel-3), var(--panel-2));
    background-size: 200% 100%;
    animation: shimmer 1.5s linear infinite;
    border-radius: 11px;
    min-height: 110px;
  }
  @keyframes shimmer {
    from { background-position: 0 0; }
    to { background-position: -200% 0; }
  }

  /* ---------------- now playing ---------------- */
  .now {
    grid-column: 1 / -1;
    grid-row: 2;
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 10px;
    padding: 9px 14px;
    border-radius: 13px;
    background: linear-gradient(180deg, var(--panel-2), var(--panel-1, var(--panel-2)));
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.32), inset 0 0 0 1px var(--line, rgb(255 255 255 / 0.07));
    animation: bar-in 240ms cubic-bezier(0.2, 0.7, 0.3, 1);
  }
  @keyframes bar-in {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: none; }
  }

  .now-track {
    display: flex;
    align-items: center;
    gap: 11px;
    min-width: 0;
    flex: 0 1 300px;
  }
  .now-art {
    position: relative;
    width: 46px;
    height: 46px;
    flex: 0 0 auto;
    border-radius: 8px;
    overflow: hidden;
    background: var(--panel-3);
    display: grid;
    place-items: center;
    color: var(--text-3);
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.35);
  }
  .now-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  /* the cover's own colours, blurred, as a hint of light behind the tile */
  .now-glow {
    position: absolute;
    inset: 0;
    background-size: cover;
    filter: blur(10px) saturate(1.4);
    opacity: 0;
    transition: opacity 500ms ease;
    transform: scale(1.25);
  }
  .now.live .now-glow { opacity: 0.55; }

  .now-meta { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .now-title {
    font-size: 12.5px;
    font-weight: 550;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .now-artist {
    font-size: 11px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .now-center {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .now-controls { display: flex; align-items: center; gap: 6px; }

  .play-btn {
    width: 34px;
    height: 34px;
    border: 0;
    border-radius: 50%;
    background: var(--text);
    color: var(--panel-1, #14141a);
    display: grid;
    place-items: center;
    cursor: pointer;
    transition: transform 120ms ease, background 120ms ease;
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.4);
  }
  .play-btn:hover { transform: scale(1.06); }
  .play-btn:active { transform: scale(0.96); }

  .spin-ring {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid rgb(0 0 0 / 0.25);
    border-top-color: currentColor;
    animation: spin 700ms linear infinite;
  }

  .state { font-size: 10.5px; color: var(--text-3); margin-left: 2px; }

  .track-line {
    display: flex;
    align-items: center;
    gap: 8px;
    width: min(420px, 100%);
  }
  .time {
    font-size: 10px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    min-width: 26px;
    text-align: center;
  }
  .seek {
    position: relative;
    flex: 1 1 auto;
    height: 12px; /* thin line, generous hit area */
    display: flex;
    align-items: center;
    cursor: pointer;
  }
  .seek::before {
    content: "";
    position: absolute;
    inset: 50% 0 auto;
    height: 3px;
    transform: translateY(-50%);
    border-radius: 999px;
    background: var(--panel-3);
  }
  .seek:hover::before { background: rgb(255 255 255 / 0.16); }
  .seek-fill {
    position: relative;
    height: 3px;
    border-radius: 999px;
    background: var(--accent, #8b7bff);
    box-shadow: 0 0 10px rgb(139 123 255 / 0.5);
    transition: width 700ms linear;
  }
  .seek-knob {
    position: absolute;
    top: 50%;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #fff;
    transform: translate(-50%, -50%) scale(0);
    transition: transform 120ms ease;
    box-shadow: 0 1px 6px rgb(0 0 0 / 0.5);
  }
  .seek:hover .seek-knob { transform: translate(-50%, -50%) scale(1); }
  .now.paused .seek-fill { background: var(--text-3); }

  .now-side { display: flex; align-items: center; gap: 6px; flex: 0 0 auto; }

  .now-save {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    height: 30px;
    padding: 0 11px;
    border: 0;
    border-radius: 7px;
    background: var(--panel-3);
    color: var(--text-2);
    font-size: 11px;
    cursor: pointer;
    transition: color 150ms ease;
  }
  .now-save:hover { color: var(--text); }
  .now-save.ready { color: var(--text); box-shadow: inset 0 0 0 1px var(--accent-line, rgb(255 255 255 / 0.1)); }
  .now-save:disabled { cursor: default; }
  .now-save-fill {
    position: absolute;
    inset: auto 0 0 0;
    background: linear-gradient(180deg, rgb(139 123 255 / 0.42), rgb(139 123 255 / 0.26));
    transition: height 900ms cubic-bezier(0.3, 0.7, 0.4, 1);
    pointer-events: none;
  }
  .now-save.ready .now-save-fill {
    background: linear-gradient(180deg, rgb(139 123 255 / 0.55), rgb(139 123 255 / 0.32));
  }
  .now-save-size {
    font-size: 10px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }

  .now-save-label {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .now-saved {
    border: 0;
    background: none;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--accent, #8b7bff);
    padding: 4px 6px;
    border-radius: 6px;
  }
  .now-saved:hover { background: var(--panel-3); color: var(--text); }

  /* ---- saved list: selection, confirmation ---- */
  .tick {
    width: 15px;
    height: 15px;
    padding: 0;
    border: 1px solid var(--line, rgb(255 255 255 / 0.18));
    border-radius: 4px;
    background: none;
    color: #0e0e12;
    display: grid;
    place-items: center;
    cursor: pointer;
    margin: 0 auto;
  }
  .tick:hover { border-color: var(--text-3); }
  .tick.on { background: var(--accent, #8b7bff); border-color: var(--accent, #8b7bff); }
  .trow.picked { background: var(--panel-2); }

  .sel-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: 9px;
    background: var(--panel-2);
    box-shadow: inset 0 0 0 1px var(--accent-line, rgb(255 255 255 / 0.08));
  }
  .sel-count { font-size: 12px; color: var(--text); margin-right: auto; }

  .danger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    border: 0;
    border-radius: 7px;
    background: #a83a3a;
    color: #fff;
    font-size: 12px;
    cursor: pointer;
  }
  .danger:hover { background: #bd4444; }

  .notice.ok { color: var(--text-2); padding: 8px 2px; }
  .notice.ok p { margin: 0; color: var(--text-2); }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: rgb(0 0 0 / 0.55);
    display: grid;
    place-items: center;
    animation: fade-in 140ms ease;
  }
  @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }

  .dialog {
    width: min(430px, calc(100vw - 40px));
    padding: 18px 20px 16px;
    border-radius: 14px;
    background: var(--panel-1, #14141a);
    box-shadow: 0 24px 60px rgb(0 0 0 / 0.55), inset 0 0 0 1px var(--line, rgb(255 255 255 / 0.08));
    animation: pop-in 160ms cubic-bezier(0.2, 0.8, 0.3, 1);
  }
  @keyframes pop-in {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: none; }
  }
  .dialog h3 { margin: 0 0 8px; font-size: 15px; font-weight: 600; color: var(--text); }
  .dlg-body { margin: 0; font-size: 12.5px; line-height: 1.5; color: var(--text-2); }
  .dlg-body code {
    background: var(--panel-2);
    padding: 1px 5px;
    border-radius: 4px;
    font-size: 11.5px;
  }
  .dlg-list {
    margin: 10px 0 0;
    padding: 0 0 0 16px;
    max-height: 150px;
    overflow-y: auto;
    font-size: 12px;
    color: var(--text-3);
  }
  .dlg-list li { margin: 2px 0; }
  .dlg-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }

  .icon {
    border: 0;
    background: none;
    color: var(--text-2);
    width: 27px;
    height: 27px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    cursor: pointer;
  }
  .icon:hover:not(:disabled) { background: var(--panel-3); color: var(--text); }
  .icon:disabled { opacity: 0.32; cursor: default; }
  .icon.on { color: var(--accent, #8b7bff); }

</style>
