<script>
  // Меню кита «Тушь и сигнал» (.k-menu, docs/design/kit.md): островок туши со строками.
  // Пункты — данные: { id, label, kind: 'item' | 'check' | 'sep' | 'note', checked, disabled, hint, danger, icon, autofocus }.
  // icon — имя значка из lib/icons.js, hint — тихая подпись справа, disabled — серая строка, tip — её подсказка.
  // Стрелки, Home и End ходят по пунктам; Esc и щелчок мимо закрывают меню (onclose). Текст уже переведён вызывающим.
  import Icon from './kit/Icon.svelte';

  let { items = [], label = '', onpick, onclose } = $props();

  let root = $state();

  const focusables = () => [...(root?.querySelectorAll('.k-menu__item:not([aria-disabled="true"])') ?? [])];

  $effect(() => {
    // Открылось или сменился набор пунктов (например, подтверждение): фокус на первый доступный.
    items;
    queueMicrotask(() => (root?.querySelector('[data-autofocus]') ?? focusables()[0])?.focus({ preventScroll: true }));
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

<div class="k-menu k-sharpen-in menu" role="menu" aria-label={label} tabindex="-1" bind:this={root} onkeydown={onkey}>
  {#each items as item, i (item.id ?? `sep${i}`)}
    {#if item.kind === 'sep'}
      <div class="k-menu__sep" role="separator"></div>
    {:else if item.kind === 'note'}
      <div class="menu__note">{item.label}</div>
    {:else}
      <button
        type="button"
        class="k-menu__item"
        class:is-danger={item.danger}
        class:is-off={item.disabled}
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
        <span class="menu__label">{item.label}</span>
        {#if item.kind === 'check'}<span class="k-switch"></span>{:else if item.hint}<span class="k-menu__value">{item.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    min-width: 0;
    width: 100%;
    outline: none;
  }
  .menu__label {
    flex: 1;
    min-width: 0;
  }
  .menu__note {
    padding: 8px 12px 6px;
    color: var(--k-on-ink-2);
    font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body);
  }
  .k-menu__item.is-danger {
    color: var(--k-attention);
  }
  .k-menu__item.is-danger :global(.k-i) {
    color: var(--k-attention);
  }
  .k-menu__item.is-off {
    color: var(--k-on-ink-3);
    cursor: default;
  }
  .k-menu__item.is-off:hover {
    background: transparent;
  }
</style>
