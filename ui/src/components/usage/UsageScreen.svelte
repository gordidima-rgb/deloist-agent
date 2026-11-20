<script>
  // Экран «Использование и лимиты»: расход токенов по месяцам, по ботам и моделям, «за кулисами»,
  // последние ответы и лимиты подписок. Данные: GET /api/usage?month=YYYY-MM и /api/providers/limits.
  import { tr } from '../../lib/i18n.js';
  import { Button, IconButton } from '../kit';
  import { api as defaultApi, lang } from '../../lib/api.js';
  import {
    monthKey, shiftMonth, monthLabel, fmtTokens, fmtPct, fmtSeconds, pct, paceForecast,
    colorMap, dayRows, validUsage, resetLine,
  } from '../../lib/usage.js';
  import { usageFixture } from '../../lib/usage.fixture.js';
  import { ruForm } from '../../lib/report.js';

  let { api = defaultApi, onOpen = null, today = new Date() } = $props();

  const thisMonth = monthKey(today);
  let month = $state(thisMonth);
  let data = $state.raw(null);
  let demo = $state(false);
  let failed = $state(false);
  let loading = $state(true);
  let subs = $state.raw([]);
  let subsDone = $state(false);

  const locale = $derived(lang() === 'en' ? 'en-US' : 'ru-RU');
  const num = (n) => fmtTokens(n, locale);

  // Каждый запрос помнит свой номер: быстрый переклик месяцев не даёт старому ответу затереть новый.
  let seq = 0;
  async function load(m) {
    const mine = (seq += 1);
    loading = true;
    failed = false;
    let next = null;
    let isDemo = false;
    try {
      next = await api(`/api/usage?month=${m}`);
      if (!validUsage(next)) throw Object.assign(new Error('bad shape'), { status: 0 });
    } catch (err) {
      if (err?.status === 404) {
        next = usageFixture(m);
        isDemo = true;
      } else {
        failed = true;
      }
    }
    if (mine !== seq) return;
    data = next;
    demo = isDemo;
    loading = false;
  }
  $effect(() => {
    load(month);
  });

  // ---- Лимиты подписок: как в левой панели (web/providers.js), но подряд и с подписями ----
  function windows(limits) {
    const list = (limits || []).map((l) => ({ ...l, left: Number(l.left) }));
    const hours = list.find((l) => /5\s*h/i.test(l.label));
    const week = list.find((l) => /weekly/i.test(l.label) && !/·/.test(l.label)) || list.find((l) => /weekly/i.test(l.label)) || list.find((l) => /monthly/i.test(l.label));
    return [hours, week, ...list.filter((l) => l !== hours && l !== week)].filter(Boolean);
  }
  function limitLabel(label) {
    const text = String(label || '').replace(/^codex\s+/i, '').replace(/\s*limit$/i, '').trim();
    const hours = /^(\d+)\s*h$/i.exec(text);
    if (hours) return tr('{n} ч', { n: hours[1] });
    if (/^weekly$/i.test(text)) return tr('Неделя');
    if (/^daily$/i.test(text)) return tr('День');
    if (/^monthly$/i.test(text)) return tr('Месяц');
    const scoped = /^weekly(?:\s+limit)?\s*·\s*(.+)$/i.exec(text);
    if (scoped) return tr('Неделя · {m}', { m: scoped[1] });
    return text;
  }
  async function loadSubs() {
    try {
      const [limits, list] = await Promise.all([api('/api/providers/limits'), api('/api/providers').catch(() => ({ providers: [] }))]);
      const agents = limits.providers || [];
      const items = [];
      if (limits.deloist) {
        const w = limits.deloist;
        const start = Math.max(w.startKop, w.nowKop, 0);
        items.push({ id: 'deloist', name: 'Deloist', rows: [{ label: tr('Баланс'), left: start > 0 ? (Math.max(w.nowKop, 0) / start) * 100 : 0, resetsAt: null }] });
      }
      for (const p of list.providers || []) {
        if (p.enabled === false || p.pickerOnly) continue;
        if (p.kind === 'acp') {
          for (const a of agents) {
            if (a.id === p.id || a.id.startsWith(`${p.id}@`)) items.push({ id: a.id, name: a.name || p.name, rows: windows(a.limits).map((l) => ({ label: limitLabel(l.label), left: l.left, resetsAt: l.resetsAt, spent: l.spent, cap: l.cap, currency: l.currency })) });
          }
        }
      }
      subs = items;
    } catch {
      subs = [];
    }
    subsDone = true;
  }
  $effect(() => {
    loadSubs();
  });

  // ---- Производные величины ----
  const sortedBots = $derived([...(data?.bots || [])].sort((a, b) => b.tokens - a.tokens));
  const colors = $derived(colorMap(sortedBots.map((b) => b.id)));
  const rows = $derived(data ? dayRows(month, data.days) : []);
  const maxDay = $derived(Math.max(1, ...rows.map((r) => r.sum)));
  const empty = $derived(!!data && data.totalTokens === 0 && (data.replies || 0) === 0);
  const modelsTotal = $derived((data?.models || []).reduce((a, m) => a + m.tokens, 0));
  const behind = $derived(data?.behind || { router: 0, memory: 0, helper: 0 });
  const behindRows = $derived([
    { key: 'router', label: tr('Маршрутизатор'), hint: tr('Выбирает, какому боту отдать сообщение'), tokens: behind.router },
    { key: 'memory', label: tr('Память'), hint: tr('Запоминает и вспоминает факты о вас'), tokens: behind.memory },
    { key: 'helper', label: tr('Помощник'), hint: tr('Заголовки, сводки и мелкие служебные запросы'), tokens: behind.helper },
  ]);
  const MODEL_COLORS = $derived(colorMap((data?.models || []).map((m) => m.id)));

  // «1 ответ», «3 ответа», «5 ответов»: те же формы и ключи перевода, что на экране «Отчёт».
  function repliesText(n) {
    const f = lang() === 'en' ? (Math.abs(n) === 1 ? 0 : 2) : ruForm(n);
    return f === 0 ? tr('{n} ответ', { n }) : f === 1 ? tr('{n} ответа', { n }) : tr('{n} ответов', { n });
  }

  function whereLabel(w) {
    const known = {
      chat: tr('Чат'),
      telegram: tr('Telegram'),
      discord: tr('Discord'),
      slack: tr('Slack'),
      routine: tr('Рутина'),
      room: tr('Команда'),
    };
    return known[w] || String(w || '');
  }

  function when(ts) {
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return '';
    return new Intl.DateTimeFormat(locale, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }).format(d);
  }

  function fmtAmount(l) {
    if (!Number.isFinite(l?.spent) || !Number.isFinite(l?.cap) || l.cap <= 0 || !l.currency) return '';
    const fmt = (v) => v.toLocaleString(locale, { style: 'currency', currency: l.currency, minimumFractionDigits: 2, maximumFractionDigits: 2 });
    return tr('{spent} из {cap}', { spent: fmt(l.spent), cap: fmt(l.cap) });
  }
  const canNext = $derived(month < thisMonth);
