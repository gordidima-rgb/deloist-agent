<script>
  // Плавающая панель параметров над нижней: фигуры (цвет и семь фигур), рисование (кисть, цвет), заметка (семь цветов).
  import { tr } from '../../lib/i18n.js';
  import { SHAPE_CHOICES, BRUSH_COLORS, ANNOTATION_COLORS, NOTE_COLORS } from '../../lib/workflows/graph.js';
  import WfIcon from './WfIcon.svelte';

  /** mode: 'draw' | 'shape' | 'note'. shapeId — id из SHAPE_CHOICES. */
  let { mode, brush = 'pen', color = 'ink', size = 3, shapeId = 'rect', fill = 'sky', noteColor = 'lemon', onbrush, oncolor, onsize, onshape, onfill, onnote } = $props();

  let palette = $state(false);
  const SIZES = [2, 4, 8];
  const POLY = {
    diamond: '50,6 94,50 50,94 6,50',
    triangle: '50,10 94,90 6,90',
    triangle_down: '6,10 94,10 50,90',
    star: '50,4 62,36 96,38 70,59 80,92 50,73 20,92 30,59 4,38 38,36',
  };
  const NAMES = () => ({
    rect: tr('Прямоугольник'),
    circle: tr('Круг'),
    diamond: tr('Ромб'),
    triangle: tr('Треугольник'),
    triangle_down: tr('Перевёрнутый треугольник'),
    oval: tr('Овал'),
    star: tr('Звезда'),
  });
  const paint = (c) => (c === 'ink' ? 'var(--k-text)' : c === 'none' ? 'transparent' : `var(--k-bot-${c})`);
  const palColors = $derived(mode === 'shape' ? [...ANNOTATION_COLORS, 'none'] : BRUSH_COLORS);
  const current = $derived(mode === 'shape' ? fill : color);

  function choose(c) {
    palette = false;
    if (mode === 'shape') onfill?.(c);
    else oncolor?.(c);
  }
</script>

