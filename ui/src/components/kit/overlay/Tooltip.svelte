<script module>
  let seq = 0;
</script>

<script>
  // Подсказка на островке туши. Показывается при наведении и фокусе с клавиатуры;
  // первому элементу внутри children ставится aria-describedby.
  // wrap — подсказка переносит строки и не шире 240 px (для длинных пояснений); align="start" прижимает её левым краем к значку.
  let { text = '', placement = 'top', wrap = false, align = 'center', class: className = '', children, ...rest } = $props();

  const uid = `k-tip-${++seq}`;
  let root = $state();

  $effect(() => {
    const child = root?.firstElementChild;
    if (!child || !text) return;
    const previous = child.getAttribute('aria-describedby');
    child.setAttribute('aria-describedby', previous ? `${previous} ${uid}` : uid);
    return () => {
      if (previous) child.setAttribute('aria-describedby', previous);
      else child.removeAttribute('aria-describedby');
    };
  });
</script>

<span class="k-tip k-tip--{placement} {className}" class:k-tip--wrap={wrap} class:k-tip--start={align === 'start'} bind:this={root} {...rest}>
  {@render children?.()}
  {#if text}<span class="k-tooltip" role="tooltip" id={uid}>{text}</span>{/if}
</span>
