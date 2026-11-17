<script>
  // Экран «Отчёт»: итог недели по всем ботам. GET /api/report?week=ГГГГ-ММ-ДД (понедельник).
  // Пока сервер отвечает 404 (маршрут не выпущен), показываем демо-данные с плашкой «Демо».
  import { api as defaultApi, lang, serverText } from '../../lib/api.js';
  import { tr } from '../../lib/i18n.js';
  import {
    addDays,
    demoReport,
    formatNum,
    formatSeconds,
    formatTs,
    mondayOf,
    ruForm,
    shortDay,
    toneOf,
    weekLabel,
  } from '../../lib/report.js';
import { Button, EmptyState, Skeleton, Icon, IconButton } from '../kit';

  let { api = defaultApi, today = mondayOf() } = $props();

  let data = $state(null);
  let demo = $state(false);
  let loading = $state(true);
  let error = $state('');
  let open = $state({});
  let seq = 0;

  const locale = $derived(lang());
  const isCurrent = $derived(!!data && data.weekStart >= today);

  async function load(week) {
    const mine = ++seq;
    loading = true;
    error = '';
    try {
      const res = await api('/api/report' + (week ? '?week=' + encodeURIComponent(week) : ''));
      if (mine !== seq) return;
      data = res;
      demo = false;
    } catch (err) {
      if (mine !== seq) return;
      if (err?.status === 404) {
        data = demoReport(week || today);
        demo = true;
      } else {
        error = serverText(err?.data?.message || err?.message) || tr('Не удалось выполнить запрос');
      }
    }
    open = {};
    loading = false;
  }

  $effect(() => {
    load('');
    return () => {
      seq += 1;
    };
  });

  const go = (week) => load(week);

  const tone = $derived(data ? toneOf(data.replies, data.okPercent) : 'empty');

  function repliesText(n) {
    const f = lang() === 'en' ? (Math.abs(n) === 1 ? 0 : 2) : ruForm(n);
    if (f === 0) return tr('{n} ответ', { n });
    if (f === 1) return tr('{n} ответа', { n });
    return tr('{n} ответов', { n });
  }

  // После «из»: родительный падеж («из 1 ответа», «из 3 ответов», «из 21 ответа»).
  function repliesOf(n) {
    if (lang() === 'en') return repliesText(n);
    return ruForm(n) === 0 ? tr('{n} ответа', { n }) : tr('{n} ответов', { n });
  }

  const title = $derived(
    tone === 'great' ? tr('Отличная неделя') : tone === 'good' ? tr('Хорошая неделя') : tone === 'rough' ? tr('Тяжёлая неделя') : tr('Тихая неделя'),
  );

  const headline = $derived(
    data && data.replies ? tr('{title}: {pct}% из {replies} закончились хорошо', { title, pct: data.okPercent, replies: repliesOf(data.replies) }) : title,
  );

  const subtitle = $derived.by(() => {
    if (!data || !data.replies) return '';
    if (isCurrent) return tr('Неделя ещё идёт: итог изменится');
    const weeks = data.weeks || [];
    const prev = weeks.length > 1 ? weeks[weeks.length - 2] : null;
    const prevTotal = prev ? prev.ok + prev.failed : 0;
    if (!prevTotal) return tr('Первая неделя с данными: сравнивать пока не с чем');
    return tr('Неделей раньше: {pct}% из {replies}', { pct: Math.round((prev.ok / prevTotal) * 100), replies: repliesOf(prevTotal) });
  });

  const maxTotal = $derived(Math.max(1, ...(data?.weeks || []).map((w) => w.ok + w.failed)));

  function barTitle(w) {
    return tr('{day}: удачно {ok}, с ошибкой {failed}', { day: shortDay(w.start, locale), ok: w.ok, failed: w.failed });
  }

  function toggle(id) {
    open = { ...open, [id]: !open[id] };
  }
</script>

