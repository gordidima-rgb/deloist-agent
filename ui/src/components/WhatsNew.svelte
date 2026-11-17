<script>
  // Окно «Что нового» посередине экрана: картинка сверху, страница на каждую категорию
  // (Новое, Улучшено, Исправлено), листать стрелками или кнопкой «Далее», внизу «Все изменения».
  import Modal from './kit/Modal.svelte';
  import Button from './kit/Button.svelte';
  import Icon from './kit/Icon.svelte';
  import { tr } from '../lib/i18n.js';
  import { markdownToText, buildPages, buildFullList, mergeSections } from '../lib/whatsNew.js';

  let {
    current = '',
    from = '',
    sections = [],
    // Загрузчик полного журнала: async () => [{ version, date, markdown }]. Без него ссылки «Все изменения» нет.
    loadAll = null,
    // Открыть сразу полный список.
    startAll = false,
    onclose = null,
  } = $props();

  let open = $state(true);
  let view = $state('pager');
  let index = $state(0);
  let everything = $state(null);
  let loading = $state(false);
  let failed = $state(false);
  let imageBroken = $state(false);
  let strip = $state();
  let paper = $state(false);

  const built = $derived(buildPages(sections));
  const pages = $derived(built.pages);
  const image = $derived(imageBroken ? '' : built.image);
  const page = $derived(pages[Math.min(index, Math.max(pages.length - 1, 0))]);
  const last = $derived(index >= pages.length - 1);
  const date = $derived(sections?.[0]?.date || '');
  const full = $derived(everything ? buildFullList(mergeSections(sections, everything)) : []);

  function label(key, raw) {
    if (key === 'new') return tr('Новое');
    if (key === 'improved') return tr('Улучшено');
    if (key === 'fixed') return tr('Исправлено');
    return tr(raw);
  }

  function close() {
    onclose?.();
  }

  function next() {
    if (!last) {
      index += 1;
      return;
    }
    open = false;
    close();
  }

  function prev() {
    if (index > 0) index -= 1;
  }

  async function showAll() {
    view = 'all';
    if (everything || !loadAll) return;
    loading = true;
    failed = false;
    try {
      everything = (await loadAll()) || [];
    } catch {
      failed = true;
    }
    loading = false;
  }

  function keydown(event) {
    if (!open || view !== 'pager' || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'ArrowRight' && !last) {
      event.preventDefault();
      next();
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      prev();
    }
  }

  $effect(() => {
    if (startAll) showAll();
  });

  // Живые шары рядом: радуются, пока окно открыто. Без WebGL остаётся рисунок из кита.
  $effect(() => {
    if (!strip || image || view !== 'pager') {
      paper = false;
      return;
    }
    const avatars = [];
    const timers = [];
    const orbs = typeof window !== 'undefined' ? window.CrewAvatar : null;
    const reduce = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (orbs && orbs.ORBS?.length) {
      const skins = [0, 1, 2, 5, 8].map((n) => n % orbs.ORBS.length);
      skins.forEach((skin, i) => {
        const el = document.createElement('span');
        el.className = 'wn-orb';
        strip.append(el);
        const avatar = orbs.mount(el, { size: 72, skin, color: orbs.COLORS?.[(skin * 3) % orbs.COLORS.length], name: '', still: reduce });
        if (!avatar) return el.remove();
        avatars.push({ avatar, el });
        if (!reduce) {
          const cheer = () => avatar.feel?.(i % 2 ? 'star' : 'joy', 2.2);
          timers.push(setTimeout(cheer, 250 + i * 220));
          timers.push(setInterval(cheer, 4200));
        }
      });
    }
    paper = avatars.length === 0;
    return () => {
      timers.forEach((t) => (clearTimeout(t), clearInterval(t)));
      avatars.forEach(({ avatar, el }) => {
        try { avatar.destroy?.(); } catch {}
        el.remove();
      });
    };
  });
</script>

<svelte:window onkeydown={keydown} />

