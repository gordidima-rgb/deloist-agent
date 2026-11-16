<script>
  // Холст процесса на Svelte Flow: узлы, связи, нижняя панель, масштаб, поиск, отмена, сохранение.
  // Работает внутри <SvelteFlowProvider>. Данные приходят от WorkflowsScreen, запуск тоже.
  import { setContext, untrack } from 'svelte';
  import '@xyflow/svelte/dist/style.css';
  import { SvelteFlow, Background, MiniMap, Panel, useSvelteFlow, useViewport } from '@xyflow/svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { CanvasModel } from '../../lib/workflows/canvasModel.svelte.js';
  import { Saver } from '../../lib/workflows/saver.js';
  import { canConnect, connectionProblem, isRunOpen, kindOf, kindsFor, latestSteps, stepSeconds } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { connectionText, statusLabel, seconds } from './Labels.svelte';
  import WfNode from './WfNode.svelte';
  import WfNote from './WfNote.svelte';
  import WfEdge from './WfEdge.svelte';
  import AddMenu from './AddMenu.svelte';
  import WfIcon from './WfIcon.svelte';

  /**
   * workflow {id,title,revision,graph}; triggers [{node_id,type,next_run}]; bots [{id,name,color,emoji}];
   * run: последний запуск или null; api(path,{method,body}); actions {decide(step,ok), retry(step), stop(step)};
   * onrun(), onstop(), onrename(title), onconflict(workflow), onlist(); onready(handle) — ручка для экрана.
   */
  let { workflow, triggers = [], bots = [], run = null, api, actions = {}, onrun, onstop, onrename, onconflict, onlist, onready } = $props();

  const flow = useSvelteFlow();
  const viewport = useViewport();

  const wf0 = untrack(() => workflow);
  let saveState = $state('idle');
  let title = $state(untrack(() => workflow.title));
  let tool = $state('select');
  let menu = $state(null);
  let searchOpen = $state(false);
  let query = $state('');
  let hitIndex = $state(0);
  let minimap = $state(readFlag('wf-minimap', false));
  let now = $state(Date.now());
  let box = $state();
  let searchInput = $state();

  function readFlag(key, fallback) {
    try {
      const v = localStorage.getItem(key);
      return v == null ? fallback : v === '1';
    } catch {
      return fallback;
    }
  }

  const model = new CanvasModel({ onchange: () => saver.schedule() });
  const saver = new Saver({
    api: (...a) => api(...a),
    id: wf0.id,
    revision: wf0.revision,
    getGraph: () => model.getGraph(),
    on: {
      state: (s) => (saveState = s),
      conflict: (fresh) => {
        toast(tr('Процесс изменили в другом месте. Загрузили свежую версию.'));
        onconflict?.(fresh);
      },
      error: (err) => {
        const d = err?.data || {};
        if (err?.status === 400) model.invalid = { node_id: d.node_id || '', edge_id: d.edge_id || '' };
        toast(trServer(err?.message || tr('Не удалось сохранить процесс')));
      },
      saved: () => {
        if (model.invalid.node_id || model.invalid.edge_id) model.invalid = { node_id: '', edge_id: '' };
      },
    },
  });
  model.load(wf0.graph);

  const steps = $derived(latestSteps(run));
  const runOpen = $derived(isRunOpen(run));
  const triggerMap = $derived(new Map(triggers.map((t) => [t.node_id, t])));
  const hits = $derived.by(() => (searchOpen ? new Set(model.search(query).map((n) => n.id)) : new Set()));
  const hitList = $derived(searchOpen ? model.search(query) : []);
  const anyRunning = $derived([...steps.values()].some((s) => s.status === 'running' || s.status === 'approval'));
  const runSeconds = $derived(run ? stepSeconds({ started_at: run.started_at, ended_at: runOpen ? null : run.ended_at }, now) : 0);

  setContext(WF, {
    model,
    get bots() {
      return bots;
    },
    get steps() {
      return steps;
    },
    get runOpen() {
      return runOpen;
    },
    get triggers() {
      return triggerMap;
    },
    get hits() {
      return hits;
    },
    get now() {
      return now;
    },
    decide: (step, ok) => actions.decide?.(step, ok),
    retry: (step) => actions.retry?.(step),
    stopStep: (step) => actions.stop?.(step),
    removeEdge: (id) => model.removeEdge(id),
  });

  const nodeTypes = { wfnode: WfNode, wfnote: WfNote };
  const edgeTypes = { wf: WfEdge };

  untrack(() => onready)?.({
    pending: () => saver.pending,
    revision: () => saver.revision,
    flush: () => saver.flush(),
  });

  // Секунды идущего шага тикают только пока шаг работает.
  $effect(() => {
    if (!anyRunning && !runOpen) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  $effect(() => () => {
    saver.flush();
    saver.destroy();
  });

  $effect(() => {
    try {
      localStorage.setItem('wf-minimap', minimap ? '1' : '0');
    } catch {}
  });

  // ---- имя процесса ----

  function commitTitle() {
    const next = title.trim();
    if (!next) title = workflow.title;
    else if (next !== workflow.title) onrename?.(next);
  }

  // ---- добавление узлов ----

  function centerPoint() {
    const r = box.getBoundingClientRect();
    return flow.screenToFlowPosition({ x: r.left + r.width / 2, y: r.top + r.height / 2 });
  }

  // Ближайшее свободное место: сдвигаем вправо и вниз, пока не перестанем налезать на другие узлы.
  function freeSpot(x, y, w, h) {
    const boxes = model.nodes.map((n) => ({
      x: n.position.x,
      y: n.position.y,
      w: n.measured?.width || n.width || 288,
      h: n.measured?.height || n.height || 170,
    }));
    const hit = (px, py) => boxes.some((b) => px < b.x + b.w + 24 && px + w + 24 > b.x && py < b.y + b.h + 24 && py + h + 24 > b.y);
    for (let i = 0; i < 60 && hit(x, y); i += 1) {
      x += 40;
      if (i % 6 === 5) {
        x -= 240;
        y += 60;
      }
    }
    return { x, y };
  }

  function addAtCenter(kind) {
    const c = centerPoint();
    const w = kind === 'note' ? 220 : kind === 'text' ? 120 : 288;
    const p = freeSpot(c.x - w / 2, c.y - 90, w, kind === 'note' ? 160 : 130);
    return model.addNode(kind, p.x, p.y);
  }

  function openAddMenu() {
    const r = box.getBoundingClientRect();
    menu = { x: r.width / 2 - 145, y: r.height - 72, origin: 'bottom', allowed: null, from: null };
  }

  function onPick(kind) {
    const m = menu;
    menu = null;
    if (!m) return;
    if (m.pos) {
      const node = model.addNode(kind, m.pos.x - (kind === 'note' ? 0 : 20), m.pos.y - 30, {}, { commit: !m.from });
      if (m.from && node) {
        const f = m.from;
        const conn =
          f.type === 'source'
            ? { source: f.nodeId, target: node.id, sourceHandle: f.handleId || 'out', targetHandle: null }
            : { source: node.id, target: f.nodeId, sourceHandle: 'out', targetHandle: null };
        const res = model.connect(conn);
        if (res.edge) model.addEdge(res.edge, { commit: false });
        model.commit();
      }
    } else {
      addAtCenter(kind);
    }
  }

  // Линию бросили на пустом месте: меню видов, которые к ней подходят.
  function onConnectEnd(event, state) {
    if (state?.isValid || state?.toNode || !state?.fromNode) return;
    const ev = 'changedTouches' in event ? event.changedTouches[0] : event;
    const r = box.getBoundingClientRect();
    const fromKind = kindOf(state.fromNode);
    const side = state.fromHandle?.type === 'target' ? 'target' : 'source';
    const allowed = kindsFor(fromKind, side === 'source' ? 'source' : 'target');
    if (!allowed.length) return;
    menu = {
      x: ev.clientX - r.left,
      y: ev.clientY - r.top,
      origin: 'top',
      allowed,
      pos: flow.screenToFlowPosition({ x: ev.clientX, y: ev.clientY }),
      from: { nodeId: state.fromNode.id, type: side, handleId: state.fromHandle?.id || null },
    };
  }

  function onPaneClick({ event }) {
    menu = null;
    if (tool === 'note' || tool === 'text') {
      const pos = flow.screenToFlowPosition({ x: event.clientX, y: event.clientY });
      model.addNode(tool, pos.x - (tool === 'note' ? 100 : 20), pos.y - 20);
      tool = 'select';
    }
  }

  function onPaneContext({ event }) {
    event.preventDefault();
    const r = box.getBoundingClientRect();
    menu = {
      x: event.clientX - r.left,
      y: event.clientY - r.top,
      origin: 'top',
      allowed: null,
      pos: flow.screenToFlowPosition({ x: event.clientX, y: event.clientY }),
      from: null,
    };
  }

  // ---- связи ----

  const isValidConnection = (c) => canConnect(model.nodes, model.edges, c);

  function onBeforeConnect(conn) {
    const res = model.connect(conn);
    if (res.edge) return res.edge;
    toast(connectionText(res.problem));
    return null;
  }

  function onConnectDone(_, state) {
    // Отпустили на порте, который не принимает: объясняем, почему нет.
    if (state && !state.isValid && state.toNode && state.fromNode) {
      const from = state.fromHandle?.type === 'target' ? state.toNode : state.fromNode;
      const to = state.fromHandle?.type === 'target' ? state.fromNode : state.toNode;
      const why = connectionProblem(kindOf(from), kindOf(to));
      toast(connectionText(why || (from.id === to.id ? 'self' : 'duplicate')));
    }
  }

  // ---- поиск ----

  function openSearch() {
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }

  function closeSearch() {
    searchOpen = false;
    query = '';
    hitIndex = 0;
  }

  function stepHit(dir) {
    if (!hitList.length) return;
    hitIndex = (hitIndex + dir + hitList.length) % hitList.length;
    const n = hitList[hitIndex];
    model.clearSelection();
    model.nodes = model.nodes.map((x) => (x.id === n.id ? { ...x, selected: true } : x));
    const w = n.measured?.width || n.width || 288;
    const h = n.measured?.height || n.height || 160;
    flow.setCenter(n.position.x + w / 2, n.position.y + h / 2, { zoom: Math.max(flow.getZoom(), 0.9), duration: 380 });
  }

  // ---- клавиши ----

  function editing(target) {
    if (!(target instanceof Element)) return false;
    return Boolean(target.closest('input, textarea, select, [contenteditable="true"]'));
  }

  function onKey(e) {
    if (!box || box.offsetParent === null) return;
    if (document.querySelector('dialog[open]')) return;
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (mod && key === 'enter') {
      e.preventDefault();
      runOrStop();
      return;
    }
    if (editing(e.target)) {
      if (e.key === 'Escape' && searchOpen) closeSearch();
      return;
    }
    if (mod && key === 'f') {
      e.preventDefault();
      openSearch();
    } else if (mod && key === 'z') {
      e.preventDefault();
      if (e.shiftKey) model.redo();
      else model.undo();
    } else if (mod && key === 'y') {
      e.preventDefault();
      model.redo();
    } else if (mod && key === 'd') {
      e.preventDefault();
      model.duplicateSelected();
    } else if (mod && key === 'a') {
      e.preventDefault();
      model.selectAll();
    } else if (e.key === 'Escape') {
      menu = null;
      tool = 'select';
      if (searchOpen) closeSearch();
      else model.clearSelection();
    } else if (!mod && !e.altKey) {
      if (key === 'v') tool = 'select';
      else if (key === 'h') tool = 'hand';
      else if (key === 'n') tool = tool === 'note' ? 'select' : 'note';
      else if (key === 't') tool = tool === 'text' ? 'select' : 'text';
    }
  }

  // ---- запуск ----

  async function runOrStop() {
    if (runOpen) return onstop?.();
    await saver.flush();
    if (!saver.dead) onrun?.();
  }

  const saveText = $derived(
    saveState === 'saving' ? tr('Сохраняю…') : saveState === 'dirty' ? tr('Есть несохранённые правки') : saveState === 'error' ? tr('Не сохранилось') : tr('Сохранено'),
  );
  const zoomPct = $derived(Math.round((viewport.current?.zoom ?? 1) * 100));
</script>

<svelte:window onkeydown={onKey} />

<div class="wf-canvas" data-tool={tool} bind:this={box}>
  <SvelteFlow
    bind:nodes={model.nodes}
    bind:edges={model.edges}
    {nodeTypes}
    {edgeTypes}
    fitView
    fitViewOptions={{ padding: 0.14, maxZoom: 1 }}
    minZoom={0.25}
    maxZoom={2}
    deleteKey={['Backspace', 'Delete']}
    panOnDrag={tool === 'hand' ? true : [1, 2]}
    selectionOnDrag={tool === 'select'}
    zoomOnDoubleClick={false}
    connectionRadius={28}
    connectionLineStyle="stroke: var(--k-brand); stroke-width: 2.4; stroke-dasharray: 6 6;"
    proOptions={{ hideAttribution: true }}
    {isValidConnection}
    onbeforeconnect={onBeforeConnect}
    onconnect={() => model.commit()}
    onconnectend={(e, s) => {
      onConnectDone(e, s);
      onConnectEnd(e, s);
    }}
    ondelete={() => model.commit()}
    onnodedragstop={() => model.commit()}
    onselectiondragstop={() => model.commit()}
    onpaneclick={onPaneClick}
    onpanecontextmenu={onPaneContext}
  >
    <Background gap={24} size={1.7} patternColor="color-mix(in oklab, var(--k-text) 22%, var(--k-canvas))" bgColor="var(--k-canvas)" />
    {#if minimap}
      <MiniMap position="bottom-right" pannable zoomable nodeColor="var(--k-line-strong)" maskColor="color-mix(in oklab, var(--k-canvas) 70%, transparent)" bgColor="var(--k-surface)" nodeBorderRadius={6} />
    {/if}

    <Panel position="top-left" class="wf-p-top">
      <div class="wf-bar wf-bar--title">
        <button type="button" class="wf-ib wf-ib--list" title={tr('Список процессов')} aria-label={tr('Список процессов')} onclick={() => onlist?.()}>
          <WfIcon name="list" size={17} />
        </button>
        <input
          class="wf-title"
          type="text"
          maxlength="120"
          aria-label={tr('Название процесса')}
          bind:value={title}
          onblur={commitTitle}
          onkeydown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            if (e.key === 'Escape') {
              title = workflow.title;
              e.currentTarget.blur();
            }
          }}
        />
      </div>
    </Panel>

    <Panel position="top-right" class="wf-p-top">
      <div class="wf-bar">
        <span class="wf-save" data-state={saveState}>{saveText}</span>
        <button type="button" class="wf-ib" title={tr('Отменить (Ctrl+Z)')} aria-label={tr('Отменить')} disabled={!model.canUndo} onclick={() => model.undo()}>
          <WfIcon name="undo" size={16} />
        </button>
        <button type="button" class="wf-ib" title={tr('Повторить (Ctrl+Shift+Z)')} aria-label={tr('Повторить')} disabled={!model.canRedo} onclick={() => model.redo()}>
          <WfIcon name="redo" size={16} />
        </button>
        {#if run}
          <span class="wf-runchip" data-status={run.status} title={statusLabel(run.status)}>
            {#if runOpen}<span class="wf-runchip__dot"></span>{/if}
            {statusLabel(run.status)}
            <span class="wf-runchip__sec">{seconds(runSeconds)}</span>
          </span>
        {/if}
        <button type="button" class="wf-run" class:is-stop={runOpen} title={runOpen ? tr('Остановить (Ctrl+Enter)') : tr('Запустить (Ctrl+Enter)')} onclick={runOrStop}>
          <WfIcon name={runOpen ? 'stop' : 'play'} size={13} />
          <span>{runOpen ? tr('Остановить') : tr('Запустить')}</span>
        </button>
      </div>
    </Panel>

    <Panel position="bottom-left" class="wf-p-zoom">
      <div class="wf-bar wf-bar--zoom">
        <button type="button" class="wf-ib" title={tr('Отдалить')} aria-label={tr('Отдалить')} onclick={() => flow.zoomOut({ duration: 200 })}><WfIcon name="minus" size={15} /></button>
        <button type="button" class="wf-zoomval" title={tr('Масштаб 100%')} onclick={() => flow.setZoom(1, { duration: 200 })}>{zoomPct}%</button>
        <button type="button" class="wf-ib" title={tr('Приблизить')} aria-label={tr('Приблизить')} onclick={() => flow.zoomIn({ duration: 200 })}><WfIcon name="plus" size={15} /></button>
        <button type="button" class="wf-ib" title={tr('Показать всё')} aria-label={tr('Показать всё')} onclick={() => flow.fitView({ padding: 0.14, maxZoom: 1, duration: 300 })}><WfIcon name="fit" size={15} /></button>
        <button type="button" class="wf-ib" class:on={minimap} aria-pressed={minimap} title={tr('Мини-карта')} aria-label={tr('Мини-карта')} onclick={() => (minimap = !minimap)}><WfIcon name="map" size={15} /></button>
      </div>
    </Panel>

    <Panel position="bottom-center" class="wf-p-tools">
      {#if searchOpen}
        <div class="wf-search nodrag nopan">
          <WfIcon name="search" size={15} />
          <input
            bind:this={searchInput}
            type="text"
            placeholder={tr('Найти узел по тексту')}
            bind:value={query}
            oninput={() => (hitIndex = -1)}
            onkeydown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                stepHit(e.shiftKey ? -1 : 1);
              }
              if (e.key === 'Escape') closeSearch();
            }}
          />
          <span class="wf-search__n">{query ? tr('{n} найдено', { n: hitList.length }) : ''}</span>
          <button type="button" class="wf-ib" aria-label={tr('Закрыть поиск')} onclick={closeSearch}><WfIcon name="x" size={14} /></button>
        </div>
      {/if}
      <div class="wf-tools nodrag nopan" role="toolbar" aria-label={tr('Инструменты холста')}>
        <button type="button" class="wf-tool" class:on={tool === 'select'} aria-pressed={tool === 'select'} title={tr('Выбор (V)')} aria-label={tr('Выбор')} onclick={() => (tool = 'select')}><WfIcon name="select" size={17} /></button>
        <button type="button" class="wf-tool" class:on={tool === 'hand'} aria-pressed={tool === 'hand'} title={tr('Рука (H)')} aria-label={tr('Рука')} onclick={() => (tool = 'hand')}><WfIcon name="hand" size={17} /></button>
        <span class="wf-sep"></span>
        <button type="button" class="wf-tool" class:on={tool === 'note'} aria-pressed={tool === 'note'} title={tr('Заметка (N)')} aria-label={tr('Заметка')} onclick={() => (tool = tool === 'note' ? 'select' : 'note')}><WfIcon name="note" size={17} /></button>
        <button type="button" class="wf-tool" class:on={tool === 'text'} aria-pressed={tool === 'text'} title={tr('Текст (T)')} aria-label={tr('Текст')} onclick={() => (tool = tool === 'text' ? 'select' : 'text')}><WfIcon name="text" size={17} /></button>
        <button type="button" class="wf-tool" class:on={searchOpen} aria-pressed={searchOpen} title={tr('Поиск (Ctrl+F)')} aria-label={tr('Поиск')} onclick={() => (searchOpen ? closeSearch() : openSearch())}><WfIcon name="search" size={17} /></button>
        <span class="wf-sep"></span>
        <button type="button" class="wf-add" aria-haspopup="menu" aria-expanded={Boolean(menu && !menu.pos)} onclick={() => (menu && !menu.pos ? (menu = null) : openAddMenu())}>
          <WfIcon name="plus" size={15} />
          <span>{tr('Добавить узел')}</span>
        </button>
      </div>
    </Panel>
  </SvelteFlow>

  {#if model.nodes.length === 0}
    <div class="wf-empty">
      <div class="wf-empty__ico"><WfIcon name="graph" size={26} /></div>
      <b>{tr('Процесс пуст')}</b>
      <p>{tr('Добавьте первый узел: триггер запускает процесс, дальше идут шаги.')}</p>
      <button type="button" class="wf-run" onclick={openAddMenu}><WfIcon name="plus" size={13} /><span>{tr('Добавить узел')}</span></button>
    </div>
  {/if}

  {#if menu}
    <AddMenu allowed={menu.allowed} origin={menu.origin} x={menu.x} y={menu.y} onpick={onPick} onclose={() => (menu = null)} />
  {/if}
</div>

<style>
  .wf-canvas {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--k-canvas);
  }
  .wf-canvas :global(.svelte-flow) {
    --xy-background-color: var(--k-canvas);
    --xy-edge-stroke: var(--k-line-strong);
    --xy-selection-background-color: color-mix(in oklab, var(--k-brand) 10%, transparent);
    --xy-selection-border: 1.5px solid var(--k-brand);
    --xy-minimap-background-color: var(--k-surface);
    --xy-node-boxshadow-selected: none;
    background: var(--k-canvas);
    font-family: var(--k-font-body);
  }
  .wf-canvas :global(.svelte-flow__node) {
    --xy-node-border-radius: 20px;
  }
  .wf-canvas :global(.svelte-flow__node.selectable:focus),
  .wf-canvas :global(.svelte-flow__node.selectable:focus-visible) {
    outline: none;
  }
  .wf-canvas :global(.svelte-flow__node-wfnode),
  .wf-canvas :global(.svelte-flow__node-wfnote) {
    padding: 0;
    border: 0;
    background: transparent;
    box-shadow: none;
    border-radius: 0;
  }
  .wf-canvas[data-tool='hand'] :global(.svelte-flow__pane) {
    cursor: grab;
  }
  .wf-canvas[data-tool='hand'] :global(.svelte-flow__pane.dragging) {
    cursor: grabbing;
  }
  .wf-canvas[data-tool='note'] :global(.svelte-flow__pane),
  .wf-canvas[data-tool='text'] :global(.svelte-flow__pane) {
    cursor: crosshair;
  }
  .wf-canvas :global(.svelte-flow__minimap) {
    margin: 0 14px 14px 0;
    border-radius: 16px;
    border: 1px solid var(--k-line);
    box-shadow: var(--k-shadow-2);
    overflow: hidden;
    animation: wf-pop 0.34s var(--k-ease-spring, ease) both;
  }
  .wf-canvas :global(.svelte-flow__panel) {
    margin: 14px;
  }
  .wf-canvas :global(.wf-p-top) {
    margin-top: 12px;
  }
  .wf-canvas :global(.wf-p-tools) {
    display: grid;
    justify-items: center;
    gap: 8px;
    margin-bottom: 14px;
  }

  .wf-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px;
    border-radius: 16px;
    border: 1px solid var(--k-line);
    background: color-mix(in oklab, var(--k-surface) 86%, transparent);
    backdrop-filter: blur(14px);
    box-shadow: var(--k-shadow-2);
    animation: wf-pop 0.45s var(--k-ease-spring, ease) both;
  }
  .wf-title {
    field-sizing: content;
    min-width: 140px;
    max-width: min(34vw, 360px);
    height: 34px;
    padding: 0 10px;
    border: 1px solid transparent;
    border-radius: 11px;
    background: transparent;
    color: var(--k-text);
    font: 800 15px/1 var(--k-font-display);
    letter-spacing: -0.01em;
    transition:
      background 150ms var(--k-ease-out, ease),
      border-color 150ms var(--k-ease-out, ease);
  }
  .wf-title:hover {
    background: var(--k-surface-2);
  }
  .wf-title:focus {
    outline: none;
    background: var(--k-surface);
    border-color: var(--k-brand);
    box-shadow: var(--k-focus);
  }
  .wf-ib {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-ib:hover:not(:disabled) {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .wf-ib:active:not(:disabled) {
    transform: scale(0.95);
  }
  .wf-ib:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .wf-ib.on {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wf-ib--list {
    display: none;
  }
  .wf-save {
    padding: 0 8px;
    font: 500 12px/1 var(--k-font-body);
    color: var(--k-text-3);
    white-space: nowrap;
  }
  .wf-save[data-state='error'] {
    color: var(--k-attention-text);
  }
  .wf-runchip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    margin: 0 2px;
    padding: 0 11px;
    border-radius: 999px;
    background: var(--k-surface-2);
    font: 700 12px/1 var(--k-font-body);
    color: var(--k-text-2);
    animation: wf-pop 0.4s var(--k-ease-spring, ease) both;
  }
  .wf-runchip[data-status='running'],
  .wf-runchip[data-status='stopping'] {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wf-runchip[data-status='done'] {
    background: color-mix(in oklab, var(--k-bot-mint) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-mint) 75%, var(--k-text));
  }
  .wf-runchip[data-status='failed'],
  .wf-runchip[data-status='approval'] {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wf-runchip__sec {
    font-weight: 500;
    opacity: 0.8;
    font-variant-numeric: tabular-nums;
  }
  .wf-runchip__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    animation: wf-blink 1.2s ease-in-out infinite;
  }
  .wf-run {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 15px 0 13px;
    border: 0;
    border-radius: 11px;
    background: var(--k-ink, #000);
    color: var(--k-on-ink, #fff);
    font: 700 13px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-run:hover {
    background: var(--k-ink-3, #24262c);
  }
  .wf-run:active {
    transform: scale(0.95);
  }
  .wf-run.is-stop {
    background: var(--k-attention);
    color: #fff;
  }
  :global(:root[data-theme='dark']) .wf-run:not(.is-stop),
  :global(:root:not([data-theme='light'])) .wf-run:not(.is-stop) {
    box-shadow: inset 0 0 0 1px var(--k-ink-line);
  }

  .wf-bar--zoom {
    gap: 2px;
  }
  .wf-zoomval {
    min-width: 50px;
    height: 34px;
    padding: 0 6px;
    border: 0;
    border-radius: 11px;
    background: transparent;
    color: var(--k-text);
    font: 700 12.5px/1 var(--k-font-body);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: background 150ms var(--k-ease-out, ease);
  }
  .wf-zoomval:hover {
    background: var(--k-surface-2);
  }

  .wf-tools {
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 6px;
    border-radius: 20px;
    border: 1px solid var(--k-line);
    background: color-mix(in oklab, var(--k-surface) 88%, transparent);
    backdrop-filter: blur(14px);
    box-shadow: var(--k-shadow-3);
    animation: wf-pop 0.5s var(--k-ease-spring, ease) both;
  }
  .wf-tool {
    display: grid;
    place-items: center;
    width: 40px;
    height: 38px;
    padding: 0;
    border: 0;
    border-radius: 13px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-tool:hover {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .wf-tool:active {
    transform: scale(0.92);
  }
  .wf-tool.on {
    background: var(--k-ink, #000);
    color: var(--k-on-ink, #fff);
  }
  .wf-sep {
    width: 1px;
    height: 22px;
    margin: 0 4px;
    background: var(--k-line);
  }
  .wf-add {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 38px;
    padding: 0 15px 0 12px;
    border: 0;
    border-radius: 13px;
    background: var(--k-brand);
    color: var(--k-on-brand, #fff);
    font: 700 13px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-add:hover {
    background: var(--k-brand-hover);
  }
  .wf-add:active {
    transform: scale(0.95);
  }

  .wf-search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: min(380px, 80vw);
    height: 44px;
    padding: 0 6px 0 14px;
    border-radius: 16px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    color: var(--k-text-3);
    animation: wf-pop 0.34s var(--k-ease-spring, ease) both;
    transform-origin: bottom center;
  }
  .wf-search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--k-text);
    font: 500 14px/1 var(--k-font-body);
  }
  .wf-search__n {
    font: 500 12px/1 var(--k-font-body);
    white-space: nowrap;
  }

  .wf-empty {
    position: absolute;
    inset: 0;
    display: grid;
    align-content: center;
    justify-items: center;
    gap: 6px;
    padding: 0 24px 90px;
    text-align: center;
    pointer-events: none;
    animation: wf-pop 0.6s var(--k-ease-spring, ease) both;
  }
  .wf-empty > * {
    pointer-events: auto;
  }
  .wf-empty__ico {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: 6px;
    border-radius: 18px;
    background: var(--k-surface);
    border: 1px solid var(--k-line);
    color: var(--k-text-2);
    box-shadow: var(--k-shadow-1);
  }
  .wf-empty b {
    font: 800 18px/1.2 var(--k-font-display);
  }
  .wf-empty p {
    max-width: 320px;
    margin: 0 0 8px;
    font: 400 13.5px/1.45 var(--k-font-body);
    color: var(--k-text-3);
  }

  @keyframes wf-pop {
    from {
      opacity: 0;
      transform: scale(0.92) translateY(6px);
      filter: blur(5px);
    }
  }
  @keyframes wf-blink {
    50% {
      opacity: 0.3;
    }
  }
  @media (max-width: 820px) {
    .wf-ib--list {
      display: grid;
    }
    .wf-save,
    .wf-add span,
    .wf-run span {
      display: none;
    }
    .wf-title {
      max-width: 34vw;
      min-width: 90px;
    }
    .wf-canvas :global(.wf-p-zoom) {
      margin-bottom: 80px;
    }
    .wf-run {
      padding: 0 11px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-bar,
    .wf-tools,
    .wf-search,
    .wf-empty,
    .wf-runchip {
      animation: none;
    }
    .wf-runchip__dot {
      animation: none;
    }
  }
</style>
