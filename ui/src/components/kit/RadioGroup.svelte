<script module>
  let seq = 0;
</script>

<script>
  // Группа радиокнопок на нативных input: стрелки и пробел работают как в браузере.
  // options: строки или { value, label, disabled }.
  let {
    options = [],
    value = $bindable(''),
    name = '',
    legend = '',
    disabled = false,
    class: className = '',
    onchange,
    ...rest
  } = $props();

  const uid = `k-radio-${++seq}`;
  const group = $derived(name || uid);

  const list = $derived(
    (options ?? []).map((option) =>
      option !== null && typeof option === 'object'
        ? { value: option.value ?? option.id ?? '', label: option.label ?? option.value ?? option.id ?? '', disabled: Boolean(option.disabled) }
        : { value: String(option), label: String(option), disabled: false },
    ),
  );
</script>

<fieldset class="k-radio-group {className}" {...rest}>
  {#if legend}<legend class="k-label">{legend}</legend>{/if}
  {#each list as option (option.value)}
    <label class="k-radio" class:is-disabled={disabled || option.disabled}>
      <input
        type="radio"
        class="k-radio__input"
        name={group}
        value={option.value}
        bind:group={value}
        disabled={disabled || option.disabled}
        onchange={(event) => onchange?.(event.currentTarget.value)}
      />
      <span class="k-radio__dot" aria-hidden="true"></span>
      <span class="k-radio__label">{option.label}</span>
    </label>
  {/each}
</fieldset>
