<script>
  // Вкладки на .k-tabs: чёрная капсула переезжает к выбранной вкладке.
  // Стрелки, Home и End переключают без анимации (как требует docs/design/kit.md).
  import { tick } from 'svelte';
  import Icon from '../media/Icon.svelte';

  let { items = [], value = $bindable(''), label = '', class: className = '', onchange, ...rest } = $props();

  let root = $state();
  let thumb = $state();
  let instant = $state(false);

  const current = $derived(value || items[0]?.id || '');
  const tabs = () => [...(root?.querySelectorAll('.k-tab') ?? [])];

  function layout() {
    if (!root || !thumb) return;
    const index = items.findIndex((item) => item.id === current);
    const button = tabs()[index];
    if (!button) return;
    thumb.style.width = `${button.offsetWidth}px`;
    thumb.style.transform = `translateX(${button.offsetLeft}px)`;
  }

  function pick(id) {
    if (id === current) return;
    value = id;
    onchange?.(id);
  }

  function keydown(event) {
    const list = items ?? [];
    if (!list.length) return;
    const at = Math.max(0, list.findIndex((item) => item.id === current));
    let next = -1;
    if (event.key === 'ArrowRight') next = (at + 1) % list.length;
    else if (event.key === 'ArrowLeft') next = (at - 1 + list.length) % list.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = list.length - 1;
    else return;
    event.preventDefault();
    instant = true;
    pick(list[next].id);
    tick().then(() => tabs()[next]?.focus({ preventScroll: true }));
  }

  $effect(() => {
    value;
    items;
    tick().then(layout);
  });

  $effect(() => {
    if (!root || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => layout());
    observer.observe(root);
    return () => observer.disconnect();
  });
</script>

<div
  class="k-tabs {className}"
  class:is-instant={instant}
  role="tablist"
  aria-label={label || undefined}
  bind:this={root}
  onkeydown={keydown}
  onpointerdown={() => (instant = false)}
  {...rest}
>
  <span class="k-tabs__thumb" bind:this={thumb} aria-hidden="true"></span>
  {#each items as item, i (item.id)}
    <button
      type="button"
      class="k-tab"
      role="tab"
      aria-selected={current === item.id}
      aria-controls={item.panelId || undefined}
      tabindex={current === item.id ? 0 : -1}
      data-id={item.id}
      onclick={() => pick(item.id)}
      aria-label={item.icon ? item.label : undefined}
      title={item.icon ? item.label : undefined}
    >
      {#if item.icon}
        <Icon name={item.icon} size={15} class="k-tab__icon" />
        <span class="k-tab__label">{item.label}</span>
      {:else}
        {item.label}
      {/if}
    </button>
  {/each}
</div>
