<script>
  // Экран «Рабочие процессы»: список слева, холст справа. Все запросы идут через api (в тестах подменяется).
  // Перечитываем по событию окна {type:"workflows"}; пока запуск открыт, добавлен запасной опрос (watchOpenRun).
  import { untrack } from 'svelte';
  import { SvelteFlowProvider } from '@xyflow/svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { isRunOpen, newId, starterGraph, toMs } from '../../lib/workflows/graph.js';
  import { versionKindLabel, whenText } from './Labels.svelte';
  import WorkflowList from './WorkflowList.svelte';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import WfIcon from './WfIcon.svelte';

  /** api(path,{method,body}); bots [{id,name,color,emoji}]; onmenu() открывает боковую панель на узком окне. */
  let { api = realApi, bots = [], models = () => ({}), onconnect, onmenu } = $props();

  const POLL_MS = 3000;
  const request = (path, opts) => untrack(() => api)(path, opts);

  let items = $state.raw([]);
  let listLoading = $state(true);
  let selected = $state(readSelected());
  let workflow = $state.raw(null);
  let triggers = $state.raw([]);
  let run = $state.raw(null);
  let viewRun = $state.raw(null);
  let previewV = $state.raw(null);
  let epoch = $state(0);
  let loadError = $state('');
  let listOpen = $state(false);
  let handle = null;

  function readSelected() {
    try {
      return localStorage.getItem('wf-selected') || '';
    } catch {
      return '';
    }
  }

  function remember(id) {
    try {
      if (id) localStorage.setItem('wf-selected', id);
      else localStorage.removeItem('wf-selected');
    } catch {}
  }

  const fail = (err) => toast(trServer(err?.message || tr('Не получилось')));

  async function loadList() {
    try {
      const res = await request('/api/workflows');
      items = res.workflows || [];
      loadError = '';
    } catch (err) {
      loadError = trServer(err?.message || '');
    } finally {
      listLoading = false;
    }
  }

  async function loadRun(id) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(id)}/runs?limit=1`);
      if (id === selected) run = res.runs?.[0] ?? null;
    } catch {}
  }

  async function open(id) {
    selected = id;
    remember(id);
    listOpen = false;
    run = null;
    viewRun = null;
    previewV = null;
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(id)}`);
      if (selected !== id) return;
      workflow = res.workflow;
      triggers = res.triggers || [];
      epoch += 1;
      loadRun(id);
    } catch (err) {
      if (err?.status === 404) {
        selected = '';
        workflow = null;
        remember('');
      } else fail(err);
    }
  }

  // Свежая копия после конфликта 409 или правки в другом месте: холст собирается заново.
  function adopt(fresh) {
    if (!fresh) return open(selected);
    workflow = fresh;
    epoch += 1;
    loadList();
  }

  async function create() {
    try {
      const res = await request('/api/workflows', { method: 'POST', body: { title: tr('Новый процесс'), graph: starterGraph() } });
      await loadList();
      await open(res.workflow.id);
    } catch (err) {
      fail(err);
    }
  }

  async function rename(id, title) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'PATCH', body: { title } });
      items = items.map((i) => (i.id === id ? { ...i, title: res.workflow?.title ?? title } : i));
      if (workflow?.id === id) workflow = { ...workflow, title: res.workflow?.title ?? title };
    } catch (err) {
      fail(err);
    }
  }

  async function duplicate(id) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(id)}/duplicate`, { method: 'POST' });
      await loadList();
      await open(res.workflow.id);
    } catch (err) {
      fail(err);
    }
  }

  async function remove(id) {
    try {
      await request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'DELETE' });
      if (selected === id) {
        selected = '';
        workflow = null;
        run = null;
        remember('');
      }
      await loadList();
      if (!selected && items.length) open(items[0].id);
    } catch (err) {
      fail(err);
    }
  }

  // ---- запуск ----

  async function startStep(nodeId) {
    if (!workflow) return;
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}/nodes/${encodeURIComponent(nodeId)}/run`, { method: 'POST', body: { request_id: newId('r') } });
      viewRun = null;
      run = res.run ?? run;
      loadList();
    } catch (err) {
      fail(err);
    }
  }

  // ---- версии ----

  async function openVersion(v) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}/versions/${encodeURIComponent(v.v)}`);
      previewV = { v: v.v, label: `${versionKindLabel(v.kind)} · ${whenText(toMs(v.at))}`, graph: res.graph || res.workflow?.graph || { nodes: [], edges: [], strokes: [] } };
    } catch (err) {
      fail(err);
    }
  }

  async function restoreVersion() {
    if (!previewV || !workflow) return;
    const v = previewV.v;
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}/versions/${encodeURIComponent(v)}/restore`, { method: 'POST' });
      previewV = null;
      adopt(res.workflow);
      toast(tr('Версия восстановлена'));
    } catch (err) {
      fail(err);
    }
  }

  async function startRun() {
    if (!workflow) return;
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}/run`, { method: 'POST', body: { request_id: newId('r') } });
      viewRun = null;
      run = res.run ?? run;
      loadList();
    } catch (err) {
      fail(err);
    }
  }

  async function stopRun() {
    if (!run) return;
    try {
      const res = await request(`/api/workflow-runs/${encodeURIComponent(run.id)}/stop`, { method: 'POST' });
      run = res.run ?? run;
    } catch (err) {
      fail(err);
    }
  }

  async function stepCall(step, verb, body) {
    try {
      const res = await request(`/api/workflow-steps/${encodeURIComponent(step.id)}/${verb}`, { method: 'POST', body });
      run = res.run ?? run;
    } catch (err) {
      fail(err);
    }
  }

  const actions = {
    decide: (step, approved) => stepCall(step, 'decide', { approved }),
    retry: (step) => stepCall(step, 'retry'),
    stop: (step) => stepCall(step, 'stop'),
  };

  // ---- обновление ----

  // Что-то изменилось на сервере: процесс или запуск.
  async function refresh(workflowId) {
    loadList();
    if (!workflowId || workflowId !== selected) return;
    await loadRun(workflowId);
    // Чужая правка графа (бот или другое окно): подтягиваем, если у нас нет несохранённого.
    if (handle && !handle.pending()) {
      try {
        const res = await request(`/api/workflows/${encodeURIComponent(workflowId)}`);
        if (workflowId !== selected || !handle || handle.pending()) return;
        triggers = res.triggers || triggers;
        if (typeof res.workflow?.revision === 'number' && res.workflow.revision > handle.revision()) adopt(res.workflow);
      } catch {}
    }
  }

  function onEvent(e) {
    const d = e.detail;
    if (d?.type === 'workflows') refresh(d.workflow_id);
  }

  // Запасной опрос: только пока запуск открыт (бежит шаг или ждёт одобрения), раз в POLL_MS.
  function watchOpenRun() {
    if (!isRunOpen(run) || !selected) return () => {};
    const id = selected;
    const t = setInterval(() => loadRun(id), POLL_MS);
    return () => clearInterval(t);
  }

  $effect(() => {
    window.addEventListener('deloist:event', onEvent);
    return () => window.removeEventListener('deloist:event', onEvent);
  });

  $effect(() => watchOpenRun());

  $effect(() => {
    untrack(async () => {
      await loadList();
      if (selected && items.some((i) => i.id === selected)) await open(selected);
      else if (items.length) await open(items[0].id);
      else selected = '';
    });
  });
</script>

<div class="wfs" class:list-open={listOpen}>
  <div class="wfs-list">
    <WorkflowList {items} {selected} loading={listLoading} onselect={open} oncreate={create} onrename={rename} onduplicate={duplicate} ondelete={remove} {onmenu} />
  </div>
  {#if listOpen}
    <button type="button" class="wfs-scrim" aria-label={tr('Закрыть список')} onclick={() => (listOpen = false)}></button>
  {/if}

  <section class="wfs-main">
    {#if workflow}
      {#key `${workflow.id}:${epoch}`}
        <SvelteFlowProvider>
          <WorkflowCanvas
            {workflow}
            {triggers}
            {bots}
            run={viewRun ?? run}
            viewing={Boolean(viewRun)}
            {models}
            {onconnect}
            api={request}
            {actions}
            onview={(r) => (viewRun = r)}
            onpreview={openVersion}
            onrunstep={startStep}
            onrun={startRun}
            onstop={stopRun}
            onrename={(t) => rename(workflow.id, t)}
            onconflict={adopt}
            onlist={() => (listOpen = !listOpen)}
            onready={(h) => (handle = h)}
          />
        </SvelteFlowProvider>
      {/key}
      {#if previewV}
        <div class="wfs-preview">
          {#key previewV.v}
            <SvelteFlowProvider>
              <WorkflowCanvas
                workflow={{ id: workflow.id, title: workflow.title, revision: workflow.revision, graph: previewV.graph }}
                {bots}
                {models}
                api={request}
                preview={{ label: previewV.label, onrestore: restoreVersion, onclose: () => (previewV = null) }}
              />
            </SvelteFlowProvider>
          {/key}
        </div>
      {/if}
    {:else if !listLoading}
      <div class="wfs-empty">
        <div class="wfs-empty__ico"><WfIcon name="graph" size={30} /></div>
        {#if loadError}
          <b>{tr('Не удалось загрузить процессы')}</b>
          <p>{loadError}</p>
          <button type="button" class="wfs-btn" onclick={loadList}>{tr('Повторить')}</button>
        {:else}
          <b>{tr('Соберите процесс из шагов')}</b>
          <p>{tr('Триггер запускает цепочку: промпт, боты, одобрение, вывод. Шаги соединяются линиями, результат виден на каждом узле.')}</p>
          <button type="button" class="wfs-btn" onclick={create}><WfIcon name="plus" size={14} /><span>{tr('Создать процесс')}</span></button>
        {/if}
      </div>
    {/if}
  </section>
</div>

<style>
  .wfs {
    position: relative;
    display: grid;
    grid-template-columns: 276px minmax(0, 1fr);
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--k-canvas);
  }
  .wfs-list {
    min-width: 0;
    min-height: 0;
  }
  .wfs-main {
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  .wfs-scrim {
    display: none;
  }
  .wfs-preview {
    position: absolute;
    inset: 0;
    z-index: 25;
    background: var(--k-canvas);
    animation: wfs-in 0.4s var(--k-ease-spring, ease) both;
  }
  .wfs-empty {
    display: grid;
    align-content: center;
    justify-items: center;
    gap: 8px;
    height: 100%;
    padding: 24px;
    text-align: center;
    animation: wfs-in 0.6s var(--k-ease-spring, ease) both;
  }
  .wfs-empty__ico {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    margin-bottom: 6px;
    border-radius: 22px;
    background: color-mix(in oklab, var(--k-bot-violet) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-violet) 70%, var(--k-text));
  }
  .wfs-empty b {
    font: 800 20px/1.2 var(--k-font-display);
  }
  .wfs-empty p {
    max-width: 380px;
    margin: 0 0 8px;
    font: 400 14px/1.5 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfs-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 18px 0 15px;
    border: 0;
    border-radius: 13px;
    background: var(--k-ink, #000);
    color: var(--k-on-ink, #fff);
    font: 700 14px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfs-btn:hover {
    background: var(--k-ink-3, #24262c);
  }
  .wfs-btn:active {
    transform: scale(0.95);
  }
  @keyframes wfs-in {
    from {
      opacity: 0;
      transform: scale(0.94) translateY(8px);
      filter: blur(6px);
    }
  }
  @media (max-width: 820px) {
    .wfs {
      grid-template-columns: minmax(0, 1fr);
    }
    .wfs-list {
      position: absolute;
      z-index: 30;
      inset: 0 auto 0 0;
      width: min(300px, 86%);
      translate: -102% 0;
      box-shadow: var(--k-shadow-3);
      transition: translate 320ms var(--k-ease-morph, ease);
    }
    .list-open .wfs-list {
      translate: 0 0;
    }
    .wfs-scrim {
      display: block;
      position: absolute;
      z-index: 20;
      inset: 0;
      border: 0;
      background: var(--k-backdrop);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfs-empty {
      animation: none;
    }
    .wfs-list {
      transition: none;
    }
  }
</style>
