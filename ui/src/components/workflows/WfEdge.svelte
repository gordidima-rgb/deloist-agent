<script>
  // Ребро: плавная кривая цвета лавы с мягким свечением; пока шаг идёт, по ней бегут искры. У выбранного ребра — кнопка удаления.
  import { getContext } from 'svelte';
  import { BaseEdge, EdgeLabel, getBezierPath } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { edgeFlow } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { IconButton } from '../kit';

  let { id, source, target, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected = false, data } = $props();

  const ctx = getContext(WF);
  const geometry = $derived(getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition, curvature: 0.35 }));
  const path = $derived(geometry[0]);
  const flow = $derived(edgeFlow({ source, target, branch: data?.branch }, ctx.steps));
  const branch = $derived(data?.branch);
</script>

<BaseEdge {id} {path} class={['wfe', `wfe--${flow}`, selected && 'wfe--sel']} interactionWidth={22} />
{#if flow === 'live'}
  <path d={path} pathLength="100" class="wfe-comet" fill="none" />
{/if}
{#if branch}
  <EdgeLabel x={geometry[1]} y={geometry[2]} class="wfe-branch wfe-branch--{branch}">{branch === 'yes' ? tr('Да') : tr('Нет')}</EdgeLabel>
{/if}
{#if selected}
  <EdgeLabel x={geometry[1]} y={geometry[2]} class="wfe-del">
    <IconButton class="nodrag nopan" name="close" size="sm" label={tr('Удалить связь')} onclick={() => ctx.removeEdge(id)} />
  </EdgeLabel>
{/if}

<style>
  :global(.svelte-flow__edge-path.wfe) {
    stroke: color-mix(in oklab, var(--k-brand) 78%, var(--k-brand-hover));
    stroke-width: 2.5;
    stroke-linecap: round;
    filter: drop-shadow(0 0 4px color-mix(in oklab, var(--k-brand) 55%, transparent));
    transition:
      stroke var(--k-dur-base) var(--k-ease-out),
      stroke-width var(--k-dur-micro) var(--k-ease-out),
      opacity var(--k-dur-base) var(--k-ease-out);
  }
  :global(.svelte-flow__edge-path.wfe--idle) {
    opacity: 0.55;
  }
  :global(.svelte-flow__edge-path.wfe--done) {
    opacity: 0.85;
  }
  :global(.svelte-flow__edge-path.wfe--off) {
    stroke-dasharray: 3 7;
    opacity: 0.3;
    filter: none;
  }
  :global(.svelte-flow__edge-path.wfe--sel) {
    stroke-width: 3.2;
    opacity: 1;
  }
  .wfe-comet {
    stroke: color-mix(in oklab, var(--k-brand) 45%, var(--k-on-brand));
    stroke-width: 2.8;
    stroke-linecap: round;
    stroke-dasharray: 0.4 6;
    filter: drop-shadow(0 0 4px color-mix(in oklab, var(--k-brand) 90%, transparent));
    pointer-events: none;
    animation: wfe-run 0.9s linear infinite;
  }
  @keyframes wfe-run {
    from {
      stroke-dashoffset: 6.4;
    }
    to {
      stroke-dashoffset: 0;
    }
  }
  :global(.wfe-branch) {
    padding: 2px 9px;
    border-radius: var(--k-radius-pill);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    font: 700 11px / 1.5 var(--k-font-body);
    color: var(--k-text-2);
  }
  :global(.wfe-branch--yes) {
    color: var(--k-brand-text);
  }
  :global(.wfe-del) {
    background: transparent !important;
    animation: wfe-pop var(--k-dur-ui) var(--k-ease-spring) both;
  }
  @keyframes wfe-pop {
    from {
      transform: scale(0.3);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfe-comet {
      animation: none;
      stroke-dasharray: none;
      filter: none;
      opacity: 0.5;
    }
  }
</style>
