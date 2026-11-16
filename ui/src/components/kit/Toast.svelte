<script>
  // Уведомление кита: чёрная капсула, одна строка и не больше одного действия.
  let { text = '', action = null, class: className = '', ...rest } = $props();
</script>

<div class="k-toast {className}" role="status" {...rest}>
  <span class="k-toast__text">{text}</span>
  {#if action}
    <button type="button" class="k-toast__action" onclick={action.onclick}>{action.label}</button>
  {/if}
</div>
