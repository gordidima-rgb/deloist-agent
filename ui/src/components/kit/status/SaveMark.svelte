<script>
  import Icon from '../media/Icon.svelte';
  // Состояние сохранения (.k-saved): у «Сохранено» галочка рисуется сама. state: saved | saving | dirty | error.
  let { state = 'saved', text = '', class: className = '', ...rest } = $props();
</script>

<span class="k-saved {className}" data-state={state} role="status" {...rest}>
  {#if !['saving', 'dirty', 'error'].includes(state)}
    <Icon name="check" class="k-saved__tick" size={14} stroke={2.4} />
  {/if}
  <span>{text}</span>
</span>
