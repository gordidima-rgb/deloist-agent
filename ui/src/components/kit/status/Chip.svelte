<script>
  // Фишка кита: капсула с точкой цвета бота. Выбранная становится тушью.
  // Ссылка (href) не переключается сама; кнопка переключает selected.
  let {
    selected = $bindable(false),
    ref = $bindable(null),
    color = '',
    href = '',
    class: className = '',
    onclick,
    children,
    ...rest
  } = $props();

  function press(event) {
    selected = !selected;
    onclick?.(event);
  }
</script>

{#if href}
  <a class="k-chip {className}" {href} aria-current={selected ? 'page' : undefined} {onclick} {...rest}>
    {#if color}<span class="k-dot" style:background={color} aria-hidden="true"></span>{/if}
    {@render children?.()}
  </a>
{:else}
  <button bind:this={ref} type="button" class="k-chip {className}" aria-pressed={selected} onclick={press} {...rest}>
    {#if color}<span class="k-dot" style:background={color} aria-hidden="true"></span>{/if}
    {@render children?.()}
  </button>
{/if}
