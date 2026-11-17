<script>
  import { tick } from 'svelte';
  import Icon from '../media/Icon.svelte';
  // Выпадающий список кита «Угли»: своя кнопка и свой список вместо списка ОС (тот не слушает тему).
  // Контракт прежний: options (строки или { value, label, disabled }), bind:value, onchange(event) с
  // event.currentTarget.value. Под капотом лежит скрытый нативный select: он держит значение и
  // получает настоящее событие change, поэтому формы и обработчики экранов не меняются.
  // Клавиатура: стрелки, Home, End, Enter и пробел выбирают, Esc закрывает, Tab закрывает и идёт дальше.
  let {
    options = [],
    value = $bindable(''),
    placeholder = '',
    id = '',
    describedby = '',
    invalid = false,
    disabled = false,
    class: className = '',
    onchange,
    ...rest
  } = $props();

  const uid = Math.random().toString(36).slice(2, 8);
  const listId = `k-sel-${uid}`;

  const list = $derived(
    (options ?? []).map((option) =>
      option !== null && typeof option === 'object'
        ? { value: option.value ?? option.id ?? '', label: option.label ?? option.value ?? option.id ?? '', disabled: Boolean(option.disabled) }
        : { value: String(option), label: String(option), disabled: false },
    ),
  );

  let open = $state(false);
  let up = $state(false);
  let active = $state(-1);
  let native = $state();
  let button = $state();
  let menu = $state();

  const selected = $derived(list.findIndex((option) => option.value === value));
  const shown = $derived(selected >= 0 ? list[selected].label : placeholder);

  const enabled = (from, step) => {
    for (let i = from; i >= 0 && i < list.length; i += step) if (!list[i].disabled) return i;
    return -1;
  };

  async function show() {
    if (disabled || open || !list.length) return;
    active = selected >= 0 ? selected : enabled(0, 1);
    const rect = button?.getBoundingClientRect();
    up = Boolean(rect) && window.innerHeight - rect.bottom < 280 && rect.top > window.innerHeight - rect.bottom;
    open = true;
    await tick();
    menu?.querySelector('[data-active="true"]')?.scrollIntoView?.({ block: 'nearest' });
  }

  function hide() {
    open = false;
  }

  function choose(index) {
    const option = list[index];
    if (!option || option.disabled) return;
    hide();
    button?.focus({ preventScroll: true });
    if (option.value === value) return;
    value = option.value;
    // Нативный select держит значение; настоящее событие change идёт обработчикам экрана.
    if (native) {
      native.value = option.value;
      native.dispatchEvent(new Event('change', { bubbles: true }));
    }
  }

  function move(index) {
    if (index < 0) return;
    active = index;
    tick().then(() => menu?.querySelector('[data-active="true"]')?.scrollIntoView?.({ block: 'nearest' }));
  }

  function keydown(event) {
    if (disabled) return;
    const key = event.key;
    if (!open) {
      if (key === 'ArrowDown' || key === 'ArrowUp' || key === 'Enter' || key === ' ') {
        event.preventDefault();
        show();
      }
      return;
    }
    if (key === 'Escape') { event.preventDefault(); event.stopPropagation(); hide(); }
    else if (key === 'Tab') hide();
    else if (key === 'ArrowDown') { event.preventDefault(); move(enabled(active + 1, 1)); }
    else if (key === 'ArrowUp') { event.preventDefault(); move(enabled(active - 1, -1)); }
    else if (key === 'Home') { event.preventDefault(); move(enabled(0, 1)); }
    else if (key === 'End') { event.preventDefault(); move(enabled(list.length - 1, -1)); }
    else if (key === 'Enter' || key === ' ') { event.preventDefault(); choose(active); }
  }

  $effect(() => {
    if (!open) return;
    const outside = (event) => {
      if (!button?.parentElement?.contains(event.target)) hide();
    };
    window.addEventListener('pointerdown', outside, true);
    return () => window.removeEventListener('pointerdown', outside, true);
  });
</script>

<span class="k-select" class:is-open={open} class:is-up={up}>
  <select bind:this={native} class="k-select__native" bind:value tabindex="-1" aria-hidden="true" {disabled} {onchange}>
    {#if placeholder}<option value="" disabled>{placeholder}</option>{/if}
    {#each list as option (option.value)}
      <option value={option.value} disabled={option.disabled}>{option.label}</option>
    {/each}
  </select>
  <button
    bind:this={button}
    type="button"
    {id}
    class="k-input k-select__input {className}"
    role="combobox"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={listId}
    aria-activedescendant={open && active >= 0 ? `${listId}-${active}` : undefined}
    aria-invalid={invalid ? 'true' : undefined}
    aria-describedby={describedby || undefined}
    {disabled}
    onclick={() => (open ? hide() : show())}
    onkeydown={keydown}
    {...rest}
  >
    <span class="k-select__value" class:is-placeholder={selected < 0}>{shown}</span>
  </button>
  <Icon name="chevron-down" size={null} stroke={2.5} class="k-select__chevron" />
  {#if open}
    <ul bind:this={menu} id={listId} class="k-select__menu" role="listbox" tabindex="-1">
      {#each list as option, i (option.value)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="{listId}-{i}"
          class="k-select__opt"
          role="option"
          aria-selected={i === selected}
          aria-disabled={option.disabled ? 'true' : undefined}
          data-active={i === active}
          style="--k-i: {Math.min(i, 8)}"
          onpointerenter={() => !option.disabled && (active = i)}
          onpointerdown={(event) => event.preventDefault()}
          onclick={() => choose(i)}
        >
          <span class="k-select__label">{option.label}</span>
          <span class="k-select__check" aria-hidden="true"></span>
        </li>
      {/each}
    </ul>
  {/if}
</span>
