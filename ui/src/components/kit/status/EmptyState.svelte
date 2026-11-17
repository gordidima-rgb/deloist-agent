<script>
  // Пустое состояние: знак, заголовок, пояснение и необязательное действие.
  let { icon = null, title = '', text = '', action = null, class: className = '', ...rest } = $props();
</script>

<div class="k-empty {className}" {...rest}>
  {#if icon}<div class="k-empty__icon" aria-hidden="true">{@render icon()}</div>{/if}
  {#if title}<h3 class="k-empty__title">{title}</h3>{/if}
  {#if text}<p class="k-empty__text">{text}</p>{/if}
  {#if action}<div class="k-empty__action">{@render action()}</div>{/if}
</div>
