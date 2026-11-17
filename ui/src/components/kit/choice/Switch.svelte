<script>
  // Переключатель на .k-switch: кнопка с role="switch", Enter и пробел переключают.
  let {
    checked = $bindable(false),
    label = '',
    disabled = false,
    class: className = '',
    onchange,
    ...rest
  } = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }

  function keydown(event) {
    if (event.key !== ' ' && event.key !== 'Enter') return;
    event.preventDefault();
    toggle();
  }
</script>

<button
  type="button"
  class="k-switch-button {className}"
  role="switch"
  aria-checked={checked}
  aria-label={label || undefined}
  {disabled}
  onclick={toggle}
  onkeydown={keydown}
  {...rest}
>
  <span class="k-switch" aria-hidden="true"></span>
  {#if label}<span class="k-switch__label">{label}</span>{/if}
</button>
