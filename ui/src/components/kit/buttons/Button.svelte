<script>
  import Icon from '../media/Icon.svelte';
  // Кнопка кита «Угли» (.k-btn, docs/design/kit.md). Вид задаёт variant: primary (лава, одна на экран), secondary (оранжевый контур), ghost, danger (красный контур), danger-solid (подтверждение удаления). Высоту — size.
  // Загрузка показывает девять пикселей «действует» и держит кнопку недоступной (aria-busy).
  let {
    variant = 'secondary',
    size = 'md',
    icon = null,
    iconName = '',
    loading = false,
    disabled = false,
    type = 'button',
    class: className = '',
    onclick,
    children,
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
            : variant === 'danger'
              ? 'k-btn--danger'
              : variant === 'danger-solid'
                ? 'k-btn--danger-solid'
                : '',
  );
</script>

<button
  {type}
  class="k-btn {variantClass} {className}"
  class:k-btn--sm={size === 'sm'}
  class:k-btn--loading={loading}
  disabled={disabled || loading}
  aria-busy={loading ? 'true' : undefined}
  {onclick}
  {...rest}
>
  {#if loading}
    <span class="k-btn__pix k-pix k-pix--act" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
  {:else if iconName}
    <span class="k-btn__icon" aria-hidden="true"><Icon name={iconName} size={16} /></span>
  {:else if icon}
    <span class="k-btn__icon" aria-hidden="true">{@render icon()}</span>
  {/if}
  {#if children}{@render children()}{/if}
</button>
