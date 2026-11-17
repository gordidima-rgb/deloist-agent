<script>
  // Уведомление кита «Угли»: тёмный уголь с оранжевым значком, одна строка и не больше одного действия.
  import Icon from './Icon.svelte';
  let { text = '', icon = '', action = null, class: className = '', ...rest } = $props();
</script>

<div class="k-toast {className}" role="status" {...rest}>
  {#if icon}<Icon name={icon} size={null} class="k-toast__icon" />{/if}
  <span class="k-toast__text">{text}</span>
  {#if action}
    <button type="button" class="k-toast__action" onclick={action.onclick}>{action.label}</button>
  {/if}
</div>
