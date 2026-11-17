<script>
  // Аннотации холста: цветной стикер и свободный текст. В запуске не участвуют, портов нет.
  import { getContext } from 'svelte';
  import { NodeResizer } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { ANNOTATION_COLORS } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { kindTitle } from './Labels.svelte';

  let { id, data, selected = false } = $props();

  const ctx = getContext(WF);
  const isNote = $derived(data.kind === 'note');
  const color = $derived(data.fields?.color || 'lemon');
  const hit = $derived(ctx.hits.has(id));
</script>

{#if isNote}
  <NodeResizer minWidth={150} minHeight={110} isVisible={selected} color="var(--k-brand)" onResizeEnd={() => ctx.model.commit()} />
{/if}

<div class="wfa wfa--{data.kind}" class:is-selected={selected} class:is-hit={hit} style:--tone="var(--k-bot-{color})">
  {#if isNote && selected}
    <div class="wfa-colors nodrag" role="radiogroup" aria-label={tr('Цвет заметки')}>
      {#each ANNOTATION_COLORS as c (c)}
        <button
          type="button"
          role="radio"
          aria-checked={c === color}
          aria-label={c}
          class:on={c === color}
          style:--dot="var(--k-bot-{c})"
          onclick={() => ctx.model.setFields(id, { color: c })}
        ></button>
      {/each}
    </div>
  {/if}
  <textarea
    class="wfa-area nodrag nowheel"
    placeholder={isNote ? tr('Заметка') : tr('Текст')}
    aria-label={kindTitle(data.kind)}
    value={data.fields?.text || ''}
    oninput={(e) => ctx.model.setField(id, 'text', e.currentTarget.value)}
  ></textarea>
</div>

<style>
  .wfa {
    --tone: var(--k-bot-lemon);
    position: relative;
    width: 100%;
    height: 100%;
    animation: wfa-in 0.5s var(--k-ease-spring, ease) both;
  }
  .wfa--note {
    min-width: 150px;
    min-height: 110px;
    box-sizing: border-box;
    padding: var(--k-space-4) var(--k-space-4);
    border-radius: var(--k-radius-md);
    background: color-mix(in oklab, var(--tone) 34%, var(--k-surface));
    box-shadow:
      0 1px 2px color-mix(in oklab, var(--k-ink) 8%, transparent),
      0 14px 26px -16px color-mix(in oklab, var(--k-ink) 35%, transparent);
    transition:
      box-shadow 220ms var(--k-ease-out, ease),
      background 220ms var(--k-ease-out, ease);
  }
  .wfa--note.is-selected {
    box-shadow:
      0 0 0 2px var(--k-brand),
      0 18px 30px -16px color-mix(in oklab, var(--k-ink) 40%, transparent);
  }
  .wfa--text {
    min-width: 120px;
    padding: 4px 6px;
    border-radius: var(--k-radius-sm);
  }
  .wfa--text.is-selected {
    outline: 2px dashed var(--k-brand);
    outline-offset: 2px;
  }
  .wfa.is-hit {
    box-shadow: 0 0 0 4px color-mix(in oklab, var(--k-bot-lemon) 55%, transparent);
  }
  .wfa-area {
    display: block;
    box-sizing: border-box;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    outline: none;
    background: transparent;
    resize: none;
    color: var(--k-text);
    font: 600 var(--k-text-md)/1.5 var(--k-font-body);
  }
  .wfa--note .wfa-area {
    color: color-mix(in oklab, var(--k-text) 92%, var(--tone));
  }
  .wfa--text .wfa-area {
    field-sizing: content;
    min-height: 1.5em;
    font: 800 var(--k-text-xl)/1.3 var(--k-font-display);
    letter-spacing: -0.01em;
  }
  .wfa-area::placeholder {
    color: var(--k-text-3);
  }
  .wfa-colors {
    position: absolute;
    left: 50%;
    bottom: calc(100% + 14px);
    display: flex;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--k-radius-pill);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    translate: -50% 0;
    animation: wfa-pop 0.34s var(--k-ease-spring, ease) both;
  }
  .wfa-colors button {
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--dot);
    cursor: pointer;
    transition: transform 300ms var(--k-ease-spring, ease);
  }
  .wfa-colors button:hover {
    transform: scale(1.2);
  }
  .wfa-colors button.on {
    box-shadow:
      0 0 0 2px var(--k-surface),
      0 0 0 3.5px var(--dot);
  }
  @keyframes wfa-in {
    from {
      opacity: 0;
      transform: scale(0.88) translateY(8px);
      filter: blur(6px);
    }
  }
  @keyframes wfa-pop {
    from {
      opacity: 0;
      transform: scale(0.85) translateY(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfa,
    .wfa-colors {
      animation: none;
    }
  }
</style>
