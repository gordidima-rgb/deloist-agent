<script>
  // Catalog page: skills, agents or apps. One fetch per page, filtering on the client.
  // DOM stays small with any number of items: a category section shows SECTION cards,
  // a full list shows PAGE at a time with "Show more". No timers or animations at rest.
  import { tr } from '../../lib/i18n.js';
  import { api, loc, serverText, CATEGORIES } from '../../lib/api.js';
  import { categoryLabel, pageTitle, pageLead } from './Labels.svelte';
  import CatalogCard from './CatalogCard.svelte';
  import CatalogDrawer from './CatalogDrawer.svelte';
  import YourSkills from './YourSkills.svelte';

  /**
   * page: 'skills' | 'agents' | 'apps'; bots: [{id, name, color, emoji, skills}];
   * skills: the person's skills from /api/state; stamp: bump to refetch;
   * onpage(page), onopenbot(id) — callbacks to the old app shell.
   */
  let { page = 'skills', bots = [], skills = [], stamp = 0, onpage, onopenbot } = $props();

  const KIND = { skills: 'skill', agents: 'agent', apps: 'app' };
  const PAGES = ['skills', 'agents', 'apps'];
  const SECTION = 6;
  const PAGE = 48;

  let items = $state.raw([]);
  let loading = $state(true);
  let error = $state('');
  let query = $state('');
  let category = $state('all');
  let tier = $state('all');
  let limit = $state(PAGE);
  let openId = $state('');
  let tick = $state(0);
  let lastKind = '';

  const kind = $derived(KIND[page] || 'skill');

  $effect(() => {
    const wanted = kind;
    stamp;
    tick;
    if (wanted !== lastKind) {
      lastKind = wanted;
      loading = true;
      query = '';
      category = 'all';
      tier = 'all';
      limit = PAGE;
      openId = '';
    }
    let cancelled = false;
    api(`/api/catalog?kind=${wanted}`)
      .then((data) => {
        if (cancelled) return;
        items = data.items || [];
        error = '';
        loading = false;
      })
      .catch((err) => {
        if (cancelled) return;
        error = serverText(err.message);
        loading = false;
      });
    return () => {
      cancelled = true;
    };
  });

  // Search text per item, built once per fetch.
  const hay = $derived(
    new Map(
      items.map((item) => [
        item.id,
        [item.title?.ru, item.title?.en, item.summary?.ru, item.summary?.en, item.source?.name, item.id].join(' ').toLowerCase(),
      ]),
    ),
  );
  const words = $derived(query.trim().toLowerCase().split(/\s+/).filter(Boolean));
  const tiered = $derived(tier === 'all' ? items : items.filter((item) => item.tier === tier));
  const searched = $derived(words.length ? tiered.filter((item) => words.every((w) => hay.get(item.id).includes(w))) : tiered);
  const counts = $derived.by(() => {
    const out = {};
    for (const item of searched) out[item.category] = (out[item.category] || 0) + 1;
    return out;
  });
  const present = $derived(CATEGORIES.filter((c) => counts[c]));
  const flat = $derived(words.length > 0 || category !== 'all');
  const shown = $derived(category === 'all' ? searched : searched.filter((item) => item.category === category));
  const sections = $derived(present.map((c) => ({ category: c, items: searched.filter((item) => item.category === c) })));

  function pickCategory(c) {
    category = c;
    limit = PAGE;
  }

  function changed() {
    tick += 1;
  }

  const emptyText = $derived(
    page === 'agents' ? tr('Агентов в каталоге пока нет.') : page === 'apps' ? tr('Приложений в каталоге пока нет.') : tr('Навыков в каталоге пока нет.'),
  );
</script>

