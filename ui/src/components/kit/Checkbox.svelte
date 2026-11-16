<script>
  // Флажок на токенах кита. Нативный input остаётся доступным и с клавиатуры,
  // видимая рамка — соседний .k-check__box.
  let {
    checked = $bindable(false),
    indeterminate = false,
    label = '',
    disabled = false,
    class: className = '',
    onchange,
    ...rest
  } = $props();

  let input = $state();

  $effect(() => {
    if (input) input.indeterminate = Boolean(indeterminate);
  });
</script>

<label class="k-check {className}" class:is-disabled={disabled}>
  <input
    bind:this={input}
    type="checkbox"
    class="k-check__input"
    bind:checked
    {disabled}
    onchange={(event) => onchange?.(event.currentTarget.checked)}
    {...rest}
  />
  <span class="k-check__box" class:is-indeterminate={indeterminate} aria-hidden="true">
    <svg class="k-check__tick" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M20 6 9 17l-5-5" />
    </svg>
  </span>
  {#if label}<span class="k-check__label">{label}</span>{/if}
</label>
