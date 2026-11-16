<script>
  // Выбор бота для шага: кнопка с аватаром и выпадающий список. Список ботов приносит app.js.
  import { tr } from '../../lib/i18n.js';
  import Avatar from '../kit/Avatar.svelte';
  import WfIcon from './WfIcon.svelte';

  let { bots = [], value = '', onchange } = $props();

  let open = $state(false);
  let root = $state();
  const current = $derived(bots.find((b) => b.id === value) || null);

  function pick(id) {
    open = false;
    onchange?.(id);
  }

  $effect(() => {
    if (!open) return;
    const away = (e) => {
      if (root && !root.contains(e.target)) open = false;
    };
    const esc = (e) => {
      if (e.key === 'Escape') {
        open = false;
        e.stopPropagation();
      }
    };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', esc, true);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', esc, true);
    };
  });
</script>

<div class="wf-bp nodrag nowheel nopan" bind:this={root}>
  <button type="button" class="wf-bp__btn" class:is-empty={!current} aria-haspopup="listbox" aria-expanded={open} onclick={() => (open = !open)}>
    {#if current}
      <Avatar emoji={current.emoji || ''} name={current.name} size={24} ring={current.color || true} />
      <span class="wf-bp__name">{current.name}</span>
    {:else if value}
      <span class="wf-bp__name">{tr('Бот удалён')}</span>
    {:else}
      <span class="wf-bp__name">{tr('Выбрать бота')}</span>
    {/if}
    <WfIcon name="chevron" size={14} />
  </button>
  {#if open}
    <ul class="wf-bp__list" role="listbox">
      {#each bots as b (b.id)}
        <li>
          <button type="button" role="option" aria-selected={b.id === value} class:on={b.id === value} onclick={() => pick(b.id)}>
            <Avatar emoji={b.emoji || ''} name={b.name} size={24} ring={b.color || true} />
            <span class="wf-bp__name">{b.name}</span>
            {#if b.id === value}<WfIcon name="check" size={14} />{/if}
          </button>
        </li>
      {:else}
        <li class="wf-bp__empty">{tr('Ботов пока нет')}</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .wf-bp {
    position: relative;
  }
  .wf-bp__btn {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 38px;
    padding: 0 10px 0 8px;
    border: 1px solid var(--k-line);
    border-radius: 12px;
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 600 13px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      border-color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-bp__btn:hover {
    background: var(--k-surface-3);
  }
  .wf-bp__btn:active {
    transform: scale(0.97);
  }
  .wf-bp__btn.is-empty {
    color: var(--k-text-3);
    border-style: dashed;
  }
  .wf-bp__name {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wf-bp__list {
    position: absolute;
    z-index: 30;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    max-height: 220px;
    overflow-y: auto;
    margin: 0;
    padding: 5px;
    list-style: none;
    border-radius: 14px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    transform-origin: top center;
    animation: wf-pop 0.32s var(--k-ease-spring, ease) both;
  }
  .wf-bp__list button {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 36px;
    padding: 0 8px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--k-text);
    font: 500 13px/1 var(--k-font-body);
    cursor: pointer;
    transition: background 150ms var(--k-ease-out, ease);
  }
  .wf-bp__list button:hover,
  .wf-bp__list button.on {
    background: var(--k-surface-2);
  }
  .wf-bp__empty {
    padding: 10px;
    font: 400 12px/1.3 var(--k-font-body);
    color: var(--k-text-3);
  }
  @keyframes wf-pop {
    from {
      opacity: 0;
      transform: scale(0.94) translateY(-4px);
      filter: blur(4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-bp__list {
      animation: none;
    }
  }
</style>
