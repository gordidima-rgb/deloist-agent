<script>
  // Круглая кнопка только со значком (.k-btn--icon). Подпись обязательна: она и aria-label, и title.
  let {
    icon,
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
  <span class="k-i" aria-hidden="true">{@render icon?.()}</span>
</button>
