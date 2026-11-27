<script>
  // Холст одного процесса: открывается из карточки раздела «Задачи и процессы» (id), кнопка «Список процессов» зовёт onback. Все запросы идут через api (в тестах подменяется).
  // Перечитываем по событию окна {type:"workflows"}; пока запуск открыт, добавлен запасной опрос (watchOpenRun).
  import { untrack } from 'svelte';
  import { SvelteFlowProvider } from '@xyflow/svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { isRunOpen, newId, toMs } from '../../lib/workflows/graph.js';
  import { versionKindLabel, whenText } from './Labels.svelte';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import { Button } from '../kit';

  /** id — процесс на холсте; api(path,{method,body}); bots [{id,name,color,emoji}]; onback() возвращает к списку. */
  let { id, api = realApi, bots = [], models = () => ({}), onconnect, onback, onsettings, uploadAsset, fileUrl } = $props();

  const POLL_MS = 3000;
  const request = (path, opts) => untrack(() => api)(path, opts);

  let selected = $state(untrack(() => id));
  let workflow = $state.raw(null);
  let triggers = $state.raw([]);
  let run = $state.raw(null);
  let viewRun = $state.raw(null);
  let previewV = $state.raw(null);
  let epoch = $state(0);
  let loadError = $state('');
  let loading = $state(true);
  let connections = $state.raw({});
  let handle = null;

  const fail = (err) => toast(trServer(err?.message || tr('Не получилось')));

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

  async function setEnabled(value) {
    const before = workflow?.enabled ?? true;
    workflow = { ...workflow, enabled: value };
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}`, { method: 'PATCH', body: { enabled: value } });
      workflow = { ...workflow, enabled: res.workflow?.enabled ?? value };
    } catch (err) {
      workflow = { ...workflow, enabled: before };
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
      loadError = trServer(err?.message || '');
      if (err?.status !== 404) fail(err);
    } finally {
      loading = false;
    }
  }

  // Свежая копия после конфликта 409 или правки в другом месте: холст собирается заново.
  function adopt(fresh) {
    if (!fresh) return open(selected);
    conflict = null;
    workflow = fresh;
    epoch += 1;
  }

  async function rename(title) {
    try {
      const res = await request(`/api/workflows/${encodeURIComponent(workflow.id)}`, { method: 'PATCH', body: { title } });
      workflow = { ...workflow, title: res.workflow?.title ?? title };
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
    untrack(() => open(selected));
  });
</script>

<div class="wfs">
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
            onenabled={setEnabled}
            {connections}
            {onsettings}
            api={request}
            {actions}
            onview={(r) => (viewRun = r)}
            onpreview={openVersion}
            onrunstep={startStep}
            onrun={startRun}
            onstop={stopRun}
            onrename={rename}
            onconflict={askConflict}
            onlist={onback}
            onready={(h) => (handle = h)}
          />
        </SvelteFlowProvider>
      {/key}
      {#if conflict}
        <div class="wfs-conflict" role="alertdialog" aria-label={tr('Процесс изменили в другом месте')}>
          <span>{tr('Процесс изменили в другом месте, а у вас есть несохранённые правки. Автосохранение на паузе.')}</span>
          <Button class="wfs-cf-btn" variant="primary" size="sm" onclick={keepMine}>{tr('Оставить мои правки')}</Button>
          <Button class="wfs-cf-btn" variant="secondary" size="sm" onclick={loadFresh}>{tr('Загрузить свежую')}</Button>
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
    {:else if !loading}
      <div class="wfs-empty">
        <b>{tr('Не удалось загрузить процессы')}</b>
        {#if loadError}<p>{loadError}</p>{/if}
        <Button variant="secondary" onclick={() => open(selected)}>{tr('Повторить')}</Button>
      </div>
    {/if}
  </section>
</div>

<style>
  .wfs {
    position: relative;
    display: grid;
        width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--k-canvas);
  }
  .wfs-main {
    position: relative;
    min-width: 0;
    min-height: 0;
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
    padding: var(--k-space-3);
    translate: -50% 0;
    border-radius: var(--k-radius-md);
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow: var(--k-shadow-2), inset 0 0 0 1px var(--k-line-strong);
    font: 600 var(--k-text-sm) / 1.35 var(--k-font-body);
  }
  .wfs-conflict span {
    flex: 1 1 240px;
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
  .wfs-empty b {
    font: 700 var(--k-text-xl) / 1.2 var(--k-font-display);
  }
  .wfs-empty p {
    max-width: 380px;
    margin: 0 0 8px;
    font: 400 var(--k-text-md) / 1.5 var(--k-font-body);
    color: var(--k-text-3);
  }
  @keyframes wfs-in {
    from {
      opacity: 0;
      transform: scale(0.94) translateY(8px);
      filter: blur(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfs-empty {
      animation: none;
    }
  }
</style>
