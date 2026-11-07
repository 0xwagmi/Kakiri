<script lang="ts">
  /**
   * The companion: she dances while the soundboard is playing, and is not there
   * at all when it is quiet.
   *
   * Nothing to derive, nothing to time: `playing` comes from the voice list the
   * page already polls, so the backend needs no new events. Mounting the <img>
   * only while playing also means every track starts the dance from frame one,
   * instead of resuming wherever the loop happened to be.
   */
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  import dance from "./assets/companion/dance.webp";

  export let playing = false;

  /** a 200 ms blip would otherwise flash her in and out */
  const MIN_VISIBLE_MS = 700;

  let visible = false;
  let visibleSince = 0;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;

  $: if (playing) {
    clearTimeout(hideTimer);
    if (!visible) {
      visible = true;
      visibleSince = Date.now();
    }
  } else if (visible) {
    const left = MIN_VISIBLE_MS - (Date.now() - visibleSince);
    if (left <= 0) {
      visible = false;
    } else {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => (visible = false), left);
    }
  }

  onMount(() => {
    // 1.5 MB of frames: fetch it while nothing is playing so the first sound
    // brings her up immediately instead of a second late.
    new Image().src = dance;
    return () => clearTimeout(hideTimer);
  });
</script>

{#if visible}
  <img
    class="companion"
    src={dance}
    alt=""
    draggable="false"
    transition:fade={{ duration: 180 }}
  />
{/if}

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
