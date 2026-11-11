<script lang="ts">
  /**
   * The app's cat, drawn inline so it stays crisp at any size and can blink.
   * Used where the UI has nothing to show — a bare warning triangle says
   * "something broke", she says "nothing here yet".
   */
  let { size = 64, still = false }: { size?: number; still?: boolean } = $props();
</script>

<svg
  class="mark"
  class:still
  width={size}
  height={size}
  viewBox="0 0 100 100"
  aria-hidden="true"
>
  <defs>
    <linearGradient id="wagmify-mark" x1="0" y1="0" x2="0.3" y2="1">
      <stop offset="0" stop-color="#b49cff" />
      <stop offset="1" stop-color="#cbb9ff" />
    </linearGradient>
  </defs>
  <rect x="2" y="2" width="96" height="96" rx="26" fill="url(#wagmify-mark)" />
  <g fill="#0b0a0f">
    <path d="M33 44 L28 13 L50 28 Z" />
    <path d="M67 44 L72 13 L50 28 Z" />
    <path d="M50 24 C68 24 79 36 79 53 C79 68 66 78 50 78 C34 78 21 68 21 53 C21 36 32 24 50 24 Z" />
  </g>
  <g class="eyes" fill="#fff">
    <ellipse cx="40" cy="50" rx="6.2" ry="8.4" transform="rotate(8 40 50)" />
    <ellipse cx="60" cy="50" rx="6.2" ry="8.4" transform="rotate(-8 60 50)" />
  </g>
</svg>

<style>
  .mark {
    display: block;
    filter: drop-shadow(0 6px 16px rgb(140 110 255 / 0.28));
    animation: bob 5s ease-in-out infinite;
  }
  .still { animation: none; }

  .eyes {
    transform-origin: 50px 50px;
    animation: blink 6.5s ease-in-out infinite;
  }

  @keyframes bob {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-3px); }
  }

  /* a blink every few seconds, long enough to notice, short enough to miss */
  @keyframes blink {
    0%, 92%, 100% { transform: scaleY(1); }
    95%, 96% { transform: scaleY(0.12); }
  }

  @media (prefers-reduced-motion: reduce) {
    .mark, .eyes { animation: none; }
  }
</style>
