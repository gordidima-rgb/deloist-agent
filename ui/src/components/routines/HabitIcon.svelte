<script>
  // Значок привычки: имя из набора приложения рисуем штрихом, любой другой знак (эмодзи) выводим текстом.
  import { HABIT_ICONS } from '../../lib/habitIcons.js';

  let { name = '', size = 22 } = $props();

  const markup = $derived(HABIT_ICONS[name] ?? '');
</script>

{#if markup}
  <svg class="hi" viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html markup}</svg>
{:else}
  <span class="hi hi--text" style:font-size="{Math.round(size * 0.9)}px" aria-hidden="true">{name}</span>
{/if}

<style>
  .hi {
    flex: none;
    display: block;
  }
  .hi--text {
    line-height: 1;
  }
</style>
