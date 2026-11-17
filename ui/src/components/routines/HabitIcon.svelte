<script>
  // Значок привычки: имя из набора приложения рисуем штрихом, любой другой знак (эмодзи) выводим текстом.
  import { habitIconName } from '../../lib/habitIcons.js';
import { Icon } from '../kit';

  let { name = '', size = 22 } = $props();

  const iconName = $derived(habitIconName(name));
</script>

{#if iconName}
  <Icon name={iconName} {size} stroke={1.8} class="hi" />
{:else}
  <span class="hi hi--text" style:font-size="{Math.round(size * 0.9)}px" aria-hidden="true">{name}</span>
{/if}

<style>
  :global(.hi) {
    flex: none;
    display: block;
  }
  .hi--text {
    line-height: 1;
  }
</style>
