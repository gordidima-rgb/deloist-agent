<script>
  // Состояние сохранения (.k-saved): у «Сохранено» галочка рисуется сама. state: saved | saving | dirty | error.
  let { state = 'saved', text = '', class: className = '', ...rest } = $props();
</script>

<span class="k-saved {className}" data-state={state} role="status" {...rest}>
  {#if !['saving', 'dirty', 'error'].includes(state)}
    <svg class="k-saved__tick" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
  {/if}
  <span>{text}</span>
</span>
