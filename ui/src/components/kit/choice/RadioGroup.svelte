<script module>
  let seq = 0;
</script>

<script>
  // Группа радиокнопок на нативных input: стрелки и пробел работают как в браузере.
  // options: строки или { value, label, hint, disabled }. variant="cards" — варианты карточками: название и строка hint под ним.
  let {
    options = [],
    value = $bindable(''),
    name = '',
    legend = '',
    variant = 'list',
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
        ? { value: option.value ?? option.id ?? '', label: option.label ?? option.value ?? option.id ?? '', hint: option.hint ?? '', disabled: Boolean(option.disabled) }
        : { value: String(option), label: String(option), hint: '', disabled: false },
    ),
  );
</script>

<fieldset class="k-radio-group {className}" class:k-radio-group--cards={variant === 'cards'} {...rest}>
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
      <span class="k-radio__label">
        {#if option.hint}
          <span class="k-radio__title">{option.label}</span>
          <span class="k-radio__hint">{option.hint}</span>
        {:else}
          {option.label}
        {/if}
      </span>
    </label>
  {/each}
</fieldset>
