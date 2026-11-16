<script>
  // Нативный select на токенах кита: рамка и фокус как у .k-input, своя стрелка.
  // options: строки или { value, label, disabled }.
  let {
    options = [],
    value = $bindable(''),
    placeholder = '',
    id = '',
    describedby = '',
    invalid = false,
    disabled = false,
    class: className = '',
    ...rest
  } = $props();

  const list = $derived(
    (options ?? []).map((option) =>
      option !== null && typeof option === 'object'
        ? { value: option.value ?? option.id ?? '', label: option.label ?? option.value ?? option.id ?? '', disabled: Boolean(option.disabled) }
        : { value: String(option), label: String(option), disabled: false },
    ),
  );
</script>

<span class="k-select">
  <select
    {id}
    bind:value
    class="k-input k-select__input {className}"
    aria-invalid={invalid ? 'true' : undefined}
    aria-describedby={describedby || undefined}
    {disabled}
    {...rest}
  >
    {#if placeholder}<option value="" disabled>{placeholder}</option>{/if}
    {#each list as option (option.value)}
      <option value={option.value} disabled={option.disabled}>{option.label}</option>
    {/each}
  </select>
  <svg class="k-select__chevron" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m4 6 4 4 4-4" />
  </svg>
</span>
