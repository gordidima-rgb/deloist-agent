<script>
  // Меню «Добавить узел»: группы Входы / Шаги / Поток. Узлы этапа 2 видны, но неактивны («скоро»).
  // allowed — список видов, допустимых для линии, которую бросили на пустом месте (null: любые).
  import { tr } from '../../lib/i18n.js';
  import { KINDS, isEnabled } from '../../lib/workflows/graph.js';
  import { kindTitle, kindHint, groupTitle } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';

  let { allowed = null, x = 0, y = 0, origin = 'bottom', onpick, onclose } = $props();

  const GROUPS = ['inputs', 'steps', 'flow'];
  const groups = $derived(
    GROUPS.map((g) => ({
      id: g,
      kinds: Object.keys(KINDS).filter((k) => KINDS[k].group === g && (!allowed || allowed.includes(k) || !isEnabled(k))),
    })).filter((g) => g.kinds.length),
  );

  let root = $state();

  let placed = $state(false);

  // Меню не выходит за холст: меряем настоящий размер (без масштаба анимации) и ставим внутрь.
  // origin «bottom»: (x, y) — нижний край меню; «top»: верхний левый угол.
  $effect(() => {
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
    // Первый щелчок, открывший меню, уже прошёл: слушаем со следующего кадра.
    const t = setTimeout(() => window.addEventListener('pointerdown', away, true), 0);
    window.addEventListener('keydown', esc, true);
    return () => {
      clearTimeout(t);
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', esc, true);
    };
  });
</script>

<div class="wf-am nodrag nopan nowheel" class:is-top={origin === 'top'} class:is-placed={placed} bind:this={root} role="menu" aria-label={tr('Добавить узел')}>
  {#each groups as g (g.id)}
    <div class="wf-am__group">{groupTitle(g.id)}</div>
    {#each g.kinds as k (k)}
      {@const on = isEnabled(k)}
      <button type="button" role="menuitem" class="wf-am__item" disabled={!on} style:--tone="var(--k-bot-{KINDS[k].tone})" onclick={() => onpick?.(k)}>
        <span class="wf-am__ico"><WfIcon name={k} size={16} /></span>
        <span class="wf-am__text">
          <b>{kindTitle(k)}</b>
          <small>{kindHint(k)}</small>
        </span>
        {#if !on}<span class="wf-am__soon">{tr('скоро')}</span>{/if}
      </button>
    {/each}
  {/each}
</div>

<style>
  .wf-am {
    position: absolute;
    z-index: 40;
    width: 290px;
    max-height: min(460px, 70vh);
    overflow-y: auto;
    padding: 8px;
    border-radius: 20px;
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
  .wf-am__group {
    padding: 8px 10px 4px;
    font: 700 11px/1 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--k-text-3);
  }
  .wf-am__item {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    padding: 7px 9px;
    border: 0;
    border-radius: 13px;
    background: transparent;
    color: var(--k-text);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-am__item:hover:not(:disabled) {
    background: var(--k-surface-2);
  }
  .wf-am__item:active:not(:disabled) {
    transform: scale(0.97);
  }
  .wf-am__item:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .wf-am__ico {
    display: grid;
    place-items: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: 11px;
    background: color-mix(in oklab, var(--tone) 24%, var(--k-surface));
    color: color-mix(in oklab, var(--tone) 70%, var(--k-text));
  }
  .wf-am__text {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .wf-am__text b {
    font: 700 13.5px/1.2 var(--k-font-body);
  }
  .wf-am__text small {
    font: 400 12px/1.3 var(--k-font-body);
    color: var(--k-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wf-am__soon {
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font: 600 11px/1.4 var(--k-font-body);
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
  }
</style>
