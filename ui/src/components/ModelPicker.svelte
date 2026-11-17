<script>
  // Выбор модели одним кликом: строка «Модель» с текущим выбором и окно со всеми моделями
  // подключённых провайдеров. Слева лента логотипов провайдеров и «+», справа шапка выбранного
  // провайдера, поиск и список моделей с круглой отметкой; лишние модели свёрнуты под «Ещё модели».
  // Выбор отдаёт пару { driver, model } — те же два поля бота и общих настроек, что раньше
  // заполнялись руками. Список моделей приходит снаружи (из /api/providers), здесь его нет.
  import { tick, onMount } from 'svelte';
  import { tr } from '../lib/i18n.js';
  import { buildGroups, filterGroups, toValue, isCurrent, labelOf, iconSvg, splitModels } from '../lib/modelPicker.js';
  import { Tooltip, Badge, TextInput, Icon, Button, IconButton } from './kit';

  let {
    providers = [],
    plan = null,
    loading = false,
    settings = {},
    value = { driver: '', model: '' },
    label = '',
    // Метка «Модель» только для скринридера: рядом уже есть заголовок раздела, дубль не нужен.
    hideLabel = false,
    allowDefault = true,
    onselect,
    onconnect,
    // Всплывающий вариант (шапка чата): окно открыто сразу, строки «Модель» нет, окно встаёт под
    // `anchor`, а под списком выводится `footer` (усилие, ссылка на настройки).
    startOpen = false,
    bare = false,
    // Всплывающее окно встаёт левым краем к `anchor` (по умолчанию в шапке чата — правым).
    alignLeft = false,
    anchor = null,
    defaultLabel = '',
    // Драйвер, на котором реально работает выбор «как в настройках»: с него открывается список.
    activeDriver = '',
    footer = null,
    onclose,
  } = $props();

  let open = $state(false);
  let query = $state('');
  let active = $state('');
  let expanded = $state({});
  let pos = $state({ left: 0, top: 0, bottom: 0, width: 440, height: 400, down: true });
  let trigger = $state(null);
  let pop = $state();
  let root = $state();

  const groups = $derived(buildGroups({ providers, plan, settings, value }));
  const searching = $derived(query.trim().length > 0);
  // Виден один провайдер (выбранный на ленте); запрос сначала ищет в нём. Если там пусто, а у других
  // провайдеров есть совпадения, показываем их, сгруппировав по провайдеру: лента — только быстрый переход.
  const inActive = $derived(filterGroups(groups, searching ? query : '', active));
  const everywhere = $derived(searching && !inActive.length ? filterGroups(groups, query) : []);
  const global = $derived(everywhere.length > 0);
  const shown = $derived(global ? everywhere : inActive);
  const activeGroup = $derived(groups.find((g) => g.id === active) || null);
  const activeIdx = $derived(groups.findIndex((g) => g.id === active));
  const countOf = (g) => g.models.filter((m) => !m.typed).length;
  const current = $derived(labelOf(groups, value));
  const showDefault = $derived(allowDefault && (!searching || tr('как в настройках').includes(query.trim().toLowerCase())));
  const heading = $derived(label || tr('Модель'));
  $effect(() => {
    if (open && groups.length && !groups.some((g) => g.id === active)) active = (groups.find((g) => g.driver === (value.driver || activeDriver)) || groups[0]).id;
  });

  function iconColor() {
    try {
      const css = getComputedStyle(document.documentElement).getPropertyValue('--k-text').trim();
      const hex = /^#([0-9a-f]{6})$/i.exec(css);
      return hex ? hex[1] : '151414';
    } catch {
      return '151414';
    }
  }

  const letter = (g) => (g.name || g.id || '?').trim().charAt(0).toUpperCase();

  function place() {
    const el = anchor || trigger;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const width = Math.min(Math.max(r.width, 440), vw - 16);
    const left = Math.max(8, Math.min(bare && !alignLeft ? r.right - width : r.left, vw - width - 8));
    const below = vh - r.bottom - 12;
    const above = r.top - 12;
    const down = below >= 300 || below >= above;
    const height = Math.max(260, Math.min(footer ? 560 : 480, down ? below : above));
    // Высота по содержимому (max-height): при двух моделях окно не стоит пустым. Вверх — прижато низом к кнопке.
    pos = { left, width, height, down, top: r.bottom + 6, bottom: window.innerHeight - r.top + 6 };
  }

  async function show() {
    place();
    query = '';
    expanded = {};
    active = (groups.find((g) => g.driver === (value.driver || activeDriver)) || groups[0])?.id || '';
    open = true;
    await tick();
    pop?.querySelector('.mp-search input')?.focus();
  }

  function hide(focusBack = true) {
    open = false;
    if (focusBack && !bare) trigger?.focus();
    onclose?.(focusBack);
  }

  onMount(() => {
    if (startOpen) show();
  });

  function pick(group, model) {
    const next = toValue(group, model);
    hide();
    onselect?.(next);
  }

  function pickDefault() {
    hide();
    onselect?.({ driver: '', model: '' });
  }

  function onkeydown(e) {
    if (e.key === 'Escape' && open) {
      e.stopPropagation();
      e.preventDefault();
      hide();
    }
  }

  function outside(e) {
    if (open && root && !root.contains(e.target) && !anchor?.contains?.(e.target)) hide(false);
  }

  function addProvider() {
    hide(false);
    onconnect?.();
  }

  const badgeText = (b) => (b === 'free' ? tr('Бесплатно') : b === 'sub' ? tr('Подписка') : b === 'default' ? tr('по умолчанию') : '');
  const isOpenMore = (g, more) => searching || expanded[g.id] || more.some((m) => isCurrent(g, m, value));
