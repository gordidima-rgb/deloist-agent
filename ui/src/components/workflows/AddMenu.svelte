<script>
  // Меню «Добавить узел»: поиск сверху, группы Входы / Шаги / Поток, цветные значки видов, стрелки и Enter.
  // allowed — список видов, допустимых для линии, которую бросили на пустом месте (null: любые).
  import { tr } from '../../lib/i18n.js';
  import { KINDS, isEnabled } from '../../lib/workflows/graph.js';
  import { kindTitle, kindHint, groupTitle } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';

  let { allowed = null, x = 0, y = 0, origin = 'bottom', onpick, onclose } = $props();

  const GROUPS = ['inputs', 'steps', 'flow'];
  let query = $state('');
  let active = $state(0);
  let root = $state();
  let input = $state();
  let placed = $state(false);

  const groups = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return GROUPS.map((g) => ({
      id: g,
      kinds: Object.keys(KINDS).filter((k) => {
        if (KINDS[k].group !== g || !isEnabled(k)) return false;
        if (allowed && !allowed.includes(k)) return false;
        return !q || `${kindTitle(k)} ${kindHint(k)} ${k}`.toLowerCase().includes(q);
      }),
    })).filter((g) => g.kinds.length);
  });
  const flat = $derived(groups.flatMap((g) => g.kinds));

  $effect(() => {
    // Новый запрос: подсветка снова на первом.
    query;
    active = 0;
  });

  // Меню не выходит за холст: меряем настоящий размер (без масштаба анимации) и ставим внутрь.
  // origin «bottom»: (x, y) — нижний край меню; «top»: верхний левый угол.
  $effect(() => {
    flat.length;
    const parent = root?.parentElement;
    if (!parent) return;
    const w = root.offsetWidth;
    const h = root.offsetHeight;
    const left = Math.min(Math.max(8, x), parent.clientWidth - w - 8);
    const top = Math.min(Math.max(8, origin === 'bottom' ? y - h : y), parent.clientHeight - h - 8);
    root.style.left = `${left}px`;
    root.style.top = `${top}px`;
    placed = true;
  });

  // Скрытый элемент не берёт фокус: ждём, пока меню расставлено.
  $effect(() => {
    if (!placed) return;
    const t = setTimeout(() => input?.focus(), 30);
    return () => clearTimeout(t);
  });

  $effect(() => {
    const away = (e) => {
      if (root && !root.contains(e.target)) onclose?.();
    };
    const t = setTimeout(() => window.addEventListener('pointerdown', away, true), 0);
    return () => {
      clearTimeout(t);
      window.removeEventListener('pointerdown', away, true);
    };
  });

  function onKey(e) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose?.();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (flat.length) active = (active + 1) % flat.length;
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (flat.length) active = (active - 1 + flat.length) % flat.length;
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (flat[active]) onpick?.(flat[active]);
    }
  }

  $effect(() => {
    root?.querySelector('.is-active')?.scrollIntoView({ block: 'nearest' });
  });
</script>

<div class="wf-am nodrag nopan nowheel" class:is-top={origin === 'top'} class:is-placed={placed} bind:this={root} role="menu" aria-label={tr('Добавить узел')} tabindex="-1" onkeydown={onKey}>
  <label class="wf-am__search">
    <WfIcon name="search" size={15} />
    <input bind:this={input} bind:value={query} type="text" placeholder={tr('Искать узлы')} aria-label={tr('Искать узлы')} autocomplete="off" />
  </label>
  <div class="wf-am__list">
    {#each groups as g (g.id)}
      <div class="wf-am__group">{groupTitle(g.id)}</div>
      {#each g.kinds as k (k)}
        <button
          type="button"
          role="menuitem"
          class="wf-am__item"
          class:is-active={flat[active] === k}
          onpointermove={() => (active = flat.indexOf(k))}
          onclick={() => onpick?.(k)}
        >
          <span class="wf-am__ico"><WfIcon name={k} size={17} /></span>
          <span class="wf-am__text">
            <b>{kindTitle(k)}</b>
            <small>{kindHint(k)}</small>
          </span>
        </button>
      {/each}
    {:else}
      <p class="wf-am__none">{tr('Ничего не нашлось')}</p>
    {/each}
  </div>
</div>

<style>
  .wf-am {
    position: absolute;
    z-index: 40;
    width: 300px;
    max-height: min(520px, 76vh);
    display: flex;
    flex-direction: column;
    border-radius: 22px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
    visibility: hidden;
    transform-origin: bottom center;
    animation: wf-am-in 0.38s var(--k-ease-spring, ease) both;
  }
  .wf-am.is-placed {
    visibility: visible;
  }
  .wf-am.is-top {
    transform-origin: top left;
  }
  .wf-am__search {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 8px 8px 2px;
    height: 40px;
    padding: 0 12px;
    border-radius: 14px;
    background: var(--k-surface-2);
    color: var(--k-text-3);
    transition: box-shadow 150ms var(--k-ease-out, ease);
  }
  .wf-am__search:focus-within {
    box-shadow: var(--k-focus);
  }
  .wf-am__search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--k-text);
    font: 500 14px/1 var(--k-font-body);
  }
  .wf-am__list {
    overflow-y: auto;
    padding: 4px 8px 8px;
  }
  .wf-am__group {
    padding: 10px 10px 4px;
    font: 700 11px/1 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--k-text-3);
  }
  .wf-am__item {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 7px 9px;
    border: 0;
    border-radius: 14px;
    background: transparent;
    color: var(--k-text);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-am__item.is-active {
    background: var(--k-surface-2);
  }
  .wf-am__item:active {
    transform: scale(0.97);
  }
  .wf-am__ico {
    display: grid;
    place-items: center;
    flex: none;
    width: 34px;
    height: 34px;
    border-radius: 12px;
    background: var(--k-brand-tint);
    color: var(--k-brand);
    transition: transform 320ms var(--k-ease-spring, ease);
  }
  .is-active .wf-am__ico {
    transform: scale(1.08) rotate(-4deg);
  }
  .wf-am__text {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .wf-am__text b {
    font: 700 14px/1.2 var(--k-font-body);
  }
  .wf-am__text small {
    font: 400 12px/1.3 var(--k-font-body);
    color: var(--k-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wf-am__none {
    margin: 14px 10px;
    font: 400 13px/1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  @keyframes wf-am-in {
    from {
      opacity: 0;
      transform: scale(0.9) translateY(8px);
      filter: blur(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-am {
      animation: none;
    }
    .wf-am__ico {
      transition: none;
    }
  }
</style>
