<script>
  // One catalog item: icon, title, one-line summary, source, tier and the action button.
  import { tr } from '../../lib/i18n.js';
  import { loc, iconOf } from '../../lib/api.js';
  import { tierLabel, actionLabel } from './Labels.svelte';

  let { item, onopen } = $props();

  const state = $derived(item.state || {});
  const icon = $derived(iconOf(item));
  const title = $derived(loc(item.title));
  const done = $derived(state.installed && !state.update);
</script>

<div class="cat-card k-card" class:done>
  <button type="button" class="body" onclick={() => onopen?.(item.id)} aria-label={tr('Подробнее: {title}', { title })}>
    <span class="icon" aria-hidden="true">{icon || title.slice(0, 1)}</span>
    <span class="text">
      <span class="title">{title}</span>
      <span class="summary">{loc(item.summary)}</span>
      <span class="meta">
        <span class="source">{item.source?.name}</span>
        <span class="tier" class:verified={item.tier === 'verified'}>{tierLabel(item.tier)}</span>
        {#if item.mode === 'cabinet'}<span class="tier">{tr('Кабинет')}</span>{/if}
      </span>
    </span>
  </button>
  <button
    type="button"
    class="k-btn k-btn--sm action"
    class:k-btn--primary={state.update}
    class:k-btn--ghost={done}
    onclick={() => onopen?.(item.id)}>{actionLabel(item)}</button
  >
</div>

<style>
  .cat-card {
    display: flex;
    align-items: center;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-3, 12px) var(--k-space-4, 16px) var(--k-space-3, 12px) var(--k-space-3, 12px);
    min-width: 0;
    transition: box-shadow var(--k-dur-fast, 140ms) var(--k-ease-out, ease-out);
  }
  .cat-card:hover {
    box-shadow: var(--k-shadow-2, 0 4px 12px rgba(0, 0, 0, 0.1));
  }
  .body {
    display: flex;
    align-items: center;
    gap: var(--k-space-3, 12px);
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--k-radius-sm, 12px);
  }
  .body:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  .icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface-2, #f0f0f0);
    font-size: 22px;
    line-height: 1;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .title {
    font: 800 15px / 1.25 var(--k-font-display, inherit);
    color: var(--k-text, inherit);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .summary {
    color: var(--k-text-2, inherit);
    font-size: var(--k-text-sm, 13px);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    color: var(--k-text-3, inherit);
    font-size: var(--k-text-xs, 12px);
  }
  .source {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 160px;
  }
  .tier {
    padding: 1px 6px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface-2, #f0f0f0);
    color: var(--k-text-2, inherit);
    font-weight: 600;
  }
  .tier.verified {
    background: var(--k-brand-tint, #e7efff);
    color: var(--k-brand-text, #0250e0);
  }
  .action {
    flex: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .cat-card {
      transition: none;
    }
  }
</style>