<Modal bind:open title={tr('Что нового в Deloist {version}', { version: current })} description={view === 'pager' ? (from ? tr('Обновлено с {from} до {to} · {date}', { from, to: current, date }) : tr('Версия {current} · {date}', { current, date })) : tr('Все изменения')} size="md" class="wn-panel" onclose={close}>
  {#if view === 'pager'}
    <div class="wn-hero" aria-hidden="true">
      {#if image}
        <img class="wn-image" src={image} alt="" onerror={() => (imageBroken = true)} />
      {:else}
        <div class="wn-strip" bind:this={strip}></div>
        {#if paper}
          <div class="wn-paper">
            {#each ['sky', 'mint', 'lemon', 'rose', 'violet'] as color, i}
              <span class="wn-dot wn-dot--{color}" style:--wn-i={i}></span>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
    {#if page}
      {#key index}
        <section class="wn-page k-sharpen-in" aria-live="polite">
          <h3 class="wn-label">{label(page.key, page.title)}</h3>
          <ul class="wn-list">
            {#each page.items as item}
              <li class="wn-item">
                {#if item.lead}<strong class="wn-lead">{@html markdownToText(item.lead)}</strong>{/if}
                {#if item.text}<span class="wn-text">{@html markdownToText(item.text)}</span>{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/key}
    {:else}
      <p class="wn-empty">{tr('Записей об этой версии пока нет.')}</p>
    {/if}
  {:else}
    <div class="wn-all">
      {#if loading}
        <p class="wn-empty">{tr('Загружаю…')}</p>
      {:else if failed}
        <p class="wn-empty">{tr('Не удалось загрузить список изменений.')}</p>
      {:else}
        {#each full as release}
          <section class="wn-release">
            <h3 class="wn-release__head">
              <span class="wn-release__version">{release.version}</span>
              {#if release.date}<span class="wn-release__date">{release.date}</span>{/if}
            </h3>
            {#each release.groups as group}
              <h4 class="wn-label">{label(group.key, group.title)}</h4>
              <ul class="wn-list">
                {#each group.items as item}
                  <li class="wn-item">
                    {#if item.lead}<strong class="wn-lead">{@html markdownToText(item.lead)}</strong>{/if}
                    {#if item.text}<span class="wn-text">{@html markdownToText(item.text)}</span>{/if}
                  </li>
                {/each}
              </ul>
            {/each}
          </section>
        {/each}
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    {#if view === 'pager'}
      <div class="wn-foot">
        {#if loadAll}
          <button type="button" class="wn-link" onclick={showAll}>{tr('Все изменения')}</button>
        {:else}
          <span></span>
        {/if}
        {#if pages.length > 1}
          <div class="wn-pager" role="group" aria-label={tr('Страницы')}>
            <button type="button" class="k-btn k-btn--ghost k-btn--icon" aria-label={tr('Назад')} disabled={index === 0} onclick={prev}><Icon name="chevron-left" size={null} /></button>
            <span class="wn-count">{tr('{n} из {total}', { n: index + 1, total: pages.length })}</span>
            <button type="button" class="k-btn k-btn--ghost k-btn--icon" aria-label={tr('Вперёд')} disabled={last} onclick={next}><Icon name="chevron-right" size={null} /></button>
          </div>
        {/if}
        <Button variant="primary" data-autofocus onclick={next}>{last ? tr('Понятно') : tr('Далее')}</Button>
      </div>
    {:else}
      <div class="wn-foot">
        {#if pages.length}
          <button type="button" class="wn-link" onclick={() => (view = 'pager')}>
            <Icon name="chevron-left" size={null} class="wn-link__icon" />{tr('Назад')}
          </button>
        {:else}
          <span></span>
        {/if}
        <Button variant="primary" onclick={() => { open = false; close(); }}>{tr('Понятно')}</Button>
      </div>
    {/if}
  {/snippet}
</Modal>

<style>
  :global(.wn-panel) {
    width: min(520px, 100%);
  }
  .wn-hero {
    display: grid;
    place-items: center;
    min-height: 112px;
    margin-bottom: var(--k-space-4, 16px);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2);
    overflow: hidden;
  }
  .wn-image {
    display: block;
    width: 100%;
    max-height: 200px;
    object-fit: cover;
  }
  .wn-strip {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px) var(--k-space-3, 12px);
  }
  .wn-strip:empty {
    display: none;
  }
  .wn-strip :global(.wn-orb) {
    position: relative;
    display: block;
    width: 72px;
    height: 72px;
    flex: none;
  }
  .wn-paper {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--k-space-3);
    padding: var(--k-space-4) 0;
  }
  .wn-dot {
    position: relative;
    display: block;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: var(--wn-color);
    animation: wn-hop 1.6s var(--k-ease-spring) infinite;
    animation-delay: calc(var(--wn-i) * 120ms);
  }
  .wn-dot::before,
  .wn-dot::after {
    content: '';
    position: absolute;
    top: 18px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--k-ink);
  }
  .wn-dot::before {
    left: 13px;
  }
  .wn-dot::after {
    right: 13px;
  }
  .wn-dot--sky { --wn-color: var(--k-bot-sky); }
  .wn-dot--mint { --wn-color: var(--k-bot-mint); }
  .wn-dot--lemon { --wn-color: var(--k-bot-lemon); }
  .wn-dot--rose { --wn-color: var(--k-bot-rose); }
  .wn-dot--violet { --wn-color: var(--k-bot-violet); }
  @keyframes wn-hop {
    0%, 60%, 100% { transform: translateY(0); }
    30% { transform: translateY(-8px); }
  }
  @media (prefers-reduced-motion: reduce) {
    .wn-dot { animation: none; }
  }
  .wn-page,
  .wn-all {
    max-height: min(44vh, 360px);
    overflow-y: auto;
    padding-right: var(--k-space-1, 4px);
  }
  .wn-all {
    max-height: min(56vh, 460px);
  }
  .wn-label {
    margin: 0 0 var(--k-space-2, 8px);
    color: var(--k-text-3);
    font: 600 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
  }
  .wn-list {
    margin: 0 0 var(--k-space-3, 12px);
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
  }
  .wn-item {
    color: var(--k-text-2);
    font: 400 var(--k-text-md, 15px) / 1.5 var(--k-font-body, system-ui);
    overflow-wrap: anywhere;
  }
  .wn-lead {
    margin-right: 0.3em;
    color: var(--k-text);
    font-weight: 600;
  }
  .wn-item :global(code) {
    padding: 2px 4px;
    border-radius: var(--k-radius-xs, 6px);
    background: var(--k-surface-2);
    font: 400 0.9em var(--k-font-mono, ui-monospace, monospace);
  }
  .wn-item :global(a) {
    color: var(--k-brand-text);
  }
  .wn-empty {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-md, 15px) / 1.5 var(--k-font-body, system-ui);
  }
  .wn-release + .wn-release {
    margin-top: var(--k-space-5, 20px);
    padding-top: var(--k-space-4, 16px);
    border-top: 1px solid var(--k-line);
  }
  .wn-release__head {
    display: flex;
    align-items: baseline;
    gap: var(--k-space-2, 8px);
    margin: 0 0 var(--k-space-3, 12px);
    font: 800 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, system-ui);
  }
  .wn-release__date {
    color: var(--k-text-3);
    font: 400 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
  }
  .wn-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3, 12px);
    width: 100%;
  }
  .wn-pager {
    display: flex;
    align-items: center;
    gap: var(--k-space-1, 4px);
  }
  .wn-count {
    min-width: 52px;
    text-align: center;
    color: var(--k-text-2);
    font: 600 var(--k-text-sm, 13px) / 1 var(--k-font-body, system-ui);
    font-variant-numeric: tabular-nums;
  }
  .wn-link {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-1, 4px);
    padding: var(--k-space-1, 4px) 0;
    border: 0;
    background: none;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .wn-link:hover {
    color: var(--k-text);
  }
  .wn-link :global(.wn-link__icon) {
    width: 16px;
    height: 16px;
  }
</style>
