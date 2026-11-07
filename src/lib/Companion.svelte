<script lang="ts">
  /**
   * The companion: a corner resident who reacts to the soundboard.
   *
   * States are derived, not commanded — she only needs to know whether anything
   * is playing, which the page already knows. That means no new backend events:
   *
   *   idle ──play──▶ react ──silence──▶ idle
   *     │                                  
   *     ├──click──▶ poke (one shot) ──▶ back to react/idle
   *     └──10 min quiet──▶ sleep ──play or click──▶ idle
   *
   * Loops are animated WebP (see scripts/companion-assets.sh) so the decode work
   * stays on the browser's image pipeline: swapping `src` restarts a loop, which
   * is all the state machine needs.
   */
  import { onMount } from "svelte";
  import idle from "./assets/companion/idle.webp";
  import react from "./assets/companion/react.webp";
  import poke from "./assets/companion/poke.webp";
  import sleep from "./assets/companion/sleep.webp";

  export let playing = false;

  type Mood = "idle" | "react" | "poke" | "sleep";
  const LOOP: Record<Mood, string> = { idle, react, poke, sleep };
  /** the poke clip runs once, then she returns to whatever she was doing */
  const POKE_MS = 1400;
  const SLEEP_AFTER_MS = 10 * 60 * 1000;

  let mood: Mood = "idle";
  let pokeTimer: ReturnType<typeof setTimeout> | undefined;
  let quietTimer: ReturnType<typeof setTimeout> | undefined;

  const resting = () => (playing ? "react" : "idle");

  function rest() {
    if (mood !== "sleep") mood = resting();
  }

  function poked() {
    mood = "poke";
    clearTimeout(pokeTimer);
    pokeTimer = setTimeout(rest, POKE_MS);
    armSleepTimer();
  }

  function armSleepTimer() {
    clearTimeout(quietTimer);
    quietTimer = setTimeout(() => {
      if (!playing) mood = "sleep";
    }, SLEEP_AFTER_MS);
  }

  // follow playback, but never interrupt a poke mid-clip
  $: if (mood !== "poke") {
    if (playing && mood !== "react") mood = "react";
    if (!playing && mood === "react") mood = "idle";
  }
  $: if (playing && mood === "sleep") mood = "idle";

  onMount(() => {
    // decode all four up front: a swap at the wrong moment would flash empty
    for (const src of Object.values(LOOP)) new Image().src = src;
    armSleepTimer();
    return () => {
      clearTimeout(pokeTimer);
      clearTimeout(quietTimer);
    };
  });
</script>

<button
  class="companion"
  class:sleeping={mood === "sleep"}
  title="Poke her"
  aria-label="Companion"
  onclick={poked}
>
  <img src={LOOP[mood]} alt="" draggable="false" />
</button>

<style>
  .companion {
    position: fixed;
    right: 16px;
    bottom: 12px;
    z-index: 20;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    /* the artwork carries its own alpha, so there is no card behind her */
    filter: drop-shadow(0 8px 18px rgba(0, 0, 0, 0.45));
    transition:
      transform 180ms ease,
      opacity 400ms ease;
    -webkit-user-select: none;
    user-select: none;
  }
  .companion img {
    display: block;
    height: clamp(150px, 26vh, 240px);
    width: auto;
    pointer-events: none;
  }
  /* hovering lifts her a little; the shadow follows because it is a filter */
  .companion:hover {
    transform: translateY(-3px);
  }
  .companion.sleeping {
    opacity: 0.55;
  }
  .companion:focus-visible {
    outline: 2px solid var(--accent-2, #8b5cf6);
    outline-offset: 6px;
    border-radius: 999px;
  }
  @media (prefers-reduced-motion: reduce) {
    .companion,
    .companion:hover {
      transition: none;
      transform: none;
    }
  }
  /* narrow windows: she would sit on top of the cards */
  @media (max-width: 720px) {
    .companion img {
      height: 130px;
    }
  }
</style>