</script>

<svelte:window onpointerdown={outside} onresize={() => open && place()} />

{#snippet logo(g)}
  {@const svg = iconSvg(g)}
  {#if svg}
    <span class="mp-logo" aria-hidden="true">{@html svg}</span>
  {:else if g.icon}
    <span class="mp-logo" aria-hidden="true">
      <img src="/api/providers/{encodeURIComponent(g.icon)}/icon?color={iconColor()}" alt="" onerror={(e) => e.currentTarget.replaceWith(document.createTextNode(letter(g)))} />
    </span>
  {:else}
    <span class="mp-logo mp-letter" aria-hidden="true">{letter(g)}</span>
  {/if}
{/snippet}

{#snippet check()}
  <span class="mp-check" aria-hidden="true"><Icon name="check-soft" size={null} stroke={2.4} /></span>
{/snippet}

{#snippet modelRow(g, m)}
  {@const on = isCurrent(g, m, value)}
  <Button variant="ghost" class="mp-item{on ? ' sel' : ''}" role="menuitemradio" aria-checked={on} onclick={() => pick(g, m)}>
    {@render logo(g)}
    <span class="mp-name">{m.typed ? tr('Использовать «{name}»', { name: m.name }) : m.name}</span>
    {#if m.badge}<Badge tone="neutral" text={badgeText(m.badge)} />{/if}
    {@render check()}
  </Button>
{/snippet}

<div class="mp" bind:this={root} {onkeydown} role="presentation">
  <Button variant="ghost" class="mp-row{bare ? ' bare' : ''}{hideLabel ? ' no-label' : ''}" bind:ref={trigger} aria-haspopup="dialog" aria-expanded={open} onclick={() => (open ? hide() : show())}>
    <span class="mp-label" class:sr-only={hideLabel}>{heading}</span>
    <span class="mp-value">
      {#if current.group}
        {@render logo(current.group)}
      {/if}
      <span class="mp-text" class:muted={!value.driver}>{current.text}</span>
    </span>
    <Icon name="chevron-down" size={null} class={['mp-chev', open && 'up']} />
  </Button>

  {#if open}
    <div class="mp-pop" class:up={!pos.down} bind:this={pop} role="dialog" aria-label={tr('Выбор модели')} style:left="{pos.left}px" style:top={pos.down ? `${pos.top}px` : null} style:bottom={pos.down ? null : `${pos.bottom}px`} style:width="{pos.width}px" style:max-height="{pos.height}px">
      <div class="mp-rail" class:scroll={groups.length > 9} role="toolbar" aria-label={tr('Провайдеры')}>
        <div class="mp-rail-in">
          {#if activeIdx >= 0}
            <span class="mp-rail-dot" class:off={global} style:transform="translateY({activeIdx * 36}px)" aria-hidden="true"></span>
          {/if}
          {#each groups as g (g.id)}
            <Tooltip text={g.name} placement="right">
              <IconButton label={g.name} title={null} class="mp-rail-btn{!global && active === g.id ? ' on' : ''}" aria-pressed={!global && active === g.id} onclick={() => { active = g.id; query = ''; }}>
                {#snippet icon()}{@render logo(g)}{/snippet}
              </IconButton>
            </Tooltip>
          {/each}
          <span class="mp-rail-gap"></span>
          <Tooltip text={tr('Подключить ИИ')} placement="right">
            <IconButton name="plus" label={tr('Подключить ИИ')} title={null} class="mp-rail-btn mp-add" onclick={addProvider} />
          </Tooltip>
        </div>
      </div>
      <div class="mp-main">
        <header class="mp-head">
          <div class="mp-head-text">
            <span class="mp-title">{global || !activeGroup ? tr('Поиск моделей') : activeGroup.name}</span>
            {#if !global && activeGroup && countOf(activeGroup) > 0}<span class="mp-count">{countOf(activeGroup)}</span>{/if}
          </div>
          <div class="mp-search">
            <TextInput type="search" bind:value={query} placeholder={tr('Поиск моделей')} aria-label={tr('Поиск моделей')} />
            <Icon name="search-short" size={null} />
          </div>
        </header>
        <div class="mp-list" role="group">
          {#if loading}
            <p class="mp-hint" role="status">{tr('Загрузка')}</p>
          {:else}
          {#if showDefault}
            <Button variant="ghost" class="mp-item{!value.driver ? ' sel' : ''}" role="menuitemradio" aria-checked={!value.driver} onclick={pickDefault}>
              <span class="mp-logo mp-default" aria-hidden="true"><Icon name="sliders" size={null} /></span>
              <span class="mp-name">{defaultLabel || tr('как в настройках')}</span>
              {@render check()}
            </Button>
          {/if}
          {#each shown as g (g.id)}
            {@const parts = splitModels(g)}
            {@const more = isOpenMore(g, parts.more)}
            {#if global}
              <div class="mp-group">{@render logo(g)}<span>{g.name}</span></div>
            {/if}
            {#if g.manual && !searching && !global}
              <p class="mp-hint">{tr('Впиши имя модели в поиск и выбери строку «Использовать».')}</p>
            {/if}
            {#each parts.main as m (m.id + (m.typed ? '#typed' : ''))}
              {@render modelRow(g, m)}
            {/each}
            {#if parts.more.length}
              <Button variant="ghost" size="sm" class="mp-more" aria-expanded={more} onclick={() => (expanded = { ...expanded, [g.id]: !more })}>
                <Icon name="chevron-right" size={null} class={['mp-more-chev', more && 'down']} />
                <span>{tr('Ещё модели · {n}', { n: parts.more.length })}</span>
              </Button>
              {#if more}
                {#each parts.more as m (m.id)}
                  {@render modelRow(g, m)}
                {/each}
              {/if}
            {/if}
          {/each}
          {#if !searching && shown.reduce((n, g) => n + g.models.length, 0) < 6}
            <Button variant="ghost" size="sm" iconName="plus" class="mp-connect" onclick={addProvider}>{tr('Подключить новую ИИ…')}</Button>
          {/if}
          {#if !shown.length && !showDefault}
            <p class="mp-hint">{tr('Ничего не нашлось')}</p>
          {/if}
          {/if}
        </div>
        {#if footer}
          <div class="mp-footer">{@render footer()}</div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .mp {
    position: relative;
    min-width: 0;
  }
  .mp :global(.mp-row) {
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    width: 100%;
    min-height: 44px;
    padding: 0 var(--k-space-4);
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-pill);
    background: var(--k-surface);
    color: var(--k-text);
    font: inherit;
    font-size: var(--k-text-sm);
    text-align: left;
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out);
  }
  .mp :global(.mp-row.bare) {
    display: none;
  }
  .mp-footer {
    flex: none;
    padding: var(--k-space-3);
    border-top: 1px solid var(--k-line);
  }
  .mp :global(.mp-row):hover {
    background: var(--k-surface-2);
  }
  .mp :global(.mp-row):focus-visible,
  .mp :global(.mp-rail-btn):focus-visible,
  .mp :global(.mp-item):focus-visible,
  .mp :global(.mp-more):focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  .mp-label {
    color: var(--k-text-3);
    flex: none;
  }
  .mp-label.sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .mp :global(.mp-row.no-label .mp-value ){
    justify-content: flex-start;
  }
  .mp-value {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--k-space-2);
    flex: 1;
    min-width: 0;
    font-weight: 600;
  }
  .mp-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp-text.muted {
    font-weight: 600;
    color: var(--k-text-2);
  }
  .mp-logo {
    flex: none;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    color: inherit;
  }
  .mp-logo :global(svg),
  .mp-logo img {
    width: 20px;
    height: 20px;
    display: block;
  }
  .mp-letter {
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font-size: var(--k-text-micro);
    font-weight: 700;
  }
  :global(.mp-chev) {
    flex: none;
    width: 16px;
    height: 16px;
    color: var(--k-text-3);
    transition: transform var(--k-dur-fast) var(--k-ease-out);
  }
  :global(.mp-chev.up) {
    transform: rotate(180deg);
  }
  .mp-pop {
    position: fixed;
    z-index: 1000;
    display: flex;
    overflow: hidden;
    border-radius: var(--k-radius-md);
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow: var(--k-shadow-3);
    transform-origin: top left;
    animation: mp-pop-in 260ms cubic-bezier(0.34, 1.36, 0.64, 1);
  }
  .mp-pop.up {
    transform-origin: bottom left;
  }
  @keyframes mp-pop-in {
    from {
      opacity: 0;
      transform: scale(0.96);
      filter: blur(4px);
    }
    to {
      opacity: 1;
      transform: scale(1);
      filter: blur(0);
    }
  }
  /* Лента: узкая «капсула» с круглыми кнопками; круг активного провайдера один и переезжает по ленте. */
  .mp-rail {
    flex: none;
    width: 44px;
    box-sizing: border-box;
    display: flex;
    margin: var(--k-space-2) 0 var(--k-space-2) var(--k-space-2);
    border-radius: var(--k-radius-lg);
    background: var(--k-canvas);
    box-shadow: inset 0 0 0 1px var(--k-line);
  }
  .mp-rail.scroll {
    overflow-y: auto;
    scrollbar-width: none;
  }
  .mp-rail-in {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0;
  }
  .mp-rail-gap {
    flex: 1;
  }
  .mp-rail-dot {
    position: absolute;
    top: 6px;
    left: 6px;
    width: 32px;
    height: 32px;
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-3);
    box-shadow: inset 0 0 0 1px var(--k-line);
    pointer-events: none;
    transition: transform 300ms cubic-bezier(0.34, 1.36, 0.64, 1), opacity var(--k-dur-fast) var(--k-ease-out);
  }
  .mp-rail-dot.off {
    opacity: 0;
  }
  .mp :global(.mp-rail-btn) {
    position: relative;
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--k-radius-pill);
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out), color var(--k-dur-fast) var(--k-ease-out);
  }
  .mp :global(.mp-rail-btn) :global(svg) {
    width: 18px;
    height: 18px;
  }
  .mp :global(.mp-rail-btn):hover {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .mp :global(.mp-rail-btn.on) {
    background: transparent;
    color: var(--k-text);
  }
  .mp :global(.mp-add) {
    color: var(--k-text-3);
  }
  .mp-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .mp-head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3);
    padding: var(--k-space-3) var(--k-space-3) var(--k-space-2) var(--k-space-3);
  }
  .mp-head-text {
    display: flex;
    align-items: baseline;
    gap: var(--k-space-2);
    min-width: 0;
  }
  .mp-title {
    font-size: var(--k-text-sm);
    font-weight: 700;
    line-height: var(--k-leading-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp-count {
    flex: none;
    font-size: var(--k-text-sm);
    color: var(--k-text-3);
  }
  .mp-search {
    position: relative;
    flex: none;
    width: 168px;
  }
  .mp-search :global(svg) {
    position: absolute;
    right: 12px;
    top: 50%;
    width: 15px;
    height: 15px;
    transform: translateY(-50%);
    color: var(--k-text-3);
    pointer-events: none;
  }
  .mp-search :global(.k-input) {
    width: 100%;
    box-sizing: border-box;
    height: 32px;
    min-height: 0;
    padding: 0 var(--k-space-8) 0 var(--k-space-4);
    border-radius: var(--k-radius-pill);
    font-size: var(--k-text-sm);
  }
  .mp-list {
    overflow-y: auto;
    padding: 0 var(--k-space-2) var(--k-space-2);
    min-height: 0;
    flex: 1;
  }
  .mp-group {
    display: flex;
    align-items: center;
    gap: var(--k-space-2);
    padding: var(--k-space-3) var(--k-space-3) var(--k-space-1);
    font-size: var(--k-text-xs);
    font-weight: 600;
    color: var(--k-text-3);
  }
  .mp-group .mp-logo,
  .mp-group .mp-logo :global(svg),
  .mp-group .mp-logo img {
    width: 14px;
    height: 14px;
  }
  .mp-hint {
    margin: var(--k-space-1) var(--k-space-3) var(--k-space-2);
    font-size: var(--k-text-xs);
    color: var(--k-text-3);
  }
  .mp :global(.mp-item),
  .mp :global(.mp-more) {
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    width: 100%;
    min-height: 36px;
    padding: 4px var(--k-space-3);
    border: 0;
    border-radius: var(--k-radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--k-text-sm);
    text-align: left;
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out);
  }
  .mp :global(.mp-item):hover,
  .mp :global(.mp-more):hover {
    background: var(--k-surface-2);
  }
  .mp :global(.mp-item.sel) {
    font-weight: 600;
  }
  .mp-check {
    flex: none;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    color: transparent;
  }
  .mp-check :global(svg) {
    width: 16px;
    height: 16px;
  }
  .mp :global(.mp-item.sel .mp-check ){
    color: var(--k-text);
  }
  .mp-default {
    color: var(--k-text-3);
  }
  .mp :global(.mp-item .mp-logo ){
    width: 18px;
    height: 18px;
  }
  .mp :global(.mp-item .mp-logo svg),
  .mp :global(.mp-item .mp-logo img ){
    width: 18px;
    height: 18px;
  }
  .mp-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp :global(.mp-more) {
    min-height: 34px;
    color: var(--k-text-3);
    font-size: var(--k-text-xs);
    font-weight: 600;
  }
  :global(.mp-connect) {
    width: 100%;
    justify-content: flex-start;
    margin-top: var(--k-space-1);
  }
  :global(.mp-more-chev) {
    flex: none;
    width: 18px;
    height: 14px;
    transition: transform var(--k-dur-fast) var(--k-ease-out);
  }
  :global(.mp-more-chev.down) {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .mp-pop {
      animation: none;
    }
    .mp-rail-dot,
    :global(.mp-chev),
    :global(.mp-more-chev),
    .mp :global(.mp-row),
    .mp :global(.mp-rail-btn),
    .mp :global(.mp-item),
    .mp :global(.mp-more) {
      transition: none;
    }
  }
</style>