<section class="rp" aria-busy={loading}>
  <div class="rp-nav">
    <div class="rp-nav-group">
      <IconButton class="rp-step" size="sm" name="chevron-left" label={tr('Прошлая неделя')} disabled={!data || loading} onclick={() => go(addDays(data.weekStart, -7))} />
      <span class="rp-range" aria-live="polite">{data ? weekLabel(data.weekStart, data.weekEnd, locale) : ''}</span>
      <IconButton class="rp-step" size="sm" name="chevron-right" label={tr('Следующая неделя')} disabled={!data || loading || isCurrent} onclick={() => go(addDays(data.weekStart, 7))} />
    </div>
    <Button variant="ghost" size="sm" disabled={!data || loading || isCurrent} onclick={() => go('')}>{tr('Сегодня')}</Button>
  </div>

  {#if demo}
    <p class="rp-demo">{tr('Демо: отчёты на сервере ещё не включены, цифры ненастоящие')}</p>
  {/if}

  {#if error}
    <div class="rp-error" role="alert">
      <span>{error}</span>
      <Button variant="ghost" size="sm" onclick={() => go('')}>{tr('Повторить')}</Button>
    </div>
  {:else if !data}
    <div class="rp-skel"><Skeleton height="28px" /><Skeleton height="96px" /><Skeleton height="64px" /></div>
  {:else}
    <div class="rp-body" class:rp-dim={loading}>
      <h2 class="rp-headline rp-tone-{tone}">{headline}</h2>
      {#if subtitle}<p class="rp-sub">{subtitle}</p>{/if}

      {#if !data.replies}
        <EmptyState title={tr('На этой неделе ответов не было')} text={tr('Когда боты поработают, здесь появится итог недели.')} />
      {:else}
        <div class="rp-chart" role="group" aria-label={tr('Последние недели: удачные и неудачные ответы')}>
          {#each data.weeks as w, i (w.start)}
            {@const total = w.ok + w.failed}
            <Button variant="ghost" class="rp-col{i === data.weeks.length - 1 ? ' rp-cur' : ''}" title={barTitle(w)} aria-label={barTitle(w)} disabled={loading} onclick={() => go(w.start)}>
              <span class="rp-bar">
                {#if total}
                  <span class="rp-seg rp-fail" style:height="{(w.failed / maxTotal) * 100}%"></span>
                  <span class="rp-seg rp-ok" style:height="{(w.ok / maxTotal) * 100}%"></span>
                {:else}
                  <span class="rp-none"></span>
                {/if}
              </span>
              <span class="rp-day">{shortDay(w.start, locale)}</span>
            </Button>
          {/each}
        </div>
        <div class="rp-legend" aria-hidden="true">
          <span><i class="rp-dot rp-ok"></i>{tr('Удачно')}</span>
          <span><i class="rp-dot rp-fail"></i>{tr('С ошибкой')}</span>
        </div>

        <h3 class="rp-h3">{tr('По ботам')}</h3>
        <ul class="rp-bots">
          {#each data.bots as bot (bot.id)}
            {@const btone = toneOf(bot.replies, bot.okPercent)}
            {@const fails = bot.failures || []}
            <li class="rp-bot">
              <div class="rp-row">
                <div class="rp-who">
                  <span class="rp-name">{bot.name}</span>
                  <span class="rp-meta">
                    {repliesText(bot.replies)} · {tr('~{n} ток.', { n: formatNum(bot.avgTokens, locale) })} · {tr('{s} с', { s: formatSeconds(bot.avgSeconds, locale) })}
                  </span>
                </div>
                <span class="rp-pct rp-tone-{btone}">{Math.round(bot.okPercent)}%</span>
                {#if fails.length}
                  <Button variant="ghost" size="sm" class="rp-more" aria-expanded={!!open[bot.id]} aria-controls="rp-f-{bot.id}" onclick={() => toggle(bot.id)}>
                    {tr('Ошибки: {n}', { n: fails.length })}
                    <Icon name="chevron-down" size={14} class={['rp-chev', open[bot.id] && 'rp-turn']} />
                  </Button>
                {/if}
              </div>
              {#if fails.length && open[bot.id]}
                <ul class="rp-fails" id="rp-f-{bot.id}">
                  {#each fails as f}
                    <li><time class="rp-time">{formatTs(f.ts, locale)}</time><span class="rp-prev">{f.preview}</span></li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .rp { display: flex; flex-direction: column; gap: var(--k-space-3, 12px); color: var(--k-text, var(--color-text-primary)); min-width: 0; }
  .rp-nav { display: flex; align-items: center; justify-content: space-between; gap: var(--k-space-2, 8px); flex-wrap: wrap; }
  .rp-nav-group { display: flex; align-items: center; gap: var(--k-space-2, 8px); }
  .rp-range { font: 700 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, inherit); min-width: 9ch; text-align: center; }
  .rp-col:focus-visible, .rp-more:focus-visible { outline: none; box-shadow: var(--k-focus); }
  .rp-demo { margin: 0; padding: var(--k-space-2, 8px) var(--k-space-3, 12px); border-radius: var(--k-radius-sm, 12px); background: var(--k-attention-tint); color: var(--k-attention-text); font: 600 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, inherit); }
  .rp-error { display: flex; align-items: center; justify-content: space-between; gap: var(--k-space-3, 12px); padding: var(--k-space-3, 12px); border-radius: var(--k-radius-sm, 12px); background: var(--k-attention-tint); color: var(--k-attention-text); font-size: var(--k-text-sm, 13px); }
  .rp-skel { display: grid; gap: var(--k-space-3, 12px); }
  .rp-body { display: flex; flex-direction: column; gap: var(--k-space-2, 8px); transition: opacity 0.15s; }
  .rp-dim { opacity: 0.55; }
  .rp-headline { margin: var(--k-space-2, 8px) 0 0; font: 800 var(--k-text-xl, 22px) / 1.2 var(--k-font-display, inherit); text-wrap: balance; }
  .rp-sub { margin: 0; color: var(--k-text-3); font-size: var(--k-text-sm, 13px); }
  .rp-tone-rough { color: var(--k-attention-text); }
  .rp-chart { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: var(--k-space-2, 8px); margin-top: var(--k-space-3, 12px); padding: var(--k-space-4, 16px) var(--k-space-4, 16px) var(--k-space-2, 8px); border-radius: var(--k-radius-md, 18px); background: var(--k-surface-2); }
  :global(.rp-col) { display: flex; flex-direction: column; align-items: center; gap: var(--k-space-1); height: auto; padding: 0; }
  .rp-bar { display: flex; flex-direction: column; justify-content: flex-end; width: 100%; max-width: 36px; height: 88px; }
  .rp-seg { display: block; width: 100%; min-height: 3px; }
  .rp-seg:first-child { border-radius: var(--k-radius-xs, 6px) var(--k-radius-xs, 6px) 0 0; }
  .rp-seg:last-child { border-radius: 0 0 var(--k-radius-xs, 6px) var(--k-radius-xs, 6px); }
  .rp-seg:only-child { border-radius: var(--k-radius-xs, 6px); }
  .rp-ok { background: var(--k-brand); }
  .rp-fail { background: var(--k-attention); }
  :global(.rp-col:not(.rp-cur) .rp-bar) { opacity: 0.45; }
  :global(.rp-col:hover:not(:disabled) .rp-bar) { opacity: 1; }
  .rp-none { display: block; width: 100%; height: 3px; border-radius: var(--k-radius-xs); background: var(--k-line-strong); }
  .rp-day { font: 600 var(--k-text-micro, 11px) / 1.2 var(--k-font-body, inherit); white-space: nowrap; }
  :global(.rp-cur .rp-day) { color: var(--k-text); }
  .rp-legend { display: flex; gap: var(--k-space-4, 16px); color: var(--k-text-3); font-size: var(--k-text-xs, 12px); }
  .rp-legend span { display: inline-flex; align-items: center; gap: 6px; }
  .rp-dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
  .rp-h3 { margin: var(--k-space-4, 16px) 0 0; font: 800 var(--k-text-lg, 18px) / 1.25 var(--k-font-display, inherit); }
  .rp-bots { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--k-space-2, 8px); }
  .rp-bot { border-radius: var(--k-radius-sm, 12px); background: var(--k-surface-2); }
  .rp-row { display: flex; align-items: center; gap: var(--k-space-3, 12px); padding: var(--k-space-3, 12px) var(--k-space-4, 16px); flex-wrap: wrap; }
  .rp-who { flex: 1 1 160px; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rp-name { font: 700 var(--k-text-md, 15px) / 1.25 var(--k-font-body, inherit); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rp-meta { color: var(--k-text-3); font-size: var(--k-text-xs, 12px); }
  .rp-pct { font: 800 var(--k-text-lg, 18px) / 1 var(--k-font-display, inherit); font-variant-numeric: tabular-nums; }
  :global(.rp-more) { background: var(--k-attention-tint); color: var(--k-attention-text); }
  :global(.rp-chev) { transition: transform 0.15s; }
  :global(.rp-turn) { transform: rotate(180deg); }
  .rp-fails { list-style: none; margin: 0; padding: 0 var(--k-space-4, 16px) var(--k-space-3, 12px); display: flex; flex-direction: column; gap: var(--k-space-2, 8px); }
  .rp-fails li { display: flex; gap: var(--k-space-3, 12px); align-items: baseline; font-size: var(--k-text-sm, 13px); }
  .rp-time { flex: none; color: var(--k-text-3); font-size: var(--k-text-xs, 12px); font-variant-numeric: tabular-nums; }
  .rp-prev { min-width: 0; overflow-wrap: anywhere; color: var(--k-text-2, inherit); }
  @media (max-width: 480px) {
    .rp-chart { padding-inline: var(--k-space-2, 8px); gap: var(--k-space-1, 4px); }
    .rp-fails li { flex-direction: column; gap: 2px; }
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.rp-chev), .rp-body { transition: none; }
  }
</style>
