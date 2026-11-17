<script>
  // Рисованные значки инструментов нижней панели: карандаш, жёлтая заметка, стопка фигур.
  // Цвета свои, нарисованы здесь; на светлой и тёмной темах читаются одинаково.
  let { name, size = 30 } = $props();
</script>

<svg class="wf-art wf-art--{name}" viewBox="0 0 32 32" width={size} height={size} aria-hidden="true">
  {#if name === 'pencil'}
    <g transform="rotate(40 16 16)">
      <rect x="12.5" y="2" width="7" height="5" rx="1.8" fill="#ff8fa6" />
      <rect x="12.5" y="6.5" width="7" height="3" fill="#aab3c2" />
      <rect x="12.5" y="9.5" width="7" height="13.5" fill="#ffc43a" />
      <rect x="16.2" y="9.5" width="3.3" height="13.5" fill="#f0a20f" />
      <rect x="12.5" y="9.5" width="1.2" height="13.5" fill="#ffe08a" />
      <path d="M12.5 23h7L16 30.2z" fill="#f3d6a8" />
      <path d="M12.5 23h7l-1.1 2.6h-4.8z" fill="#e6c08a" />
      <path d="M14.6 27.2h2.8L16 30.2z" fill="#363944" />
    </g>
  {:else if name === 'note'}
    <rect x="4" y="4" width="24" height="24" rx="5" fill="#ff9d2e" />
    <path d="M28 21.5V23a5 5 0 0 1-5 5h-1.5c0-4 2-6.5 6.5-6.5z" fill="#ffc27a" />
    <rect x="4" y="4" width="24" height="8" rx="5" fill="#ffb454" opacity="0.55" />
    <path d="M9 14h10M9 18h7" stroke="#b85f0a" stroke-width="1.8" stroke-linecap="round" opacity="0.55" />
  {:else if name === 'shapes'}
    <circle cx="20" cy="20" r="9" fill="#9aa1ae" />
    <circle cx="17" cy="17" r="9" fill="#b9bfca" opacity="0.5" />
    <rect x="4.5" y="4.5" width="17" height="17" rx="4.5" fill="#ffffff" />
    <rect x="4.5" y="4.5" width="17" height="17" rx="4.5" fill="none" stroke="#d3d6dd" stroke-width="1" />
  {/if}
</svg>

<style>
  .wf-art {
    display: block;
    overflow: visible;
    filter: drop-shadow(0 2px 2.5px color-mix(in oklab, var(--k-ink) 28%, transparent));
    transition:
      transform 360ms var(--k-ease-spring, cubic-bezier(0.34, 1.36, 0.5, 1)),
      filter 220ms var(--k-ease-out, ease);
  }
  :global(.wf-tool:hover) .wf-art {
    transform: translateY(-3px) scale(1.08);
    filter: drop-shadow(0 6px 5px color-mix(in oklab, var(--k-ink) 30%, transparent));
  }
  /* Выбранный инструмент: карандаш выдвигается вверх, как будто его взяли в руку. */
  :global(.wf-tool.on) .wf-art--pencil {
    transform: translateY(-7px) rotate(-6deg) scale(1.1);
  }
  :global(.wf-tool.on) .wf-art--note,
  :global(.wf-tool.on) .wf-art--shapes {
    transform: translateY(-5px) scale(1.1);
  }
  :global(.wf-tool:active) .wf-art {
    transform: translateY(0) scale(0.94);
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-art {
      transition: none;
    }
  }
</style>
