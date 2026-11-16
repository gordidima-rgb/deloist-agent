<script>
  // Многострочное поле на .k-input.k-textarea. autosize подгоняет высоту под текст;
  // при reduced motion это не движение, а мгновенный пересчёт.
  let {
    value = $bindable(''),
    autosize = true,
    rows = 3,
    id = '',
    describedby = '',
    invalid = false,
    disabled = false,
    class: className = '',
    oninput,
    ...rest
  } = $props();

  let el = $state();

  function resize() {
    if (!el) return;
    if (!autosize) {
      el.style.height = '';
      return;
    }
    el.style.height = 'auto';
    const height = el.scrollHeight;
    if (height > 0) el.style.height = `${height + 2}px`;
  }

  function input(event) {
    resize();
    oninput?.(event);
  }

  $effect(() => {
    value;
    resize();
  });
</script>

<textarea
  {id}
  bind:this={el}
  bind:value
  {rows}
  class="k-input k-textarea {className}"
  class:is-autosize={autosize}
  aria-invalid={invalid ? 'true' : undefined}
  aria-describedby={describedby || undefined}
  {disabled}
  oninput={input}
  {...rest}
></textarea>