<div class="catalog">
  <header class="head">
    <div class="titles">
      <h1>{pageTitle(page)}</h1>
      <p class="lead">{pageLead(page)}</p>
    </div>
    <div class="pages" role="tablist" aria-label={tr('Разделы каталога')}>
      {#each PAGES as p (p)}
        <button type="button" role="tab" class="k-chip" aria-selected={p === page} aria-pressed={p === page} onclick={() => onpage?.(p)}>{pageTitle(p)}</button>
      {/each}
    </div>
  </header>

  <div class="filters">
    <input
      class="k-input cat-search"
      type="search"
      placeholder={tr('Поиск по каталогу')}
      aria-label={tr('Поиск по каталогу')}
      bind:value={query}
      oninput={() => (limit = PAGE)}
    />
    <div class="tiers" role="group" aria-label={tr('Чьи элементы показывать')}>
      <button type="button" class="k-chip" aria-pressed={tier === 'all'} onclick={() => (tier = 'all')}>{tr('Все')}</button>
      <button type="button" class="k-chip" aria-pressed={tier === 'verified'} onclick={() => (tier = 'verified')}>{tr('Проверенные')}</button>
      <button type="button" class="k-chip" aria-pressed={tier === 'public'} onclick={() => (tier = 'public')}>{tr('Публичные')}</button>
    </div>
  </div>

  {#if present.length > 1 || category !== 'all'}
    <div class="cats" role="group" aria-label={tr('Категории')}>
      <button type="button" class="k-chip" aria-pressed={category === 'all'} onclick={() => pickCategory('all')}>{tr('Все категории')}</button>
      {#each present as c (c)}
        <button type="button" class="k-chip" aria-pressed={category === c} onclick={() => pickCategory(c)}>
          {categoryLabel(c)} <span class="count">{counts[c]}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if page === 'skills' && !flat}
    <YourSkills {skills} {bots} onopen={(id) => (openId = id)} />
  {/if}

  {#if loading}
    <p class="quiet" role="status">{tr('Загружаю каталог…')}</p>
  {:else if error}
    <div class="cat-empty" role="alert">
      <p>{tr('Каталог не загрузился: {error}', { error })}</p>
      <button type="button" class="k-btn" onclick={changed}>{tr('Повторить')}</button>
    </div>
  {:else if items.length === 0}
    <div class="cat-empty"><p>{emptyText}</p></div>
  {:else if shown.length === 0}
    <div class="cat-empty">
      <p>{tr('Ничего не нашлось.')}</p>
      <button
        type="button"
        class="k-btn"
        onclick={() => {
          query = '';
          tier = 'all';
          pickCategory('all');
        }}>{tr('Сбросить фильтры')}</button>
    </div>
  {:else if flat}
    <p class="quiet">{tr('Найдено: {count}', { count: shown.length })}</p>
    <ul class="k-lrows">
      {#each shown.slice(0, limit) as item (item.id)}
        <CatalogCard {item} onopen={(id) => (openId = id)} />
      {/each}
    </ul>
    {#if shown.length > limit}
      <div class="more"><button type="button" class="k-btn" onclick={() => (limit += PAGE)}>{tr('Показать ещё')}</button></div>
    {/if}
  {:else}
    {#each sections as section (section.category)}
      <section class="section" aria-labelledby="cat-{section.category}">
        <div class="section-head">
          <h2 id="cat-{section.category}">{categoryLabel(section.category)}</h2>
          {#if section.items.length > SECTION}
            <button type="button" class="k-btn k-btn--ghost k-btn--sm" onclick={() => pickCategory(section.category)}>
              {tr('Показать все ({count})', { count: section.items.length })}
            </button>
          {/if}
        </div>
        <ul class="k-lrows">
          {#each section.items.slice(0, SECTION) as item (item.id)}
            <CatalogCard {item} onopen={(id) => (openId = id)} />
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

{#if openId}
  <CatalogDrawer id={openId} {bots} onclose={() => (openId = '')} onchanged={changed} {onopenbot} />
{/if}

<style>
  .catalog {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-4, 16px);
    max-width: 1120px;
    margin: 0 auto;
    padding: var(--k-space-6, 24px) var(--k-space-6, 24px) var(--k-space-12, 48px);
    color: var(--k-text, inherit);
    font-family: var(--k-font-body, inherit);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--k-space-3, 12px);
  }
  h1 {
    margin: 0;
    font: 800 var(--k-text-2xl, 28px) / 1.15 var(--k-font-display, inherit);
  }
  .lead {
    margin: 4px 0 0;
    color: var(--k-text-2, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  .pages,
  .tiers,
  .cats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-3, 12px);
  }
  .cat-search {
    flex: 1 1 260px;
    min-width: 0;
  }
  .count {
    opacity: 0.6;
    font-variant-numeric: tabular-nums;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    margin-top: var(--k-space-2, 8px);
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3, 12px);
  }
  h2 {
    margin: 0;
    font: 800 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, inherit);
  }
  .more {
    display: flex;
    justify-content: center;
  }
  .quiet {
    margin: 0;
    color: var(--k-text-3, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  .cat-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-12, 48px) var(--k-space-4, 16px);
    border: 1px dashed var(--k-line-strong);
    border-radius: var(--k-radius-lg, 26px);
    color: var(--k-text-2, inherit);
    text-align: center;
  }
  .cat-empty p {
    margin: 0;
  }
</style>
