<script>
  // Строка настроек: подпись слева, элемент управления справа. children — сам элемент справа.
  let { label = '', sublabel = '', class: className = '', children, ...rest } = $props();
</script>

<div class="k-row {className}" {...rest}>
  <div class="k-row__text">
    {#if label}<span class="k-row__label">{label}</span>{/if}
    {#if sublabel}<span class="k-row__sub">{sublabel}</span>{/if}
  </div>
  {#if children}<div class="k-row__control">{@render children()}</div>{/if}
</div>
