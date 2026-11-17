<script>
  // Значок «Угли»: число, точка или короткий текст. Цветов три, статусы различаются ФОРМОЙ:
  // brand/attention = лава (новое, ждёт тебя) · ai = оттенок и живая точка (работает) · neutral = тихий счётчик ·
  // done = нейтральный с оранжевой галочкой · sleep = пустая точка · outline = метка («Бета») · error = красный.
  let {
    tone = 'neutral',
    count = null,
    dot = false,
    text = '',
    max = 99,
    label = '',
    class: className = '',
    children,
    ...rest
  } = $props();

  const shown = $derived(count === null || count === undefined ? '' : count > max ? `${max}+` : String(count));
</script>

<span
  class="k-badge k-badge--{tone} {className}"
  class:is-dot={dot}
  role={label ? 'img' : undefined}
  aria-label={label || undefined}
  aria-hidden={!label && dot ? 'true' : undefined}
  {...rest}
>
  {#if dot}{:else if shown}{shown}{:else if text}{text}{:else}{@render children?.()}{/if}
</span>
