<script>
  // Группа настроек: заголовок, пояснение, действия справа и тело. Строки внутри — Row.
  let { title = '', description = '', actions = null, class: className = '', children, ...rest } = $props();
</script>

<section class="k-section {className}" {...rest}>
  {#if title || description || actions}
    <header class="k-section__head">
      <div class="k-section__text">
        {#if title}<h2 class="k-section__title">{title}</h2>{/if}
        {#if description}<p class="k-section__desc">{description}</p>{/if}
      </div>
      {#if actions}<div class="k-section__actions">{@render actions()}</div>{/if}
    </header>
  {/if}
  {#if children}<div class="k-section__body">{@render children()}</div>{/if}
</section>
