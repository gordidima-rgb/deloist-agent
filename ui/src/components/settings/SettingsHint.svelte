<script>
  // «?» рядом с подписью: длинное пояснение живёт в подсказке кита, а не в потоке строк.
  import Tooltip from '../kit/Tooltip.svelte';

  let { text = '', label = '?' } = $props();
</script>

<Tooltip {text} placement="right" class="s-hint-tip">
  <button type="button" class="s-hint" aria-label={label}>?</button>
</Tooltip>
