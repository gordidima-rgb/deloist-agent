<script>
  // Detail drawer: full description, what the item needs, license and source, the agent's skills,
  // and the install / remove / update actions for its kind.
  import { tr } from '../../lib/i18n.js';
  import { api, loc, iconOf, lang, serverText } from '../../lib/api.js';
  import { tierLabel, categoryLabel, builtinLabel, hintLabel } from './Labels.svelte';
  import ConnectFlow from './ConnectFlow.svelte';
  import DiffView from './DiffView.svelte';

  let { id, bots = [], onclose, onchanged, onopenbot } = $props();

  let item = $state.raw(null);
  let error = $state('');
  let busy = $state(false);
  let note = $state('');
  let chosen = $state([]);
  let siteConsent = $state(false);
  let cabinetBot = $state('');
  let cabinetConsent = $state(false);
  let conflicts = $state.raw(null);
  let confirmRemove = $state(false);
  let panel = $state();

  const base = $derived(`/api/catalog/${encodeURIComponent(id)}`);
  const st = $derived(item?.state || {});
  const title = $derived(item ? loc(item.title) : '');
  const needs = $derived.by(() => {
    if (!item) return [];
    const out = [];
    for (const app of item.requires?.apps || []) out.push(tr('Приложение: {name}', { name: app }));
    for (const b of item.requires?.builtin || []) out.push(builtinLabel(b));
    const sites = [...new Set([...(item.requires?.sites || []), ...(item.agent?.sites || []), ...(item.app?.web_cabinet?.sites || [])])];
    if (sites.length) out.push(tr('Сайты: {sites}', { sites: sites.join(', ') }));
    return out;
  });

  async function load() {
    try {
      const data = await api(`${base}?lang=${lang()}`);
      item = data.item;
      error = '';
      const have = new Set(item.state?.bots || []);
      chosen = bots.filter((b) => have.has(b.id)).map((b) => b.id);
      if (!cabinetBot && bots.length) cabinetBot = item.state?.botId || bots[0].id;
    } catch (err) {
      error = serverText(err.message);
    }
  }

  $effect(() => {
    id;
    conflicts = null;
    confirmRemove = false;
    note = '';
    load();
  });

  $effect(() => {
    panel?.focus();
  });

  async function run(fn) {
    busy = true;
    error = '';
    note = '';
    try {
      await fn();
      await load();
      onchanged?.();
    } catch (err) {
      if (err.status === 409 && err.data?.conflicts) conflicts = err.data.conflicts;
      else error = serverText(err.message);
    } finally {
      busy = false;
    }
  }

  function toggleBot(botId) {
    chosen = chosen.includes(botId) ? chosen.filter((x) => x !== botId) : [...chosen, botId];
  }

  // Skill: add to the chosen bots; bots unticked later lose it.
  const saveSkill = () =>
    run(async () => {
      const data = await api(`${base}/install`, { method: 'POST', body: { bots: chosen, lang: lang() } });
      const skillId = data.skill.id;
      for (const bot of bots) {
        if (!chosen.includes(bot.id) && (bot.skills || []).includes(skillId)) {
          await api(`/api/bots/${encodeURIComponent(bot.id)}`, { method: 'PATCH', body: { skills: bot.skills.filter((s) => s !== skillId) } });
        }
      }
      note = chosen.length ? tr('Навык подключён ботам: {count}', { count: chosen.length }) : tr('Навык добавлен. Подключите его ботам, когда понадобится.');
    });

  const addAgent = () =>
    run(async () => {
      const data = await api(`${base}/install`, { method: 'POST', body: { sites: siteConsent, lang: lang() } });
      note = tr('Бот «{name}» в команде.', { name: data.bot?.name || title });
    });

  const addCabinet = () =>
    run(async () => {
      const data = await api(`${base}/install`, { method: 'POST', body: { bot: cabinetBot, consent: cabinetConsent, lang: lang() } });
      note = tr('Сайт добавлен боту. Откройте его браузер и войдите сами.');
    });

  const openBrowser = (botId) => run(() => api(`/api/bots/${encodeURIComponent(botId)}/browser`, { method: 'POST' }));

  const remove = () =>
    run(async () => {
      await api(base, { method: 'DELETE' });
      confirmRemove = false;
      note = tr('Удалено.');
    });

  const update = (resolve) =>
    run(async () => {
      await api(`${base}/update`, { method: 'POST', body: resolve ? { resolve } : {} });
      conflicts = null;
      note = resolve === 'mine' ? tr('Оставили вашу версию. Новая версия каталога отмечена как просмотренная.') : tr('Обновлено.');
    });

  function removeWarning() {
    if (item.kind === 'agent') return tr('Бот удалится вместе с перепиской и памятью. Навыки, которые принёс агент, останутся, если ими пользуются другие боты.');
    if (item.kind === 'skill') return tr('Навык уберётся у всех ботов.');
    if (item.mode === 'cabinet') return tr('Бот больше не сможет действовать на этих сайтах.');
    return tr('Приложение отключится, его ключи удалятся.');
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose?.()} />

