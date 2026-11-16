<script>
  // Рисование: штрихи из графа в координатах потока и сам инструмент (ручка, маркер, ластик).
  // Рисует через ViewportPortal, поэтому штрихи двигаются и масштабируются вместе с холстом.
  import { ViewportPortal, useSvelteFlow } from '@xyflow/svelte';
  import { newId } from '../../lib/workflows/graph.js';
  import { pathOf, thin, hitsStroke } from '../../lib/workflows/strokes.js';

  /** strokes: [{id,brush,color,width,points}]; tool: 'pen'|'marker'|'eraser'|null; color, width: параметры кисти. */
  let { strokes = [], tool = null, color = 'ink', width = 3, zoom = 1, onadd, onerase } = $props();

  const flow = useSvelteFlow();
  let live = $state.raw(null);
  let erased = $state.raw(new Set());
  let down = false;

  const paint = (c) => (c === 'ink' ? 'var(--k-text)' : `var(--k-bot-${c}, ${c})`);
  const active = $derived(tool === 'pen' || tool === 'marker' || tool === 'eraser');
  const eraseRadius = $derived(10 / Math.max(0.25, zoom));

  const at = (e) => flow.screenToFlowPosition({ x: e.clientX, y: e.clientY });

  function eraseAt(p) {
    const next = new Set(erased);
    for (const s of strokes) if (!next.has(s.id) && hitsStroke(s, p.x, p.y, eraseRadius)) next.add(s.id);
    if (next.size !== erased.size) erased = next;
  }

  function onDown(e) {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    down = true;
    const p = at(e);
    if (tool === 'eraser') {
      erased = new Set();
      eraseAt(p);
    } else {
      live = { id: newId('s'), brush: tool, color, width: tool === 'marker' ? Math.max(width, 10) : width, points: [[p.x, p.y]] };
    }
  }

  function onMove(e) {
    if (!down) return;
    const p = at(e);
    if (tool === 'eraser') eraseAt(p);
    else if (live) live = { ...live, points: [...live.points, [p.x, p.y]] };
  }

  function onUp() {
    if (!down) return;
    down = false;
    if (tool === 'eraser') {
      if (erased.size) onerase?.([...erased]);
      erased = new Set();
    } else if (live) {
      const done = { ...live, points: thin(live.points, 1.5 / Math.max(0.25, zoom)) };
      live = null;
      if (done.points.length) onadd?.(done);
    }
  }
</script>

<ViewportPortal target="front">
  <svg class="wf-strokes" width="1" height="1" aria-hidden="true">
    {#each strokes as s (s.id)}
      <path class="wf-stroke wf-stroke--{s.brush}" class:is-erasing={erased.has(s.id)} d={pathOf(s.points)} stroke={paint(s.color)} stroke-width={s.width} />
    {/each}
    {#if live}
      <path class="wf-stroke wf-stroke--{live.brush}" d={pathOf(live.points)} stroke={paint(live.color)} stroke-width={live.width} />
    {/if}
  </svg>
</ViewportPortal>

{#if active}
  <div class="wf-draw" class:is-eraser={tool === 'eraser'} role="presentation" onpointerdown={onDown} onpointermove={onMove} onpointerup={onUp} onpointercancel={onUp}></div>
{/if}

<style>
  .wf-strokes {
    position: absolute;
    left: 0;
    top: 0;
    overflow: visible;
    pointer-events: none;
  }
  .wf-stroke {
    fill: none;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: opacity 120ms ease;
  }
  .wf-stroke--marker {
    opacity: 0.45;
    mix-blend-mode: multiply;
  }
  :global(:root[data-theme='dark']) .wf-stroke--marker,
  :global(:root:not([data-theme='light'])) .wf-stroke--marker {
    mix-blend-mode: screen;
  }
  .wf-stroke.is-erasing {
    opacity: 0.15;
  }
  .wf-draw {
    position: absolute;
    inset: 0;
    z-index: 5;
    cursor: crosshair;
    touch-action: none;
  }
  .wf-draw.is-eraser {
    cursor: cell;
  }
</style>