<div class="wf-to nodrag nopan" role="group" aria-label={mode === 'draw' ? tr('Параметры рисования') : mode === 'shape' ? tr('Параметры фигуры') : tr('Цвет заметки')}>
  {#if mode === 'note'}
    <div class="wf-to__row wf-to__row--notes">
      {#each NOTE_COLORS as c (c)}
        <button type="button" class="wf-to__swatch" class:on={noteColor === c} aria-pressed={noteColor === c} aria-label={c} style:--dot="var(--k-bot-{c})" onclick={() => onnote?.(c)}></button>
      {/each}
    </div>
  {:else}
    <div class="wf-to__row">
      <div class="wf-to__col">
        <button type="button" class="wf-to__color" aria-haspopup="true" aria-expanded={palette} title={tr('Цвет')} aria-label={tr('Цвет')} disabled={mode === 'draw' && brush === 'eraser'} onclick={() => (palette = !palette)}>
          <span class="wf-to__dot" class:is-none={current === 'none'} style:background={paint(current)}></span>
          <WfIcon name="chevron" size={13} />
        </button>
        {#if palette}
          <div class="wf-to__pal" role="listbox" aria-label={tr('Цвет')}>
            {#each palColors as c (c)}
              <button type="button" class="wf-to__swatch" class:on={current === c} class:is-none={c === 'none'} aria-label={c === 'none' ? tr('Без заливки') : c} style:--dot={paint(c)} onclick={() => choose(c)}></button>
            {/each}
          </div>
        {/if}
      </div>
      <span class="wf-to__sep"></span>
      {#if mode === 'draw'}
        {#each [['pen', tr('Ручка (P)'), 'pencil'], ['marker', tr('Маркер'), 'marker'], ['eraser', tr('Ластик (E)'), 'eraser']] as [id, label, icon] (id)}
          <button type="button" class="wf-to__btn" class:on={brush === id} aria-pressed={brush === id} title={label} aria-label={label} onclick={() => onbrush?.(id)}><WfIcon name={icon} size={17} /></button>
        {/each}
        <span class="wf-to__sep"></span>
        {#each SIZES as n (n)}
          <button type="button" class="wf-to__btn wf-to__btn--size" class:on={size === n} aria-pressed={size === n} title={tr('Толщина {n}', { n })} aria-label={tr('Толщина {n}', { n })} disabled={brush === 'eraser'} onclick={() => onsize?.(n)}>
            <span class="wf-to__pt" style:width="{n * 1.6 + 3}px" style:height="{n * 1.6 + 3}px"></span>
          </button>
        {/each}
      {:else}
        {#each SHAPE_CHOICES as sh (sh.id)}
          <button type="button" class="wf-to__btn" class:on={shapeId === sh.id} aria-pressed={shapeId === sh.id} title={NAMES()[sh.id]} aria-label={NAMES()[sh.id]} onclick={() => onshape?.(sh.id)}>
            <svg viewBox="0 0 100 100" width="19" height="19" fill="none" stroke="currentColor" stroke-width="9" stroke-linejoin="round" aria-hidden="true">
              {#if sh.id === 'rect'}<rect x="10" y="16" width="80" height="68" rx="12" />
              {:else if sh.id === 'circle'}<circle cx="50" cy="50" r="40" />
              {:else if sh.id === 'oval'}<ellipse cx="50" cy="50" rx="44" ry="30" />
              {:else}<polygon points={POLY[sh.id]} />{/if}
            </svg>
          </button>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .wf-to {
    padding: 7px;
    border-radius: 20px;
    border: 1px solid var(--k-line);
    background: color-mix(in oklab, var(--k-surface) 94%, transparent);
    backdrop-filter: blur(14px);
    box-shadow: var(--k-shadow-3);
    animation: wf-to-in 0.38s var(--k-ease-spring, ease) both;
    transform-origin: bottom center;
  }
  .wf-to__row {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .wf-to__row--notes {
    gap: 9px;
    padding: 5px 8px;
  }
  .wf-to__col {
    position: relative;
  }
  .wf-to__color {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 38px;
    padding: 0 8px 0 10px;
    border: 0;
    border-radius: 13px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-to__color:hover:not(:disabled) {
    background: var(--k-surface-2);
  }
  .wf-to__color:active:not(:disabled) {
    transform: scale(0.94);
  }
  .wf-to__color:disabled {
    opacity: 0.35;
  }
  .wf-to__dot {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--k-text) 18%, transparent);
  }
  .wf-to__dot.is-none {
    background: linear-gradient(135deg, transparent 45%, var(--k-text-3) 46%, var(--k-text-3) 54%, transparent 55%), var(--k-surface-2) !important;
  }
  .wf-to__pal {
    position: absolute;
    left: -2px;
    bottom: calc(100% + 12px);
    display: flex;
    gap: 9px;
    padding: 9px 11px;
    border-radius: 16px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
    transform-origin: bottom left;
    animation: wf-to-in 0.3s var(--k-ease-spring, ease) both;
  }
  .wf-to__swatch {
    flex: none;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--dot);
    cursor: pointer;
    transition: transform 300ms var(--k-ease-spring, ease);
  }
  .wf-to__swatch:hover {
    transform: scale(1.2);
  }
  .wf-to__swatch.on {
    box-shadow:
      0 0 0 2px var(--k-surface),
      0 0 0 3.5px color-mix(in oklab, var(--k-text) 60%, var(--dot));
  }
  .wf-to__swatch.is-none {
    background: linear-gradient(135deg, transparent 45%, var(--k-text-3) 46%, var(--k-text-3) 54%, transparent 55%), var(--k-surface-2);
  }
  .wf-to__btn {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    padding: 0;
    border: 0;
    border-radius: 12px;
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
    transform: translateY(-1px);
  }
  .wf-to__btn:active:not(:disabled) {
    transform: scale(0.92);
  }
  /* Выбранный: залитая плитка. */
  .wf-to__btn.on {
    background: var(--k-text);
    color: var(--k-canvas);
  }
  .wf-to__btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .wf-to__pt {
    border-radius: 50%;
    background: currentColor;
  }
  .wf-to__sep {
    width: 1px;
    height: 22px;
    margin: 0 5px;
    background: var(--k-line);
  }
  @keyframes wf-to-in {
    from {
      opacity: 0;
      transform: scale(0.9) translateY(8px);
      filter: blur(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-to,
    .wf-to__pal {
      animation: none;
    }
  }
</style>
