<script>
  import Icon from '../media/Icon.svelte';
  // Круглая кнопка только со значком (.k-btn--icon). Подпись обязательна: она и aria-label, и title.
  let {
    icon = null,
    name = '',
    label = '',
    variant = 'ghost',
    size = 'md',
    disabled = false,
    class: className = '',
    onclick,
    ...rest
  } = $props();

  const variantClass = $derived(
    variant === 'primary'
      ? 'k-btn--primary'
      : variant === 'ink'
        ? 'k-btn--ink'
        : variant === 'ghost'
          ? 'k-btn--ghost'
          : variant === 'attention'
            ? 'k-btn--attention'
            : '',
  );
</script>

<button
  type="button"
  class="k-btn k-btn--icon {variantClass} {className}"
  class:k-btn--sm={size === 'sm'}
  aria-label={label}
  title={label}
  {disabled}
  {onclick}
  {...rest}
>
  <span class="k-i" aria-hidden="true">{#if name}<Icon {name} size={18} />{:else}{@render icon?.()}{/if}</span>
</button>
