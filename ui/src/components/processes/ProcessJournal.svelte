<script>
  // «Журнал» процессов: что боты сделали без тебя (GET /api/journal). Важное — карточками, тихие проверки — одной строкой.
  // «Запустить снова» у запуска, который ждал решения, идёт тем же запуском процесса, что и кнопка на карточке.
  import { tr, trServer } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { whenText } from '../workflows/Labels.svelte';
  import { Button, EmptyState, Skeleton, Tag } from '../kit';

  /** onrerun(processId) — запустить процесс снова; список читает сам. */
  let { api = realApi, onrerun } = $props();

  const STATUS = {
    ok: tr('готово'),
    quiet: tr('тихо: сообщать нечего'),
    needs: tr('ждёт решения'),
    failed: tr('ошибка'),
    stopped: tr('остановлено'),
    limit: tr('упёрлось в предел токенов'),
    paused: tr('на паузе'),
    skipped: tr('пропущено: компьютер спал'),
    no_mail: tr('нет новых писем'),
    no_change: tr('без изменений'),
    error: tr('не запустилась'),
    running: tr('идёт'),
    busy: tr('бот был занят'),
  };
  const REASON = { network: tr('сеть'), login: tr('нужен вход'), check: tr('проверка не прошла'), stuck: tr('застрял') };
  const BAD = ['failed', 'error', 'limit', 'needs'];

  let entries = $state.raw([]);
  let usage = $state.raw(null);
  let limit = $state(0);
  let loading = $state(true);
  let quietOpen = $state(false);

  const quiet = $derived(entries.filter((e) => e.status === 'quiet'));
  const important = $derived(entries.filter((e) => e.status !== 'quiet').slice(0, 30));
  const number = (n) => Number(n || 0).toLocaleString();
  const status = (e) => [STATUS[e.status], REASON[e.reason]].filter(Boolean).join(': ');
  const usageText = $derived.by(() => {
    if (!usage) return '';
    const spent = `${usage.estimated ? tr('примерно ') : ''}${number(usage.tokens)}`;
    const cache = Number(usage.cacheRate) > 0 ? ` · ${tr('из кэша {n}%', { n: number(usage.cacheRate) })}` : '';
    const total = limit > 0 ? tr('Сегодня потрачено {spent} токенов из {limit}', { spent, limit: number(limit) }) : tr('Сегодня потрачено {spent} токенов', { spent });
    return `${total}${cache}`;
  });

  $effect(() => {
    let alive = true;
    api('/api/journal?limit=200')
      .then((data) => {
        if (!alive) return;
        entries = data.entries || [];
        usage = data.usage || null;
        limit = Number(data.limit) || 0;
      })
      .catch((err) => toast(trServer(err?.message || tr('Не получилось'))))
      .finally(() => alive && (loading = false));
    return () => {
      alive = false;
    };
  });
</script>

<div class="pj">
  {#if loading}
    <Skeleton height="72px" />
  {:else if !entries.length}
    <EmptyState title={tr('Пока тихо')} text={tr('Здесь появится всё, что боты сделали без тебя.')} />
  {:else}
    {#if usageText}<p class="pj-usage">{usageText}</p>{/if}
    {#each important as e (e.id ?? `${e.at}:${e.routineId}`)}
      <article class="pj-row">
        <header class="pj-head">
          <span class="pj-dim">{whenText(e.at)}</span>
          <span class="pj-dim">{e.botName}</span>
          <b class="pj-title">{trServer(e.title)}</b>
          <Tag tone={BAD.includes(e.status) ? 'danger' : 'neutral'}>{status(e)}</Tag>
        </header>
        {#if e.text}<p class="pj-text">{trServer(e.text)}</p>{/if}
        {#if e.fallback}<p class="pj-dim">{tr('на запасной: {name}', { name: e.fallback })}</p>{/if}
        {#if Number(e.tokens) > 0}<p class="pj-dim">{tr('{n} ток.', { n: number(e.tokens) })}</p>{/if}
        {#if e.missed?.length}
          <div class="pj-missed">
            <b>{tr('Ждёт твоего решения:')}</b>
            <ul>{#each e.missed as m}<li>{m}</li>{/each}</ul>
            {#if e.routineId}<Button size="sm" variant="ghost" onclick={() => onrerun?.(e.routineId)}>{tr('Запустить снова')}</Button>{/if}
          </div>
        {/if}
      </article>
    {/each}
    {#if quiet.length}
      <Button class="pj-quiet" size="sm" variant="ghost" iconName={quietOpen ? 'chevron-down' : 'chevron-right'} aria-expanded={quietOpen} onclick={() => (quietOpen = !quietOpen)}>
        {tr('Тихих проверок, сообщать нечего: {count}', { count: quiet.length })}
      </Button>
      {#if quietOpen}
        {#each quiet as e (e.id ?? `${e.at}:${e.routineId}`)}
          <p class="pj-quiet-row"><span class="pj-dim">{whenText(e.at)}</span> <span class="pj-dim">{e.botName}</span> {trServer(e.title)}</p>
        {/each}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .pj {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
  }
  .pj-usage,
  .pj-text,
  .pj-dim,
  .pj-quiet-row {
    margin: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .pj-usage,
  .pj-dim {
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .pj-row {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-3) var(--k-space-4);
    border-radius: var(--k-radius-lg);
    box-shadow: 0 0 0 1px var(--k-line);
  }
  .pj-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--k-space-1) var(--k-space-2);
  }
  .pj-title {
    font: 600 var(--k-text-md) / 1.3 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .pj-text {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--k-text-2);
  }
  .pj-missed {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--k-space-1);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .pj-missed ul {
    margin: 0;
    padding-left: var(--k-space-4);
  }
  .pj :global(.pj-quiet) {
    align-self: flex-start;
  }
</style>