</script>

<div class="us-root">
  <header class="us-head">
    <h2 class="us-title">{tr('Использование и лимиты')}</h2>
    <div class="us-months" role="group" aria-label={tr('Месяц')}>
      <IconButton name="chevron-left" size="sm" class="us-nav" label={tr('Предыдущий месяц')} onclick={() => (month = shiftMonth(month, -1))} />
      <span class="us-month" aria-live="polite">{monthLabel(month, locale)}</span>
      <IconButton name="chevron-right" size="sm" class="us-nav" label={tr('Следующий месяц')} disabled={!canNext} onclick={() => (month = shiftMonth(month, 1))} />
    </div>
  </header>

  {#if demo}
    <p class="us-demo" role="status">{tr('Пример данных: сервер пока не отдаёт расход, цифры ниже не настоящие.')}</p>
  {/if}

  {#if failed}
    <div class="us-card us-state" role="alert">
      <p>{tr('Не удалось загрузить расход.')}</p>
      <Button variant="primary" size="sm" onclick={() => load(month)}>{tr('Повторить')}</Button>
    </div>
  {:else if !data}
    <div class="us-card us-state" aria-busy="true">{tr('Загружаю…')}</div>
  {:else if empty}
    <div class="us-card us-state us-empty">
      <b>{tr('В этом месяце расхода нет')}</b>
      <p>{tr('Как только боты начнут отвечать, здесь появятся токены, модели и последние ответы.')}</p>
    </div>
  {:else}
    <section class="us-card us-hero" class:us-fade={loading} aria-label={tr('Итоги месяца')}>
      <div class="us-big">
        <span class="us-num">{num(data.totalTokens)}</span>
        <span class="us-cap">{tr('токенов за месяц')}</span>
      </div>
      <div class="us-kpi">
        <div><b>{new Intl.NumberFormat(locale).format(data.replies)}</b><span>{tr('ответов')}</span></div>
        <div><b>{fmtSeconds(data.avgSeconds)}</b><span>{tr('среднее время ответа')}</span></div>
      </div>
    </section>

    <section class="us-card" aria-label={tr('По дням')}>
      <h3 class="us-h">{tr('По дням')}</h3>
      <div class="us-chart" role="img" aria-label={tr('Расход токенов по дням, по ботам')}>
        {#each rows as r (r.date)}
          <div class="us-col" title={`${r.date}: ${num(r.sum)}`}>
            <div class="us-stack" style:height={`${(r.sum / maxDay) * 100}%`}>
              {#each sortedBots as b (b.id)}
                {#if r.byBot[b.id]}
                  <i style:flex-grow={r.byBot[b.id]} style:background={colors.get(b.id)}></i>
                {/if}
              {/each}
            </div>
            <span class="us-tick">{r.day % 5 === 0 || r.day === 1 ? r.day : ''}</span>
          </div>
        {/each}
      </div>
      <ul class="us-legend">
        {#each sortedBots as b (b.id)}
          <li><i style:background={colors.get(b.id)}></i>{b.name}</li>
        {/each}
      </ul>
    </section>

    <section class="us-card" aria-label={tr('По агентам')}>
      <h3 class="us-h">{tr('По агентам')}</h3>
      {#each sortedBots as b (b.id)}
        {@const limited = b.monthlyLimit != null && b.monthlyLimit > 0}
        {@const used = limited ? pct(b.tokens, b.monthlyLimit) : 0}
        {@const forecast = limited ? paceForecast({ tokens: b.tokens, limit: b.monthlyLimit, month, today }) : null}
        <div class="us-row">
          <div class="us-rowhead">
            <span class="us-name"><i style:background={colors.get(b.id)}></i>{b.name}</span>
            <span class="us-val">
              {#if limited}{num(b.tokens)} / {num(b.monthlyLimit)}{:else}{num(b.tokens)}{/if}
            </span>
          </div>
          {#if limited}
            <div class="us-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.min(100, Math.round(used))} aria-label={b.name}>
              <i class:us-over={used >= 100} style:width={`${Math.min(100, used)}%`} style:background={colors.get(b.id)}></i>
            </div>
            <p class="us-sub" class:us-warn={forecast != null && forecast >= 100}>
              {fmtPct(used)} {tr('лимита')}
              {#if forecast != null}· {tr('При таком темпе к концу месяца: {p}', { p: fmtPct(forecast) })}{/if}
            </p>
          {:else}
            <p class="us-sub">{repliesText(b.replies)}{#if b.model} · {b.model}{/if}</p>
          {/if}
        </div>
      {/each}
    </section>

    <section class="us-card" aria-label={tr('По моделям')}>
      <h3 class="us-h">{tr('По моделям')}</h3>
      <div class="us-split" role="img" aria-label={tr('Доли моделей')}>
        {#each data.models as m (m.id)}
          <i style:flex-grow={m.tokens} style:background={MODEL_COLORS.get(m.id)} title={m.id}></i>
        {/each}
      </div>
      <ul class="us-list">
        {#each data.models as m (m.id)}
          <li>
            <span class="us-name"><i style:background={MODEL_COLORS.get(m.id)}></i>{m.id}<small>{m.provider}</small></span>
            <span class="us-val">{num(m.tokens)} · {fmtPct(pct(m.tokens, modelsTotal))}</span>
          </li>
        {/each}
      </ul>
    </section>

    <section class="us-card" aria-label={tr('За кулисами')}>
      <h3 class="us-h">{tr('За кулисами')}</h3>
      <p class="us-note">{tr('Служебные запросы, которые идут не от бота, а от самой программы.')}</p>
      <ul class="us-list">
        {#each behindRows as r (r.key)}
          <li>
            <span class="us-name">{r.label}<small>{r.hint}</small></span>
            <span class="us-val">{num(r.tokens)} · {fmtPct(pct(r.tokens, data.totalTokens))}</span>
          </li>
        {/each}
      </ul>
    </section>

    <section class="us-card" aria-label={tr('Последние ответы')}>
      <h3 class="us-h">{tr('Последние ответы')}</h3>
      {#if !data.receipts.length}
        <p class="us-note">{tr('Ответов пока нет.')}</p>
      {:else}
        <div class="us-scroll">
          <table class="us-table">
            <thead>
              <tr><th>{tr('Ответ')}</th><th>{tr('Где')}</th><th>{tr('Модель')}</th><th class="us-r">{tr('Время')}</th><th class="us-r">{tr('Токены')}</th><th class="us-r">{tr('Когда')}</th></tr>
            </thead>
            <tbody>
              {#each data.receipts as r (r.id)}
                <tr class:us-failed={r.outcome && r.outcome !== 'ok'}>
                  <td class="us-prev">
                    {#if onOpen && r.threadId}
                      <Button variant="ghost" size="sm" class="us-link" onclick={() => onOpen(r)}><b>{r.botName}</b> {r.preview}</Button>
                    {:else}
                      <b>{r.botName}</b> {r.preview}
                    {/if}
                  </td>
                  <td>{whereLabel(r.where)}</td>
                  <td>{r.model}</td>
                  <td class="us-r">{fmtSeconds(r.seconds)}</td>
                  <td class="us-r">{num(r.tokens)}</td>
                  <td class="us-r">{when(r.ts)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  {/if}

  <section class="us-card" aria-label={tr('Лимиты подписок')}>
    <h3 class="us-h">{tr('Лимиты подписок')}</h3>
    {#if !subsDone}
      <p class="us-note">{tr('Загружаю…')}</p>
    {:else if !subs.length}
      <p class="us-note">{tr('Подключённых подписок с лимитами нет.')}</p>
    {:else}
      <div class="us-subs">
        {#each subs as s (s.id)}
          <div class="us-sub-card">
            <b class="us-name">{s.name}</b>
            {#each s.rows as l (l.label)}
              <div class="us-subrow">
                <div class="us-rowhead"><span>{l.label}</span><span class="us-val" class:us-warn={l.left < 15}>{fmtAmount(l) || tr('осталось {p}', { p: fmtPct(l.left) })}</span></div>
                <div class="us-bar" aria-hidden="true"><i style:width={`${Math.max(0, Math.min(100, l.left))}%`} style:background={l.left < 15 ? 'var(--k-attention)' : 'var(--k-brand)'}></i></div>
                {#if resetLine(l.resetsAt)}<p class="us-sub">{resetLine(l.resetsAt)}</p>{/if}
              </div>
            {:else}
              <p class="us-sub">{tr('Этот провайдер не сообщает лимиты')}</p>
            {/each}
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .us-root { display: grid; gap: var(--k-space-3, 12px); color: var(--k-text); font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui); min-width: 0; }
  .us-head { display: flex; align-items: center; justify-content: space-between; gap: var(--k-space-3, 12px); flex-wrap: wrap; }
  .us-title { margin: 0; font: 800 var(--k-text-xl, 22px) / 1.2 var(--k-font-display, inherit); letter-spacing: -0.01em; }
  .us-months { display: flex; align-items: center; gap: var(--k-space-1, 4px); background: var(--k-surface-2); border-radius: var(--k-radius-pill, 999px); padding: var(--k-space-1); }
  .us-month { min-width: 128px; text-align: center; font-weight: 600; }
  .us-demo { margin: 0; padding: var(--k-space-2, 8px) var(--k-space-3, 12px); border-radius: var(--k-radius-sm, 12px); background: var(--k-attention-tint); color: var(--k-attention-text); font-weight: 600; }
  .us-card { background: var(--k-surface-2); border-radius: var(--k-radius-lg, 26px); padding: var(--k-space-4, 16px) var(--k-space-5, 20px); min-width: 0; }
  .us-fade { opacity: 0.55; transition: opacity 0.15s; }
  .us-state { text-align: center; color: var(--k-text-2); display: grid; gap: var(--k-space-2, 8px); justify-items: center; }
  .us-state p { margin: 0; }
  .us-h { margin: 0 0 var(--k-space-3, 12px); font: 700 var(--k-text-md, 15px) / 1.2 var(--k-font-display, inherit); }
  .us-note { margin: 0 0 var(--k-space-2, 8px); color: var(--k-text-2); }
  .us-hero { display: flex; align-items: flex-end; justify-content: space-between; gap: var(--k-space-4, 16px); flex-wrap: wrap; }
  .us-big { display: grid; }
  .us-num { font: 800 var(--k-text-3xl, 36px) / 1 var(--k-font-display, inherit); letter-spacing: -0.02em; }
  .us-cap { color: var(--k-text-2); }
  .us-kpi { display: flex; gap: var(--k-space-6, 24px); }
  .us-kpi div { display: grid; }
  .us-kpi b { font: 700 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, inherit); }
  .us-kpi span { color: var(--k-text-2); font-size: var(--k-text-xs, 12px); }

  .us-chart { display: flex; align-items: flex-end; gap: 2px; height: 150px; }
  .us-col { flex: 1 1 0; min-width: 0; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; }
  .us-stack { display: flex; flex-direction: column-reverse; min-height: 0; border-radius: var(--k-radius-xs) 3px 0 0; overflow: hidden; }
  .us-stack i { display: block; min-height: 1px; }
  .us-tick { height: 14px; font-size: 10px; color: var(--k-text-3); text-align: center; overflow: visible; white-space: nowrap; }
  .us-legend { list-style: none; margin: var(--k-space-3, 12px) 0 0; padding: 0; display: flex; flex-wrap: wrap; gap: var(--k-space-1, 4px) var(--k-space-4, 16px); }
  .us-legend li, .us-name { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .us-legend i, .us-name i { width: 10px; height: 10px; border-radius: var(--k-radius-xs); flex: none; }

  .us-row { padding: var(--k-space-2, 8px) 0; }
  .us-row + .us-row { border-top: 1px solid var(--k-line); }
  .us-rowhead { display: flex; justify-content: space-between; gap: var(--k-space-3, 12px); align-items: baseline; }
  .us-name { font-weight: 600; }
  .us-name small { margin-left: 6px; font-weight: 400; color: var(--k-text-3); }
  .us-val { font-variant-numeric: tabular-nums; white-space: nowrap; color: var(--k-text-2); }
  .us-bar { height: 8px; margin-top: var(--k-space-2); border-radius: var(--k-radius-xs); background: var(--k-surface-3); overflow: hidden; }
  .us-bar i { display: block; height: 100%; border-radius: var(--k-radius-xs); min-width: 2px; }
  .us-bar i.us-over { background: var(--k-attention) !important; }
  .us-sub { margin: 4px 0 0; color: var(--k-text-2); font-size: var(--k-text-xs, 12px); }
  .us-warn { color: var(--k-attention-text); }

  .us-split { display: flex; height: 14px; border-radius: var(--k-radius-xs); overflow: hidden; gap: 2px; background: var(--k-surface-3); }
  .us-split i { display: block; min-width: 3px; }
  .us-list { list-style: none; margin: var(--k-space-3, 12px) 0 0; padding: 0; display: grid; gap: var(--k-space-2, 8px); }
  .us-list li { display: flex; justify-content: space-between; gap: var(--k-space-3, 12px); align-items: baseline; flex-wrap: wrap; }

  .us-scroll { overflow-x: auto; }
  .us-table { width: 100%; border-collapse: collapse; min-width: 560px; }
  .us-table th { text-align: left; font-weight: 600; font-size: var(--k-text-xs, 12px); color: var(--k-text-3); padding: 4px 8px 6px 0; }
  .us-table td { padding: var(--k-space-2) var(--k-space-2) var(--k-space-2) 0; border-top: 1px solid var(--k-line); vertical-align: top; }
  .us-table .us-r { text-align: right; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .us-prev { max-width: 280px; overflow: hidden; text-overflow: ellipsis; }
  :global(.us-link) { display: block; max-width: 100%; justify-content: flex-start; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .us-failed td:first-child { color: var(--k-attention-text); }

  .us-subs { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: var(--k-space-3, 12px); }
  .us-sub-card { background: var(--k-surface); border-radius: var(--k-radius-md, 18px); padding: var(--k-space-3, 12px) var(--k-space-4, 16px); display: grid; gap: var(--k-space-2, 8px); align-content: start; }
  .us-subrow .us-bar { margin-top: 4px; }

  @media (max-width: 520px) {
    .us-card { padding: var(--k-space-3, 12px); }
    .us-num { font-size: var(--k-text-2xl); }
    .us-chart { height: 120px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .us-fade { transition: none; }
  }
</style>
