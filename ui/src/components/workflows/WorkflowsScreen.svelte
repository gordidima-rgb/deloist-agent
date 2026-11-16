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
  let { api = realApi, bots = [], models = () => ({}), onconnect, onmenu, onsettings, uploadAsset, fileUrl } = $props();

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
  let connections = $state.raw({});
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

  // Подключены ли мессенджеры: триггеры и выводы Telegram, Discord и Slack работают только с привязанным местом.
  async function loadConnections() {
    const ask = async (path, key) => {
      try {
        const st = await request(path);
        return Boolean(st?.connected && (st[key] ?? false));
      } catch {
        return false;
      }
    };
    const [telegram, discord, slack] = await Promise.all([ask('/api/telegram', 'chatPaired'), ask('/api/discord', 'paired'), ask('/api/slack', 'paired')]);
    connections = { telegram, discord, slack };
  }

  async function setEnabled(id, value) {
    const before = items.find((i) => i.id === id)?.enabled ?? workflow?.enabled ?? true;
    items = items.map((i) => (i.id === id ? { ...i, enabled: value } : i));
    if (workflow?.id === id) workflow = { ...workflow, enabled: value };
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'PATCH', body: { enabled: value } });
      const real = res.workflow?.enabled ?? value;
      items = items.map((i) => (i.id === id ? { ...i, enabled: real } : i));
      if (workflow?.id === id) workflow = { ...workflow, enabled: real };
    } catch (err) {
      items = items.map((i) => (i.id === id ? { ...i, enabled: before } : i));
      if (workflow?.id === id) workflow = { ...workflow, enabled: before };
      fail(err);
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
    conflict = null;
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
      loadConnections();
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
    conflict = null;
    workflow = fresh;
    epoch += 1;
    loadList();
  }

  // «Новый процесс», «Новый процесс 2», …: первое имя, которого ещё нет в списке.
  function freshTitle() {
    const base = tr('Новый процесс');
    const used = new Set(items.map((i) => i.title));
    if (!used.has(base)) return base;
    let n = 2;
    while (used.has(`${base} ${n}`)) n += 1;
    return `${base} ${n}`;
  }

  async function create() {
    try {
      const res = await request('/api/workflows', { method: 'POST', body: { title: freshTitle(), graph: starterGraph() } });
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
    if (handle) await pullRemote(workflowId, 0);
  }

  // Чужая правка графа (бот или другое окно): без своих несохранённых правок холст обновляется на месте и играет
  // анимацию; с ними правки не трогаем, а человеку показываем карточку, как при конфликте 409.
  let staleToast = 0;
  let conflict = $state.raw(null);

  // Свои правки не теряем молча: автосохранение стоит, пока человек не выберет.
  function askConflict(fresh) {
    handle?.hold();
    conflict = { fresh: fresh ?? null };
  }

  async function keepMine() {
    const c = conflict;
    conflict = null;
    let revision = c?.fresh?.revision;
    if (typeof revision !== 'number' && workflow) {
      try {
        revision = (await request(`/api/workflows/${encodeURIComponent(workflow.id)}`)).workflow?.revision;
      } catch {}
    }
    handle?.keepMine(revision);
  }

  function loadFresh() {
    const c = conflict;
    conflict = null;
    adopt(c?.fresh);
  }
  async function pullRemote(workflowId, depth) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflowId)}`);
      if (workflowId !== selected || !handle) return;
      triggers = res.triggers || triggers;
      const fresh = res.workflow;
      if (workflow && typeof fresh?.enabled === 'boolean' && fresh.enabled !== workflow.enabled) workflow = { ...workflow, enabled: fresh.enabled };
      if (typeof fresh?.revision !== 'number' || fresh.revision <= handle.revision()) return;
      // Наше собственное сохранение ещё в пути: его событие пришло раньше ответа. Ждём и смотрим снова.
      const flying = handle.inflight();
      if (flying && depth < 2) {
        await flying.catch(() => {});
        return pullRemote(workflowId, depth + 1);
      }
      if (handle.remote(fresh)) {
        workflow = { ...workflow, ...fresh, graph: workflow.graph };
      } else if (staleToast !== fresh.revision) {
        staleToast = fresh.revision;
        askConflict(fresh);
      }
    } catch {}
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
    <WorkflowList {items} {selected} loading={listLoading} onselect={open} onenabled={setEnabled} oncreate={create} onrename={rename} onduplicate={duplicate} ondelete={remove} {onmenu} />
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
            {uploadAsset}
            {fileUrl}
            enabled={workflow.enabled !== false}
            onenabled={(v) => setEnabled(workflow.id, v)}
            {connections}
            {onsettings}
            api={request}
            {actions}
            onview={(r) => (viewRun = r)}
            onpreview={openVersion}
            onrunstep={startStep}
            onrun={startRun}
            onstop={stopRun}
            onrename={(t) => rename(workflow.id, t)}
            onconflict={askConflict}
            onlist={() => (listOpen = !listOpen)}
            onready={(h) => (handle = h)}
          />
        </SvelteFlowProvider>
      {/key}
      {#if conflict}
        <div class="wfs-conflict" role="alertdialog" aria-label={tr('Процесс изменили в другом месте')}>
          <span>{tr('Процесс изменили в другом месте, а у вас есть несохранённые правки. Автосохранение на паузе.')}</span>
          <button type="button" class="wfs-cf-btn" onclick={keepMine}>{tr('Оставить мои правки')}</button>
          <button type="button" class="wfs-cf-btn" onclick={loadFresh}>{tr('Загрузить свежую')}</button>
        </div>
      {/if}
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
  .wfs-conflict {
    position: absolute;
    left: 50%;
    top: 12px;
    z-index: 40;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    max-width: min(560px, calc(100% - 24px));
    padding: 10px 12px;
    translate: -50% 0;
    border-radius: 14px;
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow: var(--k-shadow-pop, 0 8px 28px rgb(0 0 0 / 0.25)), inset 0 0 0 1px var(--k-line-strong);
    font: 500 13px/1.35 var(--k-font-body);
  }
  .wfs-conflict span {
    flex: 1 1 240px;
  }
  .wfs-cf-btn {
    padding: 6px 12px;
    border: 0;
    border-radius: 10px;
    background: var(--k-surface-3);
    color: var(--k-text);
    font: 600 12.5px/1.2 var(--k-font-body);
    cursor: pointer;
  }
  .wfs-cf-btn:first-of-type {
    background: var(--k-accent, var(--k-surface-3));
    color: var(--k-accent-ink, var(--k-text));
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
