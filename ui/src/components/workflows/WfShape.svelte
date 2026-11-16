<script>
  // Фигура-аннотация: прямоугольник, круг, ромб, треугольник, овал, звезда; цвет заливки и подпись внутри.
  import { getContext } from 'svelte';
  import { NodeResizer } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { WF } from './ctx.js';

  let { id, data, selected = false } = $props();

  const ctx = getContext(WF);
  const shape = $derived(data.fields?.shape || 'rect');
  const fill = $derived(data.fields?.fill || 'sky');
  const hit = $derived(ctx.hits.has(id));

  const POLY = {
    diamond: '50,2 98,50 50,98 2,50',
    triangle: '50,4 97,96 3,96',
    star: '50,3 62,36 97,38 70,59 80,94 50,73 20,94 30,59 3,38 38,36',
  };
</script>

<NodeResizer minWidth={70} minHeight={60} isVisible={selected} color="var(--k-brand)" onResizeEnd={() => ctx.model.commit()} />

<div class="wfs" class:is-selected={selected} class:is-hit={hit} class:is-hollow={fill === 'none'} style:--tone="var(--k-bot-{fill === 'none' ? 'sky' : fill})">
  <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
    {#if shape === 'circle' || shape === 'oval'}
      <ellipse cx="50" cy="50" rx="48" ry="48" />
    {:else if POLY[shape]}
      <polygon points={POLY[shape]} stroke-linejoin="round" />
    {:else}
      <rect x="2" y="2" width="96" height="96" rx="12" />
    {/if}
  </svg>
  <textarea class="wfs-area nodrag nowheel" placeholder={tr('Подпись')} aria-label={tr('Подпись фигуры')} value={data.fields?.text || ''} oninput={(e) => ctx.model.setField(id, 'text', e.currentTarget.value)}></textarea>
</div>

<style>
  .wfs {
    --tone: var(--k-bot-sky);
    position: relative;
    width: 100%;
    height: 100%;
    min-width: 70px;
    min-height: 60px;
    animation: wfs-in 0.5s var(--k-ease-spring, ease) both;
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  svg :global(*) {
    fill: color-mix(in oklab, var(--tone) 34%, var(--k-surface));
    stroke: var(--tone);
    stroke-width: 2.2px;
    vector-effect: non-scaling-stroke;
    transition: fill 220ms var(--k-ease-out, ease);
  }
  .is-hollow svg :global(*) {
    fill: transparent;
    stroke: var(--k-text-3);
  }
  .is-selected svg :global(*) {
    stroke: var(--k-brand);
  }
  .is-hit svg :global(*) {
    stroke: var(--k-bot-lemon);
    stroke-width: 4px;
  }
  .wfs-area {
    position: absolute;
    inset: 18% 14%;
    box-sizing: border-box;
    width: auto;
    border: 0;
    outline: none;
    background: transparent;
    resize: none;
    text-align: center;
    color: var(--k-text);
    font: 600 14px/1.35 var(--k-font-body);
    display: block;
    field-sizing: content;
    max-height: 64%;
    margin: auto 0;
    inset-block: 0;
    height: fit-content;
  }
  .wfs-area::placeholder {
    color: var(--k-text-3);
    font-weight: 400;
  }
  @keyframes wfs-in {
    from {
      opacity: 0;
      transform: scale(0.88);
      filter: blur(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfs {
      animation: none;
    }
  }
</style>
