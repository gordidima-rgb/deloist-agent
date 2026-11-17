<script>
  // Холст процесса на Svelte Flow: узлы, связи, рисование, фигуры, меню, поиск, отмена, сохранение, история.
  // Работает внутри <SvelteFlowProvider>. Данные и запуск приходят от WorkflowsScreen.
  import { setContext, untrack } from 'svelte';
  import '@xyflow/svelte/dist/style.css';
  import { SvelteFlow, Background, MiniMap, Panel, useSvelteFlow, useViewport, getNodesBounds } from '@xyflow/svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { fileUrlFor, uploadAssetTo, toDataUrl } from '../../lib/workflows/assets.js';
  import { CanvasModel } from '../../lib/workflows/canvasModel.svelte.js';
  import { Saver } from '../../lib/workflows/saver.js';
  import { SHAPE_CHOICES, canConnect, connectionProblem, typeProblem, isRunOpen, kindOf, kindsFor, kindsBetween, latestSteps, stepSeconds, toMs, newId } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { connectionText, statusLabel, seconds } from './Labels.svelte';
  import WfNode from './WfNode.svelte';
  import WfNote from './WfNote.svelte';
  import WfShape from './WfShape.svelte';
  import WfEdge from './WfEdge.svelte';
  import AddMenu from './AddMenu.svelte';
  import { Menu } from '../kit';
  import { wfIconKey } from '../../lib/workflows/iconKey.js';
  import ToolOptions from './ToolOptions.svelte';
  import StrokesLayer from './StrokesLayer.svelte';
  import HistoryPopover from './HistoryPopover.svelte';
  import ModelPicker from '../ModelPicker.svelte';
  import WfIcon from './WfIcon.svelte';
  import { Button, IconButton, Switch, SaveMark } from '../kit';
  import ToolArt from './ToolArt.svelte';

  /**
   * workflow {id,title,revision,graph}; triggers [{node_id,type,next_run}]; bots [{id,name,color,emoji}];
   * run: показанный запуск (последний или выбранный в истории) или null; viewing: показан старый запуск;
   * models(): {providers,plan,settings} для выбора модели; api(path,{method,body}); actions {decide, retry, stop};
   * onrun(), onrunstep(nodeId), onstop(), onrename(title), onconflict(workflow), onlist(), onready(handle);
   * onview(run|null), onpreview(version), onconnect(); preview {label, onrestore(), onclose()} — просмотр версии без правок.
   */
  let {
    workflow,
    triggers = [],
    bots = [],
    run = null,
    viewing = false,
    models = () => ({}),
    api,
    actions = {},
    preview = null,
    onrun,
    onrunstep,
    onstop,
    onrename,
    onconflict,
    onlist,
    onready,
    onview,
    onpreview,
    onconnect,
    uploadAsset,
    fileUrl,
    enabled = true,
    onenabled,
    connections = {},
    onsettings,
  } = $props();

  const flow = useSvelteFlow();
  const viewport = useViewport();

  const wf0 = untrack(() => workflow);
  const readonly = untrack(() => Boolean(preview));
  let saveState = $state('idle');
  let title = $state(untrack(() => workflow.title));
  let tool = $state('select'); // select | hand | note | text | shape | draw
  let brush = $state('pen');
  let brushColor = $state('ink');
  let brushSize = $state(3);
  let shapeId = $state('rect');
  let shapeFill = $state('sky');
  let noteColor = $state('lemon');
  let shakeNode = $state('');
  let previews = $state.raw({});
  let lightbox = $state('');
  let menu = $state(null);
  let ctxMenu = $state(null);
  let panel = $state(null); // 'runs' | 'versions' | null
  let searchOpen = $state(false);
  let query = $state('');
  let hitIndex = $state(0);
  let minimap = $state(readFlag('wf-minimap', false));
  let hud = $state(true);
  let now = $state(Date.now());
  let box = $state();
  let searchInput = $state();
  let modelPick = $state(null);
  let picks = $state.raw({});
  let lastPointer = { x: 0, y: 0 };

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
  if (readonly) saver.destroy();

  // Все шаги узла по порядку: стрелки «предыдущий / следующий результат» у петель и повторов.
  const stepsByNode = $derived.by(() => {
    const map = new Map();
    for (const s of run?.steps || []) {
      if (!map.has(s.node_id)) map.set(s.node_id, []);
      map.get(s.node_id).push(s);
    }
    for (const list of map.values()) list.sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0) || (a.attempt || 0) - (b.attempt || 0) || toMs(a.started_at) - toMs(b.started_at));
    return map;
  });
  const steps = $derived.by(() => {
    const map = latestSteps(run);
    for (const [id, list] of stepsByNode) {
      const i = picks[id];
      if (i != null && list[i]) map.set(id, list[i]);
    }
    return map;
  });
  const runOpen = $derived(isRunOpen(run) && !viewing);
  const triggerMap = $derived(new Map(triggers.map((t) => [t.node_id, t])));
  const hitList = $derived(searchOpen ? model.search(query) : []);
  const hits = $derived(new Set(hitList.map((n) => n.id)));
  const anyRunning = $derived([...steps.values()].some((s) => s.status === 'running' || s.status === 'approval'));
  const runSeconds = $derived(run ? stepSeconds({ started_at: run.started_at, ended_at: runOpen ? null : run.ended_at }, now) : 0);
  const zoomNow = $derived(viewport.current?.zoom ?? 1);
  const drawTool = $derived(tool === 'draw' ? brush : null);

  $effect(() => {
    // Другой запуск: выбранные стрелками результаты сбрасываем.
    run?.id;
    picks = {};
  });

  setContext(WF, {
    model,
    workflowId: wf0.id,
    get enabled() {
      return enabled;
    },
    get connections() {
      return connections;
    },
    openSettings: (sub) => onsettings?.(sub),
    makeWebhook: async (nodeId) => {
      await saver.flush();
      const res = await api(`/api/workflows/${encodeURIComponent(wf0.id)}/nodes/${encodeURIComponent(nodeId)}/webhook`, { method: 'POST' });
      return res?.url || '';
    },
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
    models: () => models() || {},
    pickModel: (id, el) => (modelPick = { id, anchor: el }),
    stepList: (id) => stepsByNode.get(id) || [],
    pickIndex: (id) => {
      const list = stepsByNode.get(id) || [];
      const cur = steps.get(id);
      const at = list.indexOf(cur);
      return at < 0 ? list.length - 1 : at;
    },
    pick: (id, i) => (picks = { ...picks, [id]: i }),
    decide: (step, ok) => actions.decide?.(step, ok),
    retry: (step) => actions.retry?.(step),
    stopStep: (step) => actions.stop?.(step),
    removeEdge: (id) => model.removeEdge(id),
    shakeId: () => shakeNode,
    assetPreview: (id) => previews[id] || '',
    fileUrl: (path) => (fileUrl ? fileUrl(path, workflow.id) : fileUrlFor(workflow.id, path)),
    assetFromFile: (id, file) => assetFromFile(id, file),
    clearAsset: (id) => {
      model.setFields(id, { name: '', path: '', mime: '', size: 0, text: '' });
      const { [id]: gone, ...rest } = previews;
      previews = rest;
    },
    zoomImage: (url) => (lightbox = url),
  });

  const nodeTypes = { wfnode: WfNode, wfnote: WfNote, wfshape: WfShape };
  const edgeTypes = { wf: WfEdge };

  untrack(() => onready)?.({
    pending: () => saver.pending || model.typing,
    revision: () => saver.revision,
    flush: () => saver.flush(),
    hold: () => saver.hold(),
    keepMine: (revision) => saver.release(revision),
    inflight: () => saver.inFlight,
    // Чужая правка: подменить граф на месте, если своих несохранённых правок нет. Иначе false.
    remote: (fresh) => {
      if (!fresh || saver.pending || saver.dead || model.typing) return false;
      model.applyRemote(fresh.graph);
      saver.revision = fresh.revision;
      title = fresh.title ?? title;
      return true;
    },
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

  // ---- ресурсы: файлы на холсте ----

  const TEXTLIKE = /^(text\/|application\/(json|xml|x-yaml|yaml|csv))/;

  async function assetFromFile(id, file) {
    const meta = { name: file.name, mime: file.type || '', size: file.size };
    // Страница не пускает blob: в картинки (CSP), поэтому показ до загрузки идёт через data:.
    if (file.type.startsWith('image/') && file.size <= 8 * 1024 * 1024) {
      try {
        previews = { ...previews, [id]: await toDataUrl(file) };
      } catch {}
    }
    if ((TEXTLIKE.test(file.type) || /\.(txt|md|csv|json|log)$/i.test(file.name)) && file.size <= 20000) {
      try {
        meta.text = await file.text();
      } catch {}
    }
    if (!meta.text) {
      try {
        const res = await (uploadAsset ? uploadAsset(file, workflow.id) : uploadAssetTo(api, workflow.id, file));
        if (res?.path) meta.path = res.path;
      } catch (err) {
        toast(trServer(err?.message || tr('Файл не загрузился')));
      }
    }
    model.setFields(id, meta);
  }

  function onDragOver(e) {
    if (readonly || !e.dataTransfer?.types?.includes('Files')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }

  function onDrop(e) {
    if (readonly || !e.dataTransfer?.files?.length) return;
    e.preventDefault();
    const pos = flow.screenToFlowPosition({ x: e.clientX, y: e.clientY });
    let i = 0;
    for (const f of e.dataTransfer.files) {
      const node = model.addNode('asset', pos.x - 150 + i * 30, pos.y - 40 + i * 30, {}, { commit: false });
      if (node) assetFromFile(node.id, f);
      i += 1;
    }
    model.commit();
  }

  // Отказ в соединении: узел-приёмник вздрагивает.
  function shake(id) {
    if (!id) return;
    shakeNode = id;
    setTimeout(() => {
      if (shakeNode === id) shakeNode = '';
    }, 460);
  }

  // ---- вид ----

  // Первый показ: карточки читаются (не меньше 85%), а если процесс шире окна, слева виден триггер.
  function initialView() {
    if (!box || !model.nodes.length) return;
    const b = getNodesBounds(model.nodes);
    const W = box.clientWidth;
    const H = box.clientHeight;
    if (!b.width || !W) return;
    const padX = 48;
    const fit = Math.min((W - padX * 2) / b.width, (H - 190) / Math.max(b.height, 1));
    const z = Math.min(1, Math.max(0.85, fit));
    const x = fit >= 0.85 ? (W - b.width * z) / 2 - b.x * z : padX - b.x * z;
    const y = (H + 40 - b.height * z) / 2 - b.y * z;
    flow.setViewport({ x, y, zoom: z });
  }

  function onInit() {
    setTimeout(initialView, 80);
  }

  const fitAll = () => flow.fitView({ padding: 0.14, maxZoom: 1, duration: 320 });

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
      w: n.measured?.width || n.width || 304,
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
    const w = kind === 'note' ? 220 : kind === 'text' ? 120 : 304;
    const p = freeSpot(c.x - w / 2, c.y - 90, w, kind === 'note' ? 160 : 130);
    return model.addNode(kind, p.x, p.y);
  }

  function openAddMenu() {
    const r = box.getBoundingClientRect();
    ctxMenu = null;
    menu = { x: r.width / 2 - 145, y: r.height - 72, origin: 'bottom', allowed: null, from: null };
  }

  function connectNew(node, from) {
    const conn =
      from.type === 'source'
        ? { source: from.nodeId, target: node.id, sourceHandle: from.handleId || 'out', targetHandle: null }
        : { source: node.id, target: from.nodeId, sourceHandle: kindOf(node) === 'condition' ? 'yes' : 'out', targetHandle: null };
    const res = model.connect(conn);
    if (res.edge) model.addEdge(res.edge, { commit: false });
  }

  function onPick(kind) {
    const m = menu;
    menu = null;
    if (!m) return;
    if (m.edgeId) {
      model.insertIntoEdge(m.edgeId, kind);
    } else if (m.pos) {
      const node = model.addNode(kind, m.pos.x - 20, m.pos.y - 30, {}, { commit: !m.from });
      if (m.from && node) {
        connectNew(node, m.from);
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
    const side = state.fromHandle?.type === 'target' ? 'target' : 'source';
    const allowed = kindsFor(kindOf(state.fromNode), side, state.fromNode.data?.fields);
    if (!allowed.length) return;
    ctxMenu = null;
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
    ctxMenu = null;
    panel = null;
    const pos = flow.screenToFlowPosition({ x: event.clientX, y: event.clientY });
    if (tool === 'note' || tool === 'text') {
      model.addNode(tool, pos.x - (tool === 'note' ? 100 : 20), pos.y - 20, tool === 'note' ? { fields: { text: '', color: noteColor }, w: 220, h: 160 } : {});
      tool = 'select';
    } else if (tool === 'shape') {
      const ch = SHAPE_CHOICES.find((c) => c.id === shapeId) || SHAPE_CHOICES[0];
      const round = ch.shape === 'circle';
      model.addNode('shape', pos.x - 80, pos.y - 60, { fields: { shape: ch.shape, fill: shapeFill, text: '', ...(ch.flip ? { flip: true } : {}) }, w: round ? 130 : 170, h: round ? 130 : 120 });
      tool = 'select';
    }
  }

  // ---- связи ----

  const isValidConnection = (c) => canConnect(model.nodes, model.edges, c);

  function onBeforeConnect(conn) {
    const res = model.connect(conn);
    if (res.edge) return res.edge;
    toast(connectionText(res.problem));
    shake(conn.target);
    return null;
  }

  function onConnectDone(_, state) {
    // Отпустили на порте, который не принимает: объясняем, почему нет.
    if (state && !state.isValid && state.toNode && state.fromNode) {
      const from = state.fromHandle?.type === 'target' ? state.toNode : state.fromNode;
      const to = state.fromHandle?.type === 'target' ? state.fromNode : state.toNode;
      const why = connectionProblem(kindOf(from), kindOf(to));
      const mismatch = !why && typeProblem(kindOf(from), from.data?.fields, kindOf(to));
      toast(connectionText(why || mismatch || (from.id === to.id ? 'self' : 'duplicate')));
      shake(to.id);
    }
  }

  // ---- удаление с возвратом ----

  function afterDelete() {
    model.commit();
    toast(tr('Удалено'), { action: { label: tr('Вернуть'), onclick: () => model.undo() } });
  }

  function deleteSelected() {
    if (model.removeSelected()) toast(tr('Удалено'), { action: { label: tr('Вернуть'), onclick: () => model.undo() } });
  }

  // ---- поиск и переход к узлу ----

  function openSearch() {
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }

  function closeSearch() {
    searchOpen = false;
    query = '';
    hitIndex = 0;
  }

  function focusNode(n) {
    model.clearSelection();
    model.nodes = model.nodes.map((x) => (x.id === n.id ? { ...x, selected: true } : x));
    const w = n.measured?.width || n.width || 304;
    const h = n.measured?.height || n.height || 160;
    flow.setCenter(n.position.x + w / 2, n.position.y + h / 2, { zoom: Math.max(flow.getZoom(), 0.9), duration: 380 });
  }

  function stepHit(dir) {
    if (!hitList.length) return;
    hitIndex = (hitIndex + dir + hitList.length) % hitList.length;
    focusNode(hitList[hitIndex]);
  }

  // ---- контекстные меню ----

  function point(event) {
    const r = box.getBoundingClientRect();
    return { x: event.clientX - r.left, y: event.clientY - r.top };
  }

  const RUNNABLE = ['bot', 'llm', 'condition', 'approval', 'output'];

  function onNodeContext({ event, node }) {
    event.preventDefault();
    if (readonly) return;
    menu = null;
    if (!node.selected) {
      model.clearSelection();
      model.nodes = model.nodes.map((n) => (n.id === node.id ? { ...n, selected: true } : n));
    }
    const kind = kindOf(node);
    const resizable = kind === 'note' || kind === 'shape';
    ctxMenu = {
      ...point(event),
      target: { type: 'node', id: node.id },
      items: [
        ...(RUNNABLE.includes(kind) ? [{ id: 'runstep', label: tr('Запустить шаг'), icon: 'run_step' }, { sep: true }] : []),
        { id: 'copy', label: tr('Копировать'), icon: 'dup', hint: 'Ctrl+C' },
        { id: 'cut', label: tr('Вырезать'), icon: 'scissors', hint: 'Ctrl+X' },
        { id: 'duplicate', label: tr('Дублировать'), icon: 'dup', hint: 'Ctrl+D' },
        { sep: true },
        { id: 'lock', label: node.data.locked ? tr('Разблокировать') : tr('Заблокировать'), icon: node.data.locked ? 'unlock' : 'lock' },
        { id: 'front', label: tr('На передний план'), icon: 'front' },
        { id: 'back', label: tr('На задний план'), icon: 'back' },
        ...(resizable ? [{ id: 'resetsize', label: tr('Вернуть размер'), icon: 'fit' }] : []),
        { sep: true },
        { id: 'delete', label: tr('Удалить'), icon: 'trash', danger: true, hint: 'Del' },
      ],
    };
  }

  function onEdgeContext({ event, edge }) {
    event.preventDefault();
    if (readonly) return;
    menu = null;
    const s = model.nodes.find((n) => n.id === edge.source);
    const t = model.nodes.find((n) => n.id === edge.target);
    const can = s && t && kindsBetween(kindOf(s), kindOf(t)).length > 0;
    ctxMenu = {
      ...point(event),
      target: { type: 'edge', id: edge.id },
      items: [
        { id: 'insert', label: tr('Вставить узел в связь'), icon: 'link', disabled: !can },
        { sep: true },
        { id: 'delete', label: tr('Удалить связь'), icon: 'trash', danger: true },
      ],
    };
  }

  function onPaneContext({ event }) {
    event.preventDefault();
    if (readonly) return;
    menu = null;
    ctxMenu = {
      ...point(event),
      target: { type: 'pane', pos: flow.screenToFlowPosition({ x: event.clientX, y: event.clientY }) },
      items: [
        { id: 'addhere', label: tr('Добавить узел сюда'), icon: 'plus' },
        { id: 'paste', label: tr('Вставить'), icon: 'paste', hint: 'Ctrl+V', disabled: !model.clipboard },
        { id: 'selectall', label: tr('Выделить всё'), icon: 'select_all', hint: 'Ctrl+A' },
      ],
    };
  }

  function onCtxPick(id) {
    const t = ctxMenu?.target;
    const at = ctxMenu ? { x: ctxMenu.x, y: ctxMenu.y } : { x: 0, y: 0 };
    ctxMenu = null;
    if (!t) return;
    if (t.type === 'node') {
      if (id === 'copy') model.copy();
      else if (id === 'cut') model.cut();
      else if (id === 'duplicate') model.duplicateSelected();
      else if (id === 'lock') model.selectedIds.forEach((n) => model.toggleLock(n));
      else if (id === 'front') model.bringToFront(t.id);
      else if (id === 'back') model.sendToBack(t.id);
      else if (id === 'resetsize') model.resetSize(t.id);
      else if (id === 'delete') deleteSelected();
      else if (id === 'runstep') runStep(t.id);
    } else if (t.type === 'edge') {
      if (id === 'delete') model.removeEdge(t.id);
      else if (id === 'insert') {
        const e = model.edges.find((x) => x.id === t.id);
        const s = model.nodes.find((n) => n.id === e?.source);
        const d = model.nodes.find((n) => n.id === e?.target);
        if (s && d) menu = { ...at, origin: 'top', allowed: kindsBetween(kindOf(s), kindOf(d)), edgeId: t.id, from: null };
      }
    } else if (id === 'addhere') {
      menu = { ...at, origin: 'top', allowed: null, pos: t.pos, from: null };
    } else if (id === 'paste') model.paste(t.pos);
    else if (id === 'selectall') model.selectAll();
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
    if (readonly) return;
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
    } else if (mod && (key === '/' || e.code === 'Slash')) {
      e.preventDefault();
      hud = !hud;
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
    } else if (mod && key === 'c') {
      if (model.copy()) e.preventDefault();
    } else if (mod && key === 'x') {
      if (model.cut()) e.preventDefault();
    } else if (mod && key === 'v') {
      if (model.clipboard) {
        e.preventDefault();
        const r = box.getBoundingClientRect();
        const inside = lastPointer.x >= r.left && lastPointer.x <= r.right && lastPointer.y >= r.top && lastPointer.y <= r.bottom;
        model.paste(inside ? flow.screenToFlowPosition(lastPointer) : null);
      }
    } else if (e.key === 'Tab') {
      const n = model.neighbour(e.shiftKey ? -1 : 1);
      if (n) {
        e.preventDefault();
        focusNode(n);
      }
    } else if (e.key === 'Escape') {
      menu = null;
      ctxMenu = null;
      panel = null;
      tool = 'select';
      if (searchOpen) closeSearch();
      else model.clearSelection();
    } else if (!mod && !e.altKey) {
      if (key === 'v') tool = 'select';
      else if (key === 'h') tool = 'hand';
      else if (key === 'n') tool = tool === 'note' ? 'select' : 'note';
      else if (key === 't') tool = tool === 'text' ? 'select' : 'text';
      else if (key === 'r') tool = tool === 'shape' ? 'select' : 'shape';
      else if (key === 'p') {
        brush = brush === 'eraser' ? 'pen' : brush;
        tool = tool === 'draw' ? 'select' : 'draw';
      } else if (key === 'e') {
        brush = 'eraser';
        tool = 'draw';
      }
    }
  }

  // ---- запуск ----

  async function runOrStop() {
    if (runOpen) return onstop?.();
    await saver.flush();
    if (!saver.dead) onrun?.();
  }

  async function runStep(id) {
    await saver.flush();
    if (!saver.dead) onrunstep?.(id);
  }

  const saveText = $derived(
    saveState === 'saving' ? tr('Сохраняю…') : saveState === 'dirty' ? tr('Есть несохранённые правки') : saveState === 'error' ? tr('Не сохранилось') : tr('Сохранено'),
  );
  const zoomPct = $derived(Math.round(zoomNow * 100));
  const modelValue = $derived.by(() => {
    const n = modelPick && model.nodes.find((x) => x.id === modelPick.id);
    return { driver: n?.data.fields?.provider || '', model: n?.data.fields?.model || '' };
  });
  const toolTitle = (label, key) => `${label} (${key})`;
</script>

<svelte:window onkeydown={onKey} onpointermove={(e) => (lastPointer = { x: e.clientX, y: e.clientY })} />

<div class="wf-canvas" class:is-ro={readonly} class:is-hud-off={!hud} data-tool={tool} bind:this={box} role="presentation" ondragover={onDragOver} ondrop={onDrop}>
  <SvelteFlow
    bind:nodes={model.nodes}
    bind:edges={model.edges}
    {nodeTypes}
    {edgeTypes}
    minZoom={0.25}
    maxZoom={2}
    deleteKey={readonly ? null : ['Backspace', 'Delete']}
    panActivationKey="Space"
    panOnDrag={tool === 'hand' || readonly ? true : [1, 2]}
    selectionOnDrag={tool === 'select' && !readonly}
    nodesDraggable={!readonly}
    nodesConnectable={!readonly}
    elementsSelectable={!readonly}
    zoomOnDoubleClick={false}
    connectionRadius={28}
    connectionLineStyle="stroke: var(--k-brand); stroke-width: 2.4; stroke-dasharray: 6 6;"
    proOptions={{ hideAttribution: true }}
    {isValidConnection}
    oninit={onInit}
    onbeforeconnect={onBeforeConnect}
    onconnect={() => model.commit()}
    onconnectend={(e, s) => {
      onConnectDone(e, s);
      onConnectEnd(e, s);
    }}
    ondelete={afterDelete}
    onnodedragstop={() => model.commit()}
    onselectiondragstop={() => model.commit()}
    onpaneclick={onPaneClick}
    onpanecontextmenu={onPaneContext}
    onnodecontextmenu={onNodeContext}
    onedgecontextmenu={onEdgeContext}
  >
    <Background gap={18} size={1.4} patternColor="color-mix(in oklab, var(--k-text) 16%, transparent)" bgColor="var(--k-canvas)" />
    <StrokesLayer
      strokes={model.strokes}
      tool={readonly ? null : drawTool}
      color={brushColor}
      width={brushSize}
      zoom={zoomNow}
      onadd={(s) => model.addStroke(s)}
      onerase={(ids) => model.removeStrokes(ids)}
    />
    {#if minimap}
      <MiniMap position="bottom-right" style="margin-bottom: 72px;" pannable zoomable nodeColor="var(--k-line-strong)" maskColor="color-mix(in oklab, var(--k-canvas) 70%, transparent)" bgColor="var(--k-surface)" nodeBorderRadius={6} />
    {/if}

    {#if readonly}
      <Panel position="top-center" class="wf-p-top">
        <div class="wf-bar wf-bar--preview">
          <WfIcon name="versions" size={16} />
          <b>{preview.label}</b>
          <span class="wf-bar__hint">{tr('Только просмотр')}</span>
          <Button variant="primary" size="sm" onclick={() => preview.onrestore?.()}>{tr('Сделать текущей')}</Button>
          <IconButton name="close" size="sm" label={tr('Закрыть просмотр')} onclick={() => preview.onclose?.()} />
        </div>
      </Panel>
    {:else}
      <Panel position="top-left" class="wf-p-top">
        <div class="wf-bar wf-bar--title">
          <IconButton class="wf-ib--list" name="wf-list" size="sm" label={tr('Список процессов')} onclick={() => onlist?.()} />
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
          {#if !readonly}
            <span class="wf-en" class:off={!enabled} title={enabled ? tr('Процесс включён: расписание, вебхук и мессенджеры запускают его') : tr('Процесс выключен: расписание, вебхук и мессенджеры его не запустят. Вручную запускать можно.')}>
              <Switch checked={enabled} aria-label={tr("Включён")} onchange={(v) => onenabled?.(v)} />
              <span>{enabled ? tr('Включён') : tr('Выключен')}</span>
            </span>
          {/if}
          <SaveMark class="wf-save" state={saveState} text={saveText} />
          <IconButton name="wf-undo" size="sm" label={tr('Отменить')} title={tr('Отменить (Ctrl+Z)')} disabled={!model.canUndo} onclick={() => model.undo()} />
          <IconButton name="wf-redo" size="sm" label={tr('Повторить')} title={tr('Повторить (Ctrl+Shift+Z)')} disabled={!model.canRedo} onclick={() => model.redo()} />
          <IconButton class="wf-ib" name="wf-history" size="sm" label={tr('История')} title={tr('История запусков')} data-wf-history-btn aria-pressed={panel === 'runs'} onclick={() => (panel = panel === 'runs' ? null : 'runs')} />
          <IconButton class="wf-ib" name="wf-versions" size="sm" label={tr('Версии')} title={tr('Версии процесса')} data-wf-history-btn aria-pressed={panel === 'versions'} onclick={() => (panel = panel === 'versions' ? null : 'versions')} />
          {#if run}
            <span class="wf-runchip" data-status={run.status} title={statusLabel(run.status)}>
              {#if runOpen}<span class="wf-runchip__dot"></span>{/if}
              {statusLabel(run.status)}
              <span class="wf-runchip__sec">{seconds(runSeconds)}</span>
            </span>
          {/if}
          <Button class="wf-run" variant={runOpen ? 'danger' : 'primary'} size="sm" iconName={runOpen ? 'wf-stop' : 'wf-play'} title={runOpen ? tr('Остановить (Ctrl+Enter)') : tr('Запустить (Ctrl+Enter)')} onclick={runOrStop}>
            <span>{runOpen ? tr('Остановить') : tr('Запустить')}</span>
          </Button>
        </div>
      </Panel>

      <Panel position="bottom-right" class="wf-p-zoom">
        <div class="wf-bar wf-bar--zoom">
          <IconButton name="wf-minus" size="sm" label={tr('Отдалить')} onclick={() => flow.zoomOut({ duration: 200 })} />
          <Button class="wf-zoomval" variant="ghost" size="sm" title={tr('Масштаб 100%')} onclick={() => flow.setZoom(1, { duration: 200 })}>{zoomPct}%</Button>
          <IconButton name="plus" size="sm" label={tr('Приблизить')} onclick={() => flow.zoomIn({ duration: 200 })} />
          <IconButton name="wf-fit" size="sm" label={tr('Показать всё')} onclick={fitAll} />
          <IconButton class="wf-ib" name="wf-map" size="sm" label={tr('Мини-карта')} aria-pressed={minimap} onclick={() => (minimap = !minimap)} />
        </div>
      </Panel>

      <Panel position="bottom-center" class="wf-p-tools">
        {#if tool === 'draw' || tool === 'shape' || tool === 'note'}
          <ToolOptions
            mode={tool}
            {brush}
            color={brushColor}
            size={brushSize}
            {shapeId}
            fill={shapeFill}
            {noteColor}
            onbrush={(b) => (brush = b)}
            oncolor={(c) => (brushColor = c)}
            onsize={(n) => (brushSize = n)}
            onshape={(id) => (shapeId = id)}
            onfill={(f) => (shapeFill = f)}
            onnote={(c) => (noteColor = c)}
          />
        {/if}
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
            <IconButton name="close" size="sm" label={tr('Закрыть поиск')} onclick={closeSearch} />
          </div>
        {/if}
        <div class="wf-tools nodrag nopan" role="toolbar" aria-label={tr('Инструменты холста')}>
          <IconButton class="wf-tool" name="wf-select" label={tr('Выбор')} title={toolTitle(tr('Выбор'), 'V')} aria-pressed={tool === 'select'} onclick={() => (tool = 'select')} />
          <IconButton class="wf-tool" name="wf-hand" label={tr('Рука')} title={toolTitle(tr('Рука'), 'H')} aria-pressed={tool === 'hand'} onclick={() => (tool = 'hand')} />
          <span class="wf-sep"></span>
          <IconButton class="wf-tool wf-tool--art" label={tr('Рисование')} title={toolTitle(tr('Рисование'), 'P')} aria-pressed={tool === 'draw'} onclick={() => (tool = tool === 'draw' ? 'select' : 'draw')}>{#snippet icon()}<ToolArt name="pencil" />{/snippet}</IconButton>
          <IconButton class="wf-tool wf-tool--art" label={tr('Заметка')} title={toolTitle(tr('Заметка'), 'N')} aria-pressed={tool === 'note'} onclick={() => (tool = tool === 'note' ? 'select' : 'note')}>{#snippet icon()}<ToolArt name="note" />{/snippet}</IconButton>
          <IconButton class="wf-tool wf-tool--art" label={tr('Фигуры')} title={toolTitle(tr('Фигуры'), 'R')} aria-pressed={tool === 'shape'} onclick={() => (tool = tool === 'shape' ? 'select' : 'shape')}>{#snippet icon()}<ToolArt name="shapes" />{/snippet}</IconButton>
          <IconButton class="wf-tool wf-tool--t" label={tr('Текст')} title={toolTitle(tr('Текст'), 'T')} aria-pressed={tool === 'text'} onclick={() => (tool = tool === 'text' ? 'select' : 'text')}>{#snippet icon()}<span class="wf-tool__t">T</span>{/snippet}</IconButton>
          <IconButton class="wf-tool" name="wf-search" label={tr('Поиск')} title={toolTitle(tr('Поиск'), 'Ctrl+F')} aria-pressed={searchOpen} onclick={() => (searchOpen ? closeSearch() : openSearch())} />
          <span class="wf-sep"></span>
          <Button class="wf-add" variant="secondary" iconName="plus" aria-haspopup="menu" aria-expanded={Boolean(menu && !menu.pos && !menu.edgeId)} onclick={() => (menu && !menu.pos ? (menu = null) : openAddMenu())}>
            <span>{tr('Добавить узел')}</span>
          </Button>
        </div>
      </Panel>
    {/if}
  </SvelteFlow>

  {#if model.nodes.length === 0 && !readonly}
    <div class="wf-empty">
      <div class="wf-empty__ico"><WfIcon name="graph" size={26} /></div>
      <b>{tr('Процесс пуст')}</b>
      <p>{tr('Добавьте первый узел: триггер запускает процесс, дальше идут шаги.')}</p>
      <Button variant="primary" iconName="plus" onclick={openAddMenu}>{tr('Добавить узел')}</Button>
    </div>
  {/if}

  {#if menu}
    <AddMenu allowed={menu.allowed} origin={menu.origin} x={menu.x} y={menu.y} onpick={onPick} onclose={() => (menu = null)} />
  {/if}
  {#if ctxMenu}
    <Menu
      class="nodrag nopan nowheel"
      label={tr('Действия')}
      items={ctxMenu.items.map((it, i) => (it.sep ? { id: `sep${i}`, kind: 'sep' } : { id: it.id, label: it.label, icon: it.icon ? wfIconKey(it.icon) : undefined, danger: it.danger, disabled: it.disabled, shortcut: it.hint }))}
      x={ctxMenu.x}
      y={ctxMenu.y}
      onpick={onCtxPick}
      onclose={() => (ctxMenu = null)}
    />
  {/if}
  {#if panel}
    <HistoryPopover
      {api}
      workflowId={workflow.id}
      mode={panel}
      viewedId={viewing ? run?.id : ''}
      onview={(r) => {
        onview?.(r);
        if (!r) panel = null;
      }}
      onpreview={(v) => {
        panel = null;
        onpreview?.(v);
      }}
      onclose={() => (panel = null)}
    />
  {/if}
  {#if modelPick}
    <ModelPicker
      {...models()}
      value={modelValue}
      startOpen
      bare
      alignLeft
      anchor={modelPick.anchor}
      onselect={(v) => {
        model.setFields(modelPick.id, { provider: v.driver || '', model: v.model || '' });
        modelPick = null;
      }}
      onconnect={() => {
        modelPick = null;
        onconnect?.();
      }}
      onclose={() => (modelPick = null)}
    />
  {/if}
  {#if lightbox}
    <div class="wf-lightbox" role="presentation" onclick={() => (lightbox = '')} onkeydown={(e) => e.key === 'Escape' && (lightbox = '')}>
      <img src={lightbox} alt="" />
    </div>
  {/if}
  {#if viewing && run && !readonly}
    <div class="wf-viewing">
      <WfIcon name="history" size={15} />
      <span>{tr('Показан прошлый запуск')}</span>
      <Button variant="secondary" size="sm" onclick={() => onview?.(null)}>{tr('К текущему')}</Button>
    </div>
  {/if}
</div>

<style>
  .wf-canvas {
    container-type: inline-size;
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--k-canvas);
  }
  .wf-canvas :global(.svelte-flow) {
    --xy-background-color: var(--k-canvas);
    --xy-edge-stroke: var(--k-brand);
    --xy-selection-background-color: color-mix(in oklab, var(--k-brand) 10%, transparent);
    --xy-selection-border: 1.5px solid var(--k-brand);
    --xy-minimap-background-color: var(--k-surface);
    --xy-node-boxshadow-selected: none;
    background: var(--k-canvas);
    font-family: var(--k-font-body);
  }
  .wf-canvas :global(.svelte-flow__node) {
    --xy-node-border-radius: var(--k-radius-inset);
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
    margin: 0 var(--k-space-4) var(--k-space-4) 0;
    border-radius: var(--k-radius-md);
    border: 1px solid var(--k-line);
    box-shadow: var(--k-shadow-2);
    overflow: hidden;
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wf-canvas :global(.svelte-flow__panel) {
    margin: var(--k-space-3);
  }
  .wf-canvas :global(.wf-p-top) {
    margin-top: var(--k-space-3);
  }
  .wf-canvas :global(.wf-p-tools) {
    display: grid;
    justify-items: center;
    gap: var(--k-space-2);
    margin-bottom: var(--k-space-3);
  }
  .wf-bar {
    display: flex;
    align-items: center;
    gap: var(--k-space-1);
    padding: var(--k-space-1) var(--k-space-1) var(--k-space-1) var(--k-space-3);
    border-radius: var(--k-radius-md);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wf-bar--title,
  .wf-bar--zoom {
    padding: var(--k-space-1);
  }
  .wf-title {
    field-sizing: content;
    min-width: 140px;
    max-width: min(34vw, 360px);
    height: 30px;
    padding: 0 var(--k-space-3);
    border: 1px solid transparent;
    border-radius: var(--k-radius-sm);
    background: transparent;
    color: var(--k-text);
    font: 700 var(--k-text-md) / 1 var(--k-font-display);
    transition:
      background var(--k-dur-micro) var(--k-ease-out),
      border-color var(--k-dur-micro) var(--k-ease-out);
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
  .wf-canvas :global(.wf-ib--list) {
    display: none;
  }
  .wf-canvas :global(.wf-ib[aria-pressed='true']),
  .wf-canvas :global(.wf-tool[aria-pressed='true']) {
    color: var(--k-brand-text);
    background: var(--k-surface);
    box-shadow:
      0 0 0 1px color-mix(in oklab, var(--k-brand) 50%, transparent),
      0 2px 10px -4px color-mix(in oklab, var(--k-brand) 60%, transparent);
  }
  .wf-bar :global(.wf-save) {
    padding: 0 var(--k-space-2);
  }
  .wf-runchip {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-2);
    height: 26px;
    margin: 0 var(--k-space-0);
    padding: 0 var(--k-space-3);
    border-radius: var(--k-radius-sm);
    background: var(--k-surface-2);
    font: 700 var(--k-text-xs) / 1 var(--k-font-body);
    color: var(--k-text-2);
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wf-runchip[data-status='running'],
  .wf-runchip[data-status='stopping'] {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wf-runchip[data-status='failed'] {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .wf-runchip[data-status='approval'] {
    background: var(--k-brand);
    color: var(--k-on-brand);
  }
  .wf-runchip__sec {
    font-weight: 600;
    opacity: 0.8;
    font-variant-numeric: tabular-nums;
  }
  .wf-runchip__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--k-ember);
    box-shadow: 0 0 3px 1px color-mix(in oklab, var(--k-brand) 80%, transparent);
    animation: wf-blink 1.2s ease-in-out infinite;
  }
  .wf-bar--zoom {
    gap: var(--k-space-0);
  }
  .wf-bar :global(.wf-zoomval) {
    min-width: 50px;
    font-variant-numeric: tabular-nums;
  }
  .wf-tools {
    display: flex;
    align-items: center;
    gap: var(--k-space-0);
    padding: var(--k-space-1);
    border-radius: var(--k-radius-lg);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wf-tools :global(.wf-tool) {
    width: 40px;
    color: var(--k-text-2);
  }
  .wf-tools :global(.wf-tool--art) {
    width: 46px;
  }
  .wf-tool__t {
    font: 800 var(--k-text-xl) / 1 var(--k-font-display);
  }
  .wf-sep {
    width: 1px;
    height: 22px;
    margin: 0 var(--k-space-1);
    background: var(--k-line);
  }
  .wf-en {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-2);
    padding: 0 var(--k-space-2) 0 var(--k-space-0);
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    color: var(--k-text-2);
    transition: color var(--k-dur-base) var(--k-ease-out);
  }
  .wf-en.off {
    color: var(--k-text-3);
  }
  .wf-lightbox {
    position: absolute;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    background: var(--k-backdrop);
    cursor: zoom-out;
    animation: wf-pop var(--k-dur-appear) var(--k-ease-out) both;
  }
  .wf-lightbox img {
    max-width: 86%;
    max-height: 86%;
    border-radius: var(--k-radius-lg);
    box-shadow: var(--k-shadow-3);
  }
  .wf-search {
    display: flex;
    align-items: center;
    gap: var(--k-space-2);
    width: min(380px, 80vw);
    height: 44px;
    padding: 0 var(--k-space-1) 0 var(--k-space-4);
    border-radius: var(--k-radius-md);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    color: var(--k-text-3);
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
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
    font: 600 var(--k-text-md) / 1 var(--k-font-body);
  }
  .wf-search__n {
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    white-space: nowrap;
  }
  .wf-empty {
    position: absolute;
    inset: 0;
    display: grid;
    align-content: center;
    justify-items: center;
    gap: var(--k-space-2);
    padding: 0 var(--k-space-6) calc(var(--k-space-12) + var(--k-space-8) + var(--k-space-1) * 2.5);
    text-align: center;
    pointer-events: none;
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wf-empty > * {
    pointer-events: auto;
  }
  .wf-empty__ico {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: var(--k-space-2);
    border-radius: var(--k-radius-inset);
    background: var(--k-brand-tint);
    border: 1px solid var(--k-line);
    color: var(--k-brand);
    box-shadow: var(--k-shadow-1);
  }
  .wf-empty b {
    font: 800 var(--k-text-lg) / 1.2 var(--k-font-display);
  }
  .wf-empty p {
    max-width: 320px;
    margin: 0 0 var(--k-space-2);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
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
  .wf-canvas.is-ro :global(.svelte-flow__node) {
    pointer-events: none;
  }
  .wf-canvas :global(.svelte-flow__panel) {
    transition:
      opacity var(--k-dur-base) var(--k-ease-out),
      translate var(--k-dur-ui) var(--k-ease-out);
  }
  .wf-canvas.is-hud-off :global(.svelte-flow__panel) {
    opacity: 0;
    pointer-events: none;
  }
  .wf-canvas.is-hud-off :global(.svelte-flow__panel.top) {
    translate: 0 -12px;
  }
  .wf-canvas.is-hud-off :global(.svelte-flow__panel.bottom) {
    translate: 0 12px;
  }
  .wf-bar--preview {
    white-space: nowrap;
    gap: var(--k-space-3);
    padding: var(--k-space-1) var(--k-space-1) var(--k-space-1) var(--k-space-4);
    color: var(--k-text-2);
  }
  .wf-bar--preview b {
    font: 800 var(--k-text-md) / 1 var(--k-font-display);
    color: var(--k-text);
  }
  .wf-bar__hint {
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wf-viewing {
    position: absolute;
    left: 50%;
    top: 70px;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    padding: var(--k-space-1) var(--k-space-1) var(--k-space-1) var(--k-space-4);
    border-radius: var(--k-radius-pill);
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-2);
    color: var(--k-text-2);
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    translate: -50% 0;
    animation: wf-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  @container (max-width: 1040px) {
    .wf-canvas :global(.wf-p-zoom) {
      margin-bottom: 80px;
    }
    .wf-canvas :global(.svelte-flow__minimap) {
      margin-bottom: 128px !important;
    }
  }
  @media (max-width: 1500px) {
    .wf-en span {
      display: none;
    }
  }
  @media (max-width: 1300px) {
    .wf-bar :global(.wf-save) {
      display: none;
    }
  }
  @media (max-width: 820px) {
    .wf-canvas :global(.wf-ib--list) {
      display: inline-flex;
    }
    .wf-bar :global(.wf-save),
    .wf-tools :global(.wf-add span),
    .wf-bar :global(.wf-run span) {
      display: none;
    }
    .wf-title {
      max-width: 34vw;
      min-width: 90px;
    }
    .wf-canvas :global(.wf-p-zoom) {
      margin-bottom: 80px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-bar,
    .wf-tools,
    .wf-search,
    .wf-empty,
    .wf-runchip,
    .wf-runchip__dot {
      animation: none;
    }
  }
</style>
