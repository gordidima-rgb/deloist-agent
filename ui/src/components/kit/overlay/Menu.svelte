<script>
  // Меню кита «Угли» (.k-menu): островок туши со строками, растёт из кнопки, пункты входят каскадом.
  // Пункты — данные: { id, label, kind: 'item' | 'check' | 'sep' | 'note', checked, disabled, hint, shortcut, danger, icon, autofocus }.
  // icon — имя значка из lib/icons.js, hint — тихая подпись справа, shortcut — сочетание клавиш справа,
  // disabled — серая строка, tip — её подсказка. Стрелки, Home и End ходят по пунктам; Esc и щелчок мимо закрывают (onclose).
  // search — подпись поля поиска (строка): над пунктами появляется поле, оно фильтрует по label, sub и keywords; empty — текст, когда ничего не найдено.
  // kind 'group' — заголовок группы (пустые при поиске скрываются), sub — вторая строка пункта, lead — сниппет значка вместо icon, fromBottom — y это нижний край меню.
  // Без x/y меню заполняет свой контейнер; с x/y встаёт в точку внутри родителя (контекстное меню) и не выходит за край.
  // origin — откуда растёт: 'top-left' | 'bottom-left' | 'top-right' | 'bottom-right'. Текст уже переведён вызывающим.
  import Icon from '../media/Icon.svelte';

  let { items = [], label = '', x = null, y = null, origin = 'top-left', search = '', empty = '', fromBottom = false, lead = null, class: className = '', onpick, onclose } = $props();

  let query = $state('');
  let field = $state(null);
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!search || !q) return items;
    const hit = (it) => `${it.label ?? ''} ${it.sub ?? ''} ${it.keywords ?? ''}`.toLowerCase().includes(q);
    const out = [];
    let head = null;
    for (const it of items) {
      if (it.kind === 'group') head = it;
      else if (it.kind === 'sep' || it.kind === 'note') continue;
      else if (hit(it)) {
        if (head) out.push(head);
        head = null;
        out.push(it);
      }
    }
    return out;
  });

  let root = $state();
  let placed = $state(false);
  const floating = $derived(x !== null && y !== null);

  const focusables = () => [...(root?.querySelectorAll('.k-menu__item:not([aria-disabled="true"])') ?? [])];

  $effect(() => {
    // Открылось или сменился набор пунктов (например, подтверждение): фокус на первый доступный.
    items;
    if (floating && !placed) return;
    queueMicrotask(() => (field ?? root?.querySelector('[data-autofocus]') ?? focusables()[0])?.focus({ preventScroll: true }));
  });

  $effect(() => {
    // Контекстное меню: меряем настоящий размер и ставим внутрь родителя.
    if (!floating) return;
    shown;
    const parent = root?.parentElement;
    if (!parent) return;
    const w = root.offsetWidth;
    const h = root.offsetHeight;
    root.style.left = `${Math.min(Math.max(8, x), parent.clientWidth - w - 8)}px`;
    root.style.top = `${Math.min(Math.max(8, fromBottom ? y - h : y), parent.clientHeight - h - 8)}px`;
    placed = true;
  });

  function onkey(event) {
    const list = focusables();
    if (event.target === field && event.key === 'Enter') {
      event.preventDefault();
      list[0]?.click();
      return;
    }
    if (!list.length && event.key !== 'Escape') return;
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
  {#if search}
    <label class="k-menu__search">
      <Icon name="search-short" size={null} class="k-i" />
      <input bind:this={field} bind:value={query} type="search" placeholder={search} aria-label={search} autocomplete="off" />
    </label>
  {/if}
  {#each shown as item, i (item.id ?? `${item.kind}${i}`)}
    {#if item.kind === 'group'}
      <div class="k-menu__group" role="presentation">{item.label}</div>
    {:else if item.kind === 'sep'}
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
        {#if lead}
          <span class="k-menu__lead">{@render lead(item)}</span>
        {:else if item.icon}
          <Icon name={item.icon} size={null} stroke={1.8} class="k-i" />
        {/if}
        {#if item.sub}
          <span class="k-menu__label k-menu__two"><b>{item.label}</b><small>{item.sub}</small></span>
        {:else}
          <span class="k-menu__label">{item.label}</span>
        {/if}
        {#if item.kind === 'check'}
          <span class="k-switch"></span>
        {:else if item.shortcut}
          <kbd class="k-menu__kbd">{item.shortcut}</kbd>
        {:else if item.hint}
          <span class="k-menu__value">{item.hint}</span>
        {/if}
      </button>
    {/if}
  {:else}
    {#if search && empty}<p class="k-menu__empty">{empty}</p>{/if}
  {/each}
</div>
