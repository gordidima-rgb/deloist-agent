<script>
  // Контекстное меню холста: список пунктов у точки (x, y в координатах холста), не выходит за края.
  import WfIcon from './WfIcon.svelte';

  /** items: [{id, label, icon?, danger?, disabled?, hint?}] или {sep:true}; onpick(id); onclose(). */
  let { items = [], x = 0, y = 0, onpick, onclose } = $props();

  let root = $state();
  let placed = $state(false);

  $effect(() => {
    const parent = root?.parentElement;
    if (!parent) return;
    const w = root.offsetWidth;
    const h = root.offsetHeight;
    root.style.left = `${Math.min(Math.max(8, x), parent.clientWidth - w - 8)}px`;
    root.style.top = `${Math.min(Math.max(8, y), parent.clientHeight - h - 8)}px`;
    placed = true;
  });

  $effect(() => {
    const away = (e) => {
      if (root && !root.contains(e.target)) onclose?.();
    };
    const esc = (e) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onclose?.();
      }
    };
    const t = setTimeout(() => window.addEventListener('pointerdown', away, true), 0);
    window.addEventListener('keydown', esc, true);
    return () => {
      clearTimeout(t);
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', esc, true);
    };
  });
</script>

<div class="wf-cm nodrag nopan nowheel" class:is-placed={placed} bind:this={root} role="menu">
  {#each items as it, i (i)}
    {#if it.sep}
      <div class="wf-cm__sep"></div>
    {:else}
      <button type="button" role="menuitem" class="wf-cm__item" class:is-danger={it.danger} disabled={it.disabled} onclick={() => onpick?.(it.id)}>
        <span class="wf-cm__ico">{#if it.icon}<WfIcon name={it.icon} size={15} />{/if}</span>
        <span class="wf-cm__label">{it.label}</span>
        {#if it.hint}<span class="wf-cm__hint">{it.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .wf-cm {
    position: absolute;
    z-index: 45;
    min-width: 220px;
    padding: 6px;
    border-radius: 16px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
    visibility: hidden;
    transform-origin: top left;
    animation: wf-cm-in 0.3s var(--k-ease-spring, ease) both;
  }
  .wf-cm.is-placed {
    visibility: visible;
  }
  .wf-cm__sep {
    height: 1px;
    margin: 5px 6px;
    background: var(--k-line);
  }
  .wf-cm__item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 34px;
    padding: 0 10px 0 8px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--k-text);
    font: 500 13.5px/1 var(--k-font-body);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-cm__item:hover:not(:disabled) {
    background: var(--k-surface-2);
  }
  .wf-cm__item:active:not(:disabled) {
    transform: scale(0.97);
  }
  .wf-cm__item:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .wf-cm__item.is-danger {
    color: var(--k-attention-text);
  }
  .wf-cm__item.is-danger:hover:not(:disabled) {
    background: var(--k-attention-tint);
  }
  .wf-cm__ico {
    display: grid;
    place-items: center;
    width: 18px;
    color: var(--k-text-2);
  }
  .is-danger .wf-cm__ico {
    color: inherit;
  }
  .wf-cm__label {
    flex: 1;
  }
  .wf-cm__hint {
    font: 500 11.5px/1 var(--k-font-body);
    color: var(--k-text-3);
  }
  @keyframes wf-cm-in {
    from {
      opacity: 0;
      transform: scale(0.92);
      filter: blur(5px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-cm {
      animation: none;
    }
  }
</style>
