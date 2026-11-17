<script>
  // Выбор внутри карточки как сетка-переключатель (.k-choice): выбранный — поверхность с оранжевым кольцом.
  // Вариант с need: true (нужно подключение) не прячется, а отмечен оранжевой точкой.
  // items: [{ id, label, title?, need?, disabled? }]; значок вариантов — сниппет icon(item).
  let { items = [], value = '', label = '', columns = 3, class: className = '', onchange, icon, ...rest } = $props();
</script>

<div class="k-choice {className}" role="group" aria-label={label || undefined} style:--k-choice-cols={columns} {...rest}>
  {#each items as item (item.id)}
    <button
      type="button"
      class="k-choice__opt"
      class:is-need={item.need}
      aria-pressed={value === item.id}
      title={item.title || undefined}
      disabled={item.disabled}
      onclick={() => onchange?.(item.id)}
    >
      {#if icon}<span class="k-choice__ico" aria-hidden="true">{@render icon(item)}</span>{/if}
      <span class="k-choice__lbl">{item.label}</span>
    </button>
  {/each}
</div>
