<script>
  // Параметры инструмента над нижней панелью: кисть, цвет, толщина (рисование) или фигура и заливка (фигуры).
  import { tr } from '../../lib/i18n.js';
  import { SHAPES, BRUSH_COLORS, ANNOTATION_COLORS } from '../../lib/workflows/graph.js';
  import WfIcon from './WfIcon.svelte';

  /** mode: 'draw' | 'shape'. */
  let { mode, brush = 'pen', color = 'ink', size = 3, shape = 'rect', fill = 'sky', onbrush, oncolor, onsize, onshape, onfill } = $props();

  const SIZES = [2, 4, 8];
  const POLY = {
    diamond: '50,6 94,50 50,94 6,50',
    triangle: '50,8 94,90 6,90',
    star: '50,4 62,36 96,38 70,59 80,92 50,73 20,92 30,59 4,38 38,36',
  };
  const shapeName = (s) =>
    ({ rect: tr('Прямоугольник'), circle: tr('Круг'), diamond: tr('Ромб'), triangle: tr('Треугольник'), oval: tr('Овал'), star: tr('Звезда') })[s];
  const paint = (c) => (c === 'ink' ? 'var(--k-text)' : `var(--k-bot-${c})`);
</script>

<div class="wf-to nodrag nopan" role="group" aria-label={mode === 'draw' ? tr('Параметры рисования') : tr('Параметры фигуры')}>
  {#if mode === 'draw'}
    <div class="wf-to__row">
      {#each [['pen', tr('Ручка (P)'), 'pencil'], ['marker', tr('Маркер'), 'marker'], ['eraser', tr('Ластик (E)'), 'eraser']] as [id, label, icon] (id)}
        <button type="button" class="wf-to__btn" class:on={brush === id} aria-pressed={brush === id} title={label} aria-label={label} onclick={() => onbrush?.(id)}><WfIcon name={icon} size={16} /></button>
      {/each}
      <span class="wf-to__sep"></span>
      {#each SIZES as n (n)}
        <button type="button" class="wf-to__btn" class:on={size === n} aria-pressed={size === n} title={tr('Толщина {n}', { n })} aria-label={tr('Толщина {n}', { n })} disabled={brush === 'eraser'} onclick={() => onsize?.(n)}>
          <span class="wf-to__dot" style:width="{n * 1.6 + 3}px" style:height="{n * 1.6 + 3}px"></span>
        </button>
      {/each}
    </div>
    <div class="wf-to__row wf-to__row--colors">
      {#each BRUSH_COLORS as c (c)}
        <button type="button" class="wf-to__color" class:on={color === c} aria-pressed={color === c} aria-label={c} style:--dot={paint(c)} disabled={brush === 'eraser'} onclick={() => oncolor?.(c)}></button>
      {/each}
    </div>
  {:else}
    <div class="wf-to__row">
      {#each SHAPES as sh (sh)}
        <button type="button" class="wf-to__btn" class:on={shape === sh} aria-pressed={shape === sh} title={shapeName(sh)} aria-label={shapeName(sh)} onclick={() => onshape?.(sh)}>
          <svg viewBox="0 0 100 100" width="18" height="18" fill="none" stroke="currentColor" stroke-width="9" stroke-linejoin="round" aria-hidden="true">
            {#if sh === 'rect'}<rect x="10" y="16" width="80" height="68" rx="12" />
            {:else if sh === 'circle'}<circle cx="50" cy="50" r="40" />
            {:else if sh === 'oval'}<ellipse cx="50" cy="50" rx="44" ry="30" />
            {:else}<polygon points={POLY[sh]} />{/if}
          </svg>
        </button>
      {/each}
    </div>
    <div class="wf-to__row wf-to__row--colors">
      {#each ANNOTATION_COLORS as c (c)}
        <button type="button" class="wf-to__color" class:on={fill === c} aria-pressed={fill === c} aria-label={c} style:--dot="var(--k-bot-{c})" onclick={() => onfill?.(c)}></button>
      {/each}
      <button type="button" class="wf-to__color wf-to__color--none" class:on={fill === 'none'} aria-pressed={fill === 'none'} aria-label={tr('Без заливки')} title={tr('Без заливки')} onclick={() => onfill?.('none')}></button>
    </div>
  {/if}
</div>

<style>
  .wf-to {
    display: grid;
    gap: 6px;
    padding: 8px;
    border-radius: 18px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    animation: wf-to-in 0.34s var(--k-ease-spring, ease) both;
    transform-origin: bottom center;
  }
  .wf-to__row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
  }
  .wf-to__row--colors {
    gap: 8px;
    padding: 2px 4px 2px;
  }
  .wf-to__btn {
    display: grid;
    place-items: center;
    width: 36px;
    height: 34px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-to__btn:hover:not(:disabled) {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .wf-to__btn:active:not(:disabled) {
    transform: scale(0.92);
  }
  .wf-to__btn.on {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wf-to__btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .wf-to__dot {
    border-radius: 50%;
    background: currentColor;
  }
  .wf-to__sep {
    width: 1px;
    height: 20px;
    margin: 0 4px;
    background: var(--k-line);
  }
  .wf-to__color {
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--dot);
    cursor: pointer;
    transition: transform 300ms var(--k-ease-spring, ease);
  }
  .wf-to__color:hover:not(:disabled) {
    transform: scale(1.18);
  }
  .wf-to__color.on {
    box-shadow:
      0 0 0 2px var(--k-surface),
      0 0 0 3.5px var(--dot, var(--k-text-3));
  }
  .wf-to__color:disabled {
    opacity: 0.35;
  }
  .wf-to__color--none {
    --dot: var(--k-text-3);
    background: linear-gradient(135deg, transparent 45%, var(--k-text-3) 46%, var(--k-text-3) 54%, transparent 55%), var(--k-surface-2);
  }
  @keyframes wf-to-in {
    from {
      opacity: 0;
      transform: scale(0.92) translateY(6px);
      filter: blur(5px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-to {
      animation: none;
    }
  }
</style>
