<script>
  // Список процессов карточками: вкладка «Процессы» раздела и вкладка «Рутины» панели бота (botId — только его процессы).
  // Перечитывает список по событию окна {type:"workflows"}. Все запросы идут через api (в тестах подменяется).
  import { untrack } from 'svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { starterGraph } from '../../lib/workflows/graph.js';
  import { Button, EmptyState, Skeleton } from '../kit';
  import ProcessCard from './ProcessCard.svelte';
  import NewProcess from './NewProcess.svelte';

  /** bots [{id,name}]; botId — показать процессы одного бота; onopen(id) открывает холст; lead — подпись над списком. */
  let { api = realApi, bots = [], botId = '', onopen, lead = '' } = $props();

  const request = (path, opts) => untrack(() => api)(path, opts);
  let items = $state.raw([]);
  let loading = $state(true);
  let adding = $state(false);

  const shown = $derived(botId ? items.filter((i) => i.card?.bot_id === botId) : items);
  const fail = (err) => toast(trServer(err?.message || tr('Не получилось')));

  async function load() {
    try {
      items = (await request('/api/workflows')).workflows || [];
    } catch (err) {
      fail(err);
    } finally {
      loading = false;
    }
  }

  // Карточка меняется сразу, а при отказе сервера возвращается как была.
  async function change(id, patch, body) {
    const before = items;
    items = items.map((i) => (i.id === id ? patch(i) : i));
    try {
      await request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'PATCH', body });
    } catch (err) {
      items = before;
      fail(err);
    }
    load();
  }

  const toggle = (id, enabled) => change(id, (i) => ({ ...i, enabled }), { enabled });
  const quiet = (id, value) => change(id, (i) => ({ ...i, card: { ...i.card, quiet: value } }), { quiet: value });
  const resume = (id) => change(id, (i) => ({ ...i, error_paused: false, failures: 0, stop_reason: null }), { error_paused: false });

  async function run(id) {
    try {
      await request(`/api/workflows/${encodeURIComponent(id)}/run`, { method: 'POST', body: {} });
      toast(tr('Запустил, смотри в чате бота'));
    } catch (err) {
      fail(err);
    }
    load();
  }

  async function remove(id) {
    try {
      await request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'DELETE' });
    } catch (err) {
      fail(err);
    }
    load();
  }

  async function create(phrase, bot) {
    try {
      const res = await request('/api/workflows/simple', { method: 'POST', body: { phrase, bot_id: bot } });
      adding = false;
      await load();
      toast(tr('Процесс создан: {title}', { title: res.workflow.title }));
      return '';
    } catch (err) {
      return err?.data?.code === 'no_when' ? tr('Не понял, когда запускать. Напишите, например: «каждый понедельник в 9:00 собери отчёт».') : trServer(err?.message || tr('Не получилось'));
    }
  }

  // «Новый процесс», «Новый процесс 2»: первое имя, которого ещё нет.
  async function blank() {
    const base = tr('Новый процесс');
    const used = new Set(items.map((i) => i.title));
    let title = base;
    for (let n = 2; used.has(title); n += 1) title = `${base} ${n}`;
    try {
      const res = await request('/api/workflows', { method: 'POST', body: { title, graph: starterGraph() } });
      onopen?.(res.workflow.id);
      return '';
    } catch (err) {
      return trServer(err?.message || tr('Не получилось'));
    }
  }

  $effect(() => {
    const onEvent = (e) => {
      if (e.detail?.type === 'workflows') load();
    };
    window.addEventListener('deloist:event', onEvent);
    untrack(load);
    return () => window.removeEventListener('deloist:event', onEvent);
  });
</script>

<div class="pl">
  {#if lead}<p class="pl-lead">{lead}</p>{/if}
  {#if loading}
    <Skeleton height="72px" />
  {:else}
    {#each shown as item (item.id)}
      <ProcessCard {item} {bots} {api} onrun={run} onquiet={quiet} ontoggle={toggle} onresume={resume} ondelete={remove} {onopen} />
    {:else}
      <EmptyState title={tr('Процессов пока нет')} text={tr('Процесс запускает бота по расписанию или событию. Опишите его фразой, и бот начнёт работать сам.')} />
    {/each}
  {/if}
  {#if adding}
    <NewProcess {bots} {botId} oncreate={create} onblank={blank} oncancel={() => (adding = false)} />
  {:else}
    <Button class="pl-new" variant="ghost" iconName="plus" onclick={() => (adding = true)}>{tr('Новый процесс')}</Button>
  {/if}
</div>

<style>
  .pl {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
  }
  .pl-lead {
    margin: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    color: var(--k-text-3);
  }
  .pl :global(.pl-new) {
    align-self: flex-start;
  }
</style>
