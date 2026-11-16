<script>
  // Ребро: плавная кривая; пока шаг идёт, по ней бежит «комета». У выбранного ребра — кнопка удаления.
  import { getContext } from 'svelte';
  import { BaseEdge, EdgeLabel, getBezierPath } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { edgeFlow, outTypeOf } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import WfIcon from './WfIcon.svelte';

  let { id, source, target, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected = false, data } = $props();

  const ctx = getContext(WF);
  const geometry = $derived(getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition, curvature: 0.35 }));
  const path = $derived(geometry[0]);
  const flow = $derived(edgeFlow({ source, target, branch: data?.branch }, ctx.steps));
  const branch = $derived(data?.branch);
  const type = $derived.by(() => {
    const n = ctx.model.nodes.find((x) => x.id === source);
    const t = n ? outTypeOf(n.data.kind, n.data.fields) : 'text';
    return t === 'image' || t === 'file' ? t : 'text';
  });
</script>

<BaseEdge {id} {path} class={['wfe', `wfe--${flow}`, `wfe--t-${type}`, selected && 'wfe--sel']} interactionWidth={22} />
{#if flow === 'live'}
  <path d={path} pathLength="100" class="wfe-comet" fill="none" />
{/if}
{#if branch}
  <EdgeLabel x={geometry[1]} y={geometry[2]} class="wfe-branch wfe-branch--{branch}">{branch === 'yes' ? tr('Да') : tr('Нет')}</EdgeLabel>
{/if}
{#if selected}
  <EdgeLabel x={geometry[1]} y={geometry[2]} class="wfe-del">
    <button type="button" class="nodrag nopan" aria-label={tr('Удалить связь')} title={tr('Удалить связь')} onclick={() => ctx.removeEdge(id)}>
      <WfIcon name="x" size={12} />
    </button>
  </EdgeLabel>
{/if}

<style>
  :global(.svelte-flow__edge-path.wfe) {
    stroke: var(--k-line-strong);
    stroke-width: 2.2;
    transition:
      stroke 220ms var(--k-ease-out, ease),
      stroke-width 150ms var(--k-ease-out, ease);
  }
  :global(.svelte-flow__edge-path.wfe--t-image) {
    stroke: color-mix(in oklab, var(--k-bot-rose) 70%, var(--k-line-strong));
  }
  :global(.svelte-flow__edge-path.wfe--t-file) {
    stroke: color-mix(in oklab, var(--k-bot-tangerine) 75%, var(--k-line-strong));
  }
  :global(.svelte-flow__edge-path.wfe--pending) {
    stroke: color-mix(in oklab, var(--k-brand) 55%, var(--k-line-strong));
  }
  :global(.svelte-flow__edge-path.wfe--done) {
    stroke: color-mix(in oklab, var(--k-bot-mint) 80%, var(--k-line-strong));
  }
  :global(.svelte-flow__edge-path.wfe--live) {
    stroke: color-mix(in oklab, var(--k-brand) 60%, var(--k-line-strong));
  }
  :global(.svelte-flow__edge-path.wfe--off) {
    stroke-dasharray: 3 7;
    opacity: 0.45;
  }
  :global(.svelte-flow__edge-path.wfe--sel) {
    stroke: var(--k-brand);
    stroke-width: 3;
  }
  .wfe-comet {
    stroke: var(--k-brand);
    stroke-width: 3.6;
    stroke-linecap: round;
    stroke-dasharray: 14 86;
    filter: drop-shadow(0 0 5px var(--k-brand));
    pointer-events: none;
    animation: wfe-run 1.1s linear infinite;
  }
  @keyframes wfe-run {
    from {
      stroke-dashoffset: 14;
    }
    to {
      stroke-dashoffset: -86;
    }
  }
  :global(.wfe-branch) {
    padding: 2px 9px;
    border-radius: 999px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    font: 700 11px/1.5 var(--k-font-body);
    color: var(--k-text-2);
  }
  :global(.wfe-branch--yes) {
    color: color-mix(in oklab, var(--k-bot-mint) 75%, var(--k-text));
  }
  :global(.wfe-branch--no) {
    color: var(--k-attention-text);
  }
  :global(.wfe-del) {
    background: transparent !important;
  }
  :global(.wfe-del) button {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 1px solid var(--k-line);
    border-radius: 50%;
    background: var(--k-surface);
    color: var(--k-text-2);
    box-shadow: var(--k-shadow-1);
    cursor: pointer;
    animation: wfe-pop 0.34s var(--k-ease-spring, ease) both;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease);
  }
  :global(.wfe-del) button:hover {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
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
