<script lang="ts">
  /**
   * The companion: idles in the corner, dances while the soundboard is playing.
   *
   * Both states come from the voice list the page already polls, so the backend
   * needs no new events. Each state is one animated WebP with alpha baked in
   * (scripts/companion-assets.sh); swapping `src` restarts the loop, which is all
   * the state machine needs.
   */
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  import idle from "./assets/companion/idle.webp";
  import dance from "./assets/companion/dance.webp";

  export let playing = false;

  /** a 200 ms blip should not flip her into the dance and straight back */
  const MIN_DANCE_MS = 700;

  let dancing = false;
  let danceSince = 0;
  let settle: ReturnType<typeof setTimeout> | undefined;

  $: if (playing) {
    clearTimeout(settle);
    if (!dancing) {
      dancing = true;
      danceSince = Date.now();
    }
  } else if (dancing) {
    const left = MIN_DANCE_MS - (Date.now() - danceSince);
    if (left <= 0) {
      dancing = false;
    } else {
      clearTimeout(settle);
      settle = setTimeout(() => (dancing = false), left);
    }
  }

  $: src = dancing ? dance : idle;

  onMount(() => {
    // both loops up front: fetching 2 MB mid-gesture would show an empty frame
    for (const s of [idle, dance]) new Image().src = s;
    return () => clearTimeout(settle);
  });
</script>

{#key src}
  <img
    class="companion"
    {src}
    alt=""
    draggable="false"
    in:fade={{ duration: 130 }}
    out:fade={{ duration: 130 }}
  />
{/key}

<style>
  .companion {
    position: fixed;
    right: 16px;
    bottom: 12px;
    z-index: 20;
    height: clamp(150px, 26vh, 240px);
    width: auto;
    pointer-events: none;
    /* the artwork carries its own alpha, so there is no card behind her */
    filter: drop-shadow(0 8px 18px rgba(0, 0, 0, 0.45));
    -webkit-user-select: none;
    user-select: none;
  }
  /* narrow windows: she would sit on top of the cards */
  @media (max-width: 720px) {
    .companion {
      height: 130px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .companion {
      transition: none;
    }
  }
</style>