<div class="scrim" role="presentation" onclick={() => onclose?.()}></div>
<aside class="cat-drawer" aria-label={title || tr('Подробности')} tabindex="-1" bind:this={panel}>
  <header class="top">
    <button type="button" class="k-btn k-btn--ghost k-btn--icon" aria-label={tr('Закрыть')} title={tr('Закрыть')} onclick={() => onclose?.()}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
  </header>

  {#if !item && error}
    <p class="k-hint is-error" role="alert">{error}</p>
  {:else if !item}
    <p class="quiet" role="status">{tr('Загружаю…')}</p>
  {:else}
    <div class="hero">
      <span class="icon" aria-hidden="true">{iconOf(item) || title.slice(0, 1)}</span>
      <div>
        <h2>{title}</h2>
        <p class="meta">
          <span class="tier" class:verified={item.tier === 'verified'}>{tierLabel(item.tier)}</span>
          <span>{categoryLabel(item.category)}</span>
          <span>{tr('версия {version}', { version: item.version })}</span>
        </p>
      </div>
    </div>

    <p class="desc">{loc(item.description) || loc(item.summary)}</p>

    {#if needs.length}
      <section>
        <h3>{tr('Что нужно')}</h3>
        <ul class="needs">{#each needs as need (need)}<li>{need}</li>{/each}</ul>
      </section>
    {/if}

    {#if item.kind === 'agent' && item.skillItems?.length}
      <section>
        <h3>{tr('Навыки агента')}</h3>
        <ul class="skills">
          {#each item.skillItems as skill (skill.id)}
            <li><span class="mini" aria-hidden="true">{iconOf(skill)}</span><span><b>{loc(skill.title)}</b> · {loc(skill.summary)}</span></li>
          {/each}
        </ul>
        {#if hintLabel(item.agent?.model_hint)}
          <p class="quiet">{hintLabel(item.agent.model_hint)}. {tr('Бот работает на вашей модели по умолчанию, поменять её можно в профиле бота.')}</p>
        {/if}
      </section>
    {/if}

    {#if item.kind === 'skill'}
      <details class="body">
        <summary>{tr('Текст навыка')}</summary>
        <pre>{loc(item.skill?.body)}</pre>
      </details>
    {/if}

    <section class="license">
      <h3>{tr('Источник и лицензия')}</h3>
      <p>
        {item.source?.name} · {item.source?.license}
        {#if item.source?.modified}· {tr('изменено для Deloist')}{/if}
      </p>
      {#if item.source?.notice}<p class="quiet">{item.source.notice}</p>{/if}
      {#if item.source?.url}<a href={item.source.url} target="_blank" rel="noopener noreferrer">{tr('Открыть источник')}</a>{/if}
    </section>

    <section class="act">
      {#if conflicts}
        <h3>{tr('Вы правили этот текст')}</h3>
        <p class="quiet">{tr('В каталоге вышла новая версия. Ваши правки не тронуты: выберите, что оставить.')}</p>
        {#each conflicts as c (c.what + c.id)}
          <div class="conflict">
            <b>{c.what === 'persona' ? tr('Характер бота «{name}»', { name: c.title }) : tr('Навык «{name}»', { name: c.title })}</b>
            <DiffView mine={c.mine.body} theirs={c.theirs.body} />
          </div>
        {/each}
        <div class="buttons">
          <button type="button" class="k-btn" disabled={busy} onclick={() => update('mine')}>{tr('Оставить мою')}</button>
          <button type="button" class="k-btn k-btn--primary" disabled={busy} onclick={() => update('theirs')}>{tr('Взять новую')}</button>
        </div>
        <p class="k-hint">{tr('Ваша версия навыка останется в истории: её можно вернуть кнопкой «Отменить правку».')}</p>
      {:else}
        {#if st.update}
          <div class="update">
            <span>{tr('Вышла версия {version}', { version: item.version })}</span>
            <button type="button" class="k-btn k-btn--primary k-btn--sm" disabled={busy} onclick={() => update('')}>{tr('Обновить')}</button>
          </div>
        {/if}

        {#if item.kind === 'skill'}
          <h3>{tr('Каким ботам')}</h3>
          {#if bots.length === 0}
            <p class="quiet">{tr('Ботов пока нет: навык можно добавить и подключить позже.')}</p>
          {:else}
            <div class="bots">
              {#each bots as bot (bot.id)}
                <label class="cat-bot">
                  <input type="checkbox" checked={chosen.includes(bot.id)} onchange={() => toggleBot(bot.id)} />
                  <span class="dot" style:background={bot.color}></span>
                  {bot.name}
                </label>
              {/each}
            </div>
          {/if}
          <div class="buttons">
            <button type="button" class="k-btn k-btn--primary" disabled={busy} onclick={saveSkill}>{st.installed ? tr('Сохранить') : tr('Добавить')}</button>
          </div>
        {:else if item.kind === 'agent'}
          {#if st.installed}
            <div class="buttons">
              <button type="button" class="k-btn k-btn--primary" onclick={() => onopenbot?.(st.botId)}>{tr('Открыть бота')}</button>
            </div>
          {:else}
            {#if item.agent?.sites?.length}
              <label class="consent">
                <input type="checkbox" bind:checked={siteConsent} />
                {tr('Разрешить боту нажимать и вводить текст на сайтах: {sites}', { sites: item.agent.sites.join(', ') })}
              </label>
            {/if}
            <div class="buttons">
              <button type="button" class="k-btn k-btn--primary" disabled={busy} onclick={addAgent}>{tr('Добавить в команду')}</button>
            </div>
          {/if}
        {:else if item.mode === 'cabinet'}
          <div class="k-card cabinet">
            <b>{tr('Бот работает в браузере')}</b>
            <p class="quiet">{loc(item.app?.web_cabinet?.note) || tr('Вы входите сами в окне бота. Пароль и коды бот не вводит.')}</p>
            {#if st.installed}
              <div class="buttons">
                <button type="button" class="k-btn k-btn--ink" disabled={busy} onclick={() => openBrowser(st.botId)}>{tr('Открыть браузер бота')}</button>
                <button type="button" class="k-btn" onclick={() => onopenbot?.(st.botId)}>{tr('Открыть бота')}</button>
              </div>
            {:else if bots.length === 0}
              <p class="quiet">{tr('Сначала добавьте бота в команду.')}</p>
            {:else}
              <label class="k-field">
                <span class="k-label">{tr('Какой бот')}</span>
                <select class="k-input" bind:value={cabinetBot}>
                  {#each bots as bot (bot.id)}<option value={bot.id}>{bot.name}</option>{/each}
                </select>
              </label>
              <label class="consent">
                <input type="checkbox" bind:checked={cabinetConsent} />
                {tr('Разрешить этому боту нажимать и вводить текст на сайтах: {sites}', { sites: (item.app?.web_cabinet?.sites || []).join(', ') })}
              </label>
              <div class="buttons">
                <button type="button" class="k-btn k-btn--primary" disabled={busy || !cabinetConsent} onclick={addCabinet}>{tr('Подключить кабинет')}</button>
              </div>
            {/if}
          </div>
        {:else}
          {#if st.installed}
            <p class="quiet">{tr('Приложение подключено.')}</p>
          {:else}
            {#key item.id}<ConnectFlow {item} onchanged={() => { load(); onchanged?.(); }} />{/key}
          {/if}
        {/if}

        {#if st.installed || st.status === 'pending'}
          {#if confirmRemove}
            <div class="remove">
              <p>{removeWarning()}</p>
              <div class="buttons">
                <button type="button" class="k-btn k-btn--attention" disabled={busy} onclick={remove}>{tr('Удалить')}</button>
                <button type="button" class="k-btn k-btn--ghost" onclick={() => (confirmRemove = false)}>{tr('Отмена')}</button>
              </div>
            </div>
          {:else}
            <button type="button" class="k-btn k-btn--ghost k-btn--sm remove-btn" onclick={() => (confirmRemove = true)}>{tr('Удалить')}</button>
          {/if}
        {/if}
      {/if}

      {#if note}<p class="ok" role="status">{note}</p>{/if}
      {#if error}<p class="k-hint is-error" role="alert">{error}</p>{/if}
    </section>
  {/if}
</aside>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.18);
  }
  .cat-drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 41;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-4, 16px);
    width: min(440px, 100vw);
    padding: var(--k-space-4, 16px) var(--k-space-6, 24px) var(--k-space-8, 32px);
    overflow-y: auto;
    background: var(--k-surface, #fff);
    color: var(--k-text, inherit);
    box-shadow: var(--k-shadow-3, -8px 0 32px rgba(0, 0, 0, 0.2));
    font-family: var(--k-font-body, inherit);
    animation: slide var(--k-dur-base, 220ms) var(--k-ease-out, ease-out);
  }
  .cat-drawer:focus {
    outline: none;
  }
  @keyframes slide {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  .top {
    display: flex;
    justify-content: flex-end;
    margin-bottom: -8px;
  }
  .hero {
    display: flex;
    gap: var(--k-space-3, 12px);
    align-items: center;
  }
  .icon {
    display: grid;
    place-items: center;
    flex: none;
    width: 56px;
    height: 56px;
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-surface-2, #f0f0f0);
    font-size: 28px;
  }
  h2 {
    margin: 0;
    font: 800 var(--k-text-xl, 22px) / 1.2 var(--k-font-display, inherit);
  }
  h3 {
    margin: 0 0 6px;
    font: 600 var(--k-text-sm, 13px) / 1.3 var(--k-font-body, inherit);
    color: var(--k-text-2, inherit);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 4px 0 0;
    color: var(--k-text-3, inherit);
    font-size: var(--k-text-xs, 12px);
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
  .desc {
    margin: 0;
    font-size: var(--k-text-md, 15px);
    line-height: var(--k-leading-body, 1.5);
  }
  section p {
    margin: 0;
    font-size: var(--k-text-sm, 13px);
  }
  .needs,
  .skills {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--k-text-sm, 13px);
  }
  .skills li {
    display: flex;
    gap: 8px;
  }
  .needs li::before {
    content: '· ';
    color: var(--k-text-3, inherit);
  }
  .mini {
    flex: none;
  }
  .body pre {
    max-height: 240px;
    overflow: auto;
    margin: 8px 0 0;
    padding: var(--k-space-2, 8px) var(--k-space-3, 12px);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface-2, #f0f0f0);
    white-space: pre-wrap;
    font: 400 var(--k-text-xs, 12px) / 1.5 var(--k-font-mono, monospace);
  }
  .body summary {
    cursor: pointer;
    font-size: var(--k-text-sm, 13px);
    color: var(--k-text-2, inherit);
  }
  .license a {
    font-size: var(--k-text-sm, 13px);
    color: var(--k-brand-text, #0250e0);
  }
  .act {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding-top: var(--k-space-4, 16px);
    border-top: 1px solid var(--k-line, #e8e8e8);
  }
  .bots {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cat-bot,
  .consent {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: var(--k-text-sm, 13px);
  }
  .consent {
    align-items: flex-start;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: none;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
  .update {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: var(--k-space-2, 8px) var(--k-space-3, 12px);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-brand-tint, #e7efff);
    font-size: var(--k-text-sm, 13px);
  }
  .cabinet {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
  }
  .conflict {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--k-text-sm, 13px);
  }
  .remove {
    padding: var(--k-space-3, 12px);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-attention-tint, #ffe8e2);
  }
  .remove p {
    margin: 0 0 8px;
  }
  .remove-btn {
    align-self: flex-start;
  }
  .ok {
    color: var(--k-text-2, inherit);
  }
  .quiet {
    color: var(--k-text-3, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  @media (prefers-reduced-motion: reduce) {
    .cat-drawer {
      animation: none;
    }
  }
</style>
