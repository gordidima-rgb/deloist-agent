<script>
  // Строка списка в общей рамке (.k-lrow, правило «раздел — карточка, внутри строки в одной рамке»).
  // Живёт внутри <ul class="k-lrows">. Значок, заголовок, подпись и место справа (trailing).
  // Виды: variant="chevron" — строка нажимается и раскрывает children по высоте (expanded, bindable);
  // "action" — справа кнопка (actionLabel + onaction), сама строка не нажимается;
  // leading — сниппет перед значком (например, отметка), meta — сниппет вместо sub (несколько строк подписи).
  // "switch" — справа переключатель (checked, bindable; onchange); без variant — просто строка, onclick делает её кнопкой.
  import Icon from '../media/Icon.svelte';
  import Button from '../buttons/Button.svelte';
  import Switch from '../choice/Switch.svelte';

  let {
    icon = '',
    title = '',
    sub = '',
    variant = 'plain',
    expanded = $bindable(false),
    checked = $bindable(false),
    actionLabel = '',
    actionVariant = 'secondary',
    onaction,
    onchange,
    onclick,
    done = false,
    titleClass = '',
    leading = null,
    meta = null,
    class: className = '',
    trailing = null,
    children = null,
    ...rest
  } = $props();

  const uid = `lrow-${Math.random().toString(36).slice(2, 8)}`;

  function press() {
    if (variant === 'chevron') expanded = !expanded;
    onclick?.();
  }
</script>

<li class="k-lrow {className}" class:is-open={expanded} class:is-done={done} {...rest}>
  {#snippet text()}
    <b class="k-lrow__title {titleClass}">{title}</b>
    {#if meta}{@render meta()}{:else if sub}<small class="k-lrow__sub">{sub}</small>{/if}
  {/snippet}
  {#snippet head()}
    {#if leading}{@render leading()}{/if}
    {#if icon}<span class="k-lrow__ico"><Icon name={icon} size={null} /></span>{/if}
  {/snippet}
  {#snippet tail()}
    {#if trailing}<span class="k-lrow__end">{@render trailing()}</span>{/if}
  {/snippet}

  {#if variant === 'chevron'}
    <button type="button" class="k-lrow__main" aria-expanded={expanded} aria-controls={uid} onclick={press}>
      {@render head()}
      <span class="k-lrow__t">{@render text()}</span>
      {@render tail()}
      <Icon name="chevron-right" size={null} class="k-lrow__chev" />
    </button>
  {:else}
    <div class="k-lrow__main is-static">
      {@render head()}
      {#if onclick}
        <button type="button" class="k-lrow__t k-lrow__t--btn" onclick={press}>{@render text()}</button>
      {:else}
        <span class="k-lrow__t">{@render text()}</span>
      {/if}
      {@render tail()}
      {#if variant === 'action'}
        <Button size="sm" variant={actionVariant} onclick={onaction}>{actionLabel}</Button>
      {:else if variant === 'switch'}
        <Switch bind:checked aria-label={title} {onchange} />
      {/if}
    </div>
  {/if}

  {#if variant === 'chevron' && children}
    <div class="k-lrow__acc" id={uid} {...(expanded ? {} : { inert: true })}>
      <div class="k-lrow__in">{@render children()}</div>
    </div>
  {/if}
</li>
