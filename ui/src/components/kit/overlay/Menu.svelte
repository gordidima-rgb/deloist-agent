<script>
  // Меню кита «Угли» (.k-menu): островок туши со строками, растёт из кнопки, пункты входят каскадом.
  // Пункты — данные: { id, label, kind: 'item' | 'check' | 'sep' | 'note', checked, disabled, hint, shortcut, danger, icon, autofocus }.
  // icon — имя значка из lib/icons.js, hint — тихая подпись справа, shortcut — сочетание клавиш справа,
  // disabled — серая строка, tip — её подсказка. Стрелки, Home и End ходят по пунктам; Esc и щелчок мимо закрывают (onclose).
  // Без x/y меню заполняет свой контейнер; с x/y встаёт в точку внутри родителя (контекстное меню) и не выходит за край.
  // origin — откуда растёт: 'top-left' | 'bottom-left' | 'top-right' | 'bottom-right'. Текст уже переведён вызывающим.
  import Icon from '../media/Icon.svelte';

  let { items = [], label = '', x = null, y = null, origin = 'top-left', class: className = '', onpick, onclose } = $props();

  let root = $state();
  let placed = $state(false);
  const floating = $derived(x !== null && y !== null);

  const focusables = () => [...(root?.querySelectorAll('.k-menu__item:not([aria-disabled="true"])') ?? [])];

  $effect(() => {
    // Открылось или сменился набор пунктов (например, подтверждение): фокус на первый доступный.
    items;
    if (floating && !placed) return;
    queueMicrotask(() => (root?.querySelector('[data-autofocus]') ?? focusables()[0])?.focus({ preventScroll: true }));
  });

  $effect(() => {
    // Контекстное меню: меряем настоящий размер и ставим внутрь родителя.
    if (!floating) return;
    items;
    const parent = root?.parentElement;
    if (!parent) return;
    const w = root.offsetWidth;
    const h = root.offsetHeight;
    root.style.left = `${Math.min(Math.max(8, x), parent.clientWidth - w - 8)}px`;
    root.style.top = `${Math.min(Math.max(8, y), parent.clientHeight - h - 8)}px`;
    placed = true;
  });

  function onkey(event) {
    const list = focusables();
    if (!list.length) return;
    const at = list.indexOf(document.activeElement);
    let next = -1;
    if (event.key === 'ArrowDown') next = (at + 1) % list.length;
    else if (event.key === 'ArrowUp') next = (at - 1 + list.length) % list.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = list.length - 1;
    else if (event.key === 'Escape') {
      event.stopPropagation();
      onclose?.(true);
      return;
    } else if (event.key === 'Tab') {
      onclose?.(false);
      return;
    }
    if (next < 0) return;
    event.preventDefault();
    list[next].focus({ preventScroll: true });
  }

  function pick(item) {
    if (item.disabled) return;
    onpick?.(item.id);
  }

  function onoutside(event) {
    if (root && !root.contains(event.target) && !event.target.closest?.('[data-menu-trigger]')) onclose?.(false);
  }
</script>

<svelte:document onpointerdown={onoutside} />

<div
  class="k-menu k-menu--cascade {className}"
  class:k-menu--float={floating}
  class:is-placed={!floating || placed}
  class:k-sharpen-in={!floating}
  data-origin={origin}
  role="menu"
  aria-label={label}
  tabindex="-1"
  bind:this={root}
  onkeydown={onkey}
>
  {#each items as item, i (item.id ?? `sep${i}`)}
    {#if item.kind === 'sep'}
      <div class="k-menu__sep" role="separator"></div>
    {:else if item.kind === 'note'}
      <div class="k-menu__note">{item.label}</div>
    {:else}
      <button
        type="button"
        class="k-menu__item"
        class:is-danger={item.danger}
        class:is-off={item.disabled}
        style:--i={i}
        role={item.kind === 'check' ? 'menuitemcheckbox' : 'menuitem'}
        aria-checked={item.kind === 'check' ? Boolean(item.checked) : undefined}
        aria-disabled={item.disabled ? 'true' : undefined}
        title={item.disabled ? item.tip || item.hint : undefined}
        tabindex="-1"
        data-autofocus={item.autofocus ? '' : undefined}
        onclick={() => pick(item)}
      >
        {#if item.icon}
          <Icon name={item.icon} size={null} stroke={1.8} class="k-i" />
        {/if}
        <span class="k-menu__label">{item.label}</span>
        {#if item.kind === 'check'}
          <span class="k-switch"></span>
        {:else if item.shortcut}
          <kbd class="k-menu__kbd">{item.shortcut}</kbd>
        {:else if item.hint}
          <span class="k-menu__value">{item.hint}</span>
        {/if}
      </button>
    {/if}
  {/each}
</div>
