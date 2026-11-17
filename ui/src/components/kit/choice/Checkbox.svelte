<script>
  import Icon from '../media/Icon.svelte';
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
    <Icon name="tick" size={null} stroke={3.3} class="k-check__tick" />
  </span>
  {#if label}<span class="k-check__label">{label}</span>{/if}
</label>
