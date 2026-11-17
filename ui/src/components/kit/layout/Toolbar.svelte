<script>
  // Шапка страницы: заголовок, пояснение и действия справа.
  let { title = '', subtitle = '', actions = null, class: className = '', ...rest } = $props();
</script>

<header class="k-toolbar {className}" {...rest}>
  <div class="k-toolbar__text">
    {#if title}<h1 class="k-toolbar__title">{title}</h1>{/if}
    {#if subtitle}<p class="k-toolbar__sub">{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="k-toolbar__actions">{@render actions()}</div>{/if}
</header>
