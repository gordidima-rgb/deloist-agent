<script>
  // One catalog item as a kit row: icon, title (opens the card), one-line summary, source and tier tags, the action button.
  // Lives inside <ul class="k-lrows"> (see Catalog.svelte).
  import { tr } from '../../lib/i18n.js';
  import { loc, iconOf } from '../../lib/api.js';
  import { ListRow, Tag } from '../kit';
  import { tierLabel, actionLabel } from './Labels.svelte';

  let { item, onopen } = $props();

  const state = $derived(item.state || {});
  const icon = $derived(iconOf(item));
  const title = $derived(loc(item.title));
  const done = $derived(state.installed && !state.update);
</script>

<ListRow
  {title}
  variant="action"
  actionLabel={actionLabel(item)}
  actionVariant={state.update ? 'primary' : done ? 'ghost' : 'secondary'}
  onaction={() => onopen?.(item.id)}
  onclick={() => onopen?.(item.id)}
  class="cat-card"
  aria-label={tr('Подробнее: {title}', { title })}
>
  {#snippet leading()}
    <span class="cat-ico" aria-hidden="true">{icon || title.slice(0, 1)}</span>
  {/snippet}
  {#snippet meta()}
    <small class="k-lrow__sub cat-summary">{loc(item.summary)}</small>
    <span class="cat-meta">
      {#if item.source?.name}<span class="cat-source">{item.source.name}</span>{/if}
      <Tag tone={item.tier === 'verified' ? 'accent' : 'neutral'}>{tierLabel(item.tier)}</Tag>
      {#if item.mode === 'cabinet'}<Tag>{tr('Кабинет')}</Tag>{/if}
    </span>
  {/snippet}
</ListRow>

<style>
  .cat-ico {
    flex: none;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: var(--k-radius-sm);
    background: var(--k-surface-2);
    font-size: var(--k-text-lg);
    line-height: 1;
  }
  .cat-summary {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .cat-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
    align-items: center;
    color: var(--k-text-3);
    font-size: var(--k-text-xs);
  }
  .cat-source {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 160px;
  }
</style>
