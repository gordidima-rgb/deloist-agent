// Состояние холста: узлы и рёбра Svelte Flow, отмена/повтор и операции над выбранным.
// Холст и карточки узлов получают модель через контекст; сохранение вешается на onchange.
import { History } from './history.js';
import {
  UNDO_LIMIT,
  MAX_NODES,
  MAX_EDGES,
  canConnect,
  cloneSelection,
  graphKey,
  kindOf,
  makeNode,
  newId,
  toFlow,
  toGraph,
  connectionProblem,
} from './graph.js';

export class CanvasModel {
  nodes = $state.raw([]);
  edges = $state.raw([]);
  strokes = [];
  canUndo = $state(false);
  canRedo = $state(false);
  // Узел или ребро, на которые указал сервер при отказе сохранить.
  invalid = $state.raw({ node_id: '', edge_id: '' });

  #history = new History(UNDO_LIMIT);
  #onchange;
  #typingTimer = 0;
  #typingKey = '';

  constructor({ onchange } = {}) {
    this.#onchange = onchange;
  }

  load(graph) {
    const flow = toFlow(graph);
    this.nodes = flow.nodes;
    this.edges = flow.edges;
    this.strokes = graph?.strokes || [];
    this.#history.reset(this.key());
    this.#flags();
  }

  getGraph() {
    return toGraph(this.nodes, this.edges, this.strokes);
  }

  key() {
    return graphKey(this.getGraph());
  }

  #flags() {
    this.canUndo = this.#history.canUndo;
    this.canRedo = this.#history.canRedo;
  }

  // Правка завершена: запомнить шаг отмены и сообщить, что пора сохранять.
  commit() {
    clearTimeout(this.#typingTimer);
    this.#typingTimer = 0;
    this.#typingKey = '';
    if (this.#history.push(this.key())) {
      this.#flags();
      this.#onchange?.();
    }
  }

  // Размеры меняются при перетаскивании и растягивании: перерасчёт без шага отмены.
  #apply(snapshotKey) {
    const parsed = JSON.parse(snapshotKey);
    const flow = toFlow({ nodes: parsed.n, edges: parsed.e, strokes: parsed.s });
    // Выделение и размеры сохраняем по id: отмена не должна сбрасывать картинку.
    const selected = new Set(this.nodes.filter((n) => n.selected).map((n) => n.id));
    this.nodes = flow.nodes.map((n) => (selected.has(n.id) ? { ...n, selected: true } : n));
    this.edges = flow.edges;
    this.strokes = parsed.s || [];
    this.#flags();
    this.#onchange?.();
  }

  undo() {
    this.#flush();
    const snap = this.#history.undo();
    if (snap == null) return false;
    this.#apply(snap);
    return true;
  }

  redo() {
    this.#flush();
    const snap = this.#history.redo();
    if (snap == null) return false;
    this.#apply(snap);
    return true;
  }

  #flush() {
    if (this.#typingTimer) this.commit();
  }

  // ---- узлы ----

  addNode(kind, x, y, extra = {}, { commit = true } = {}) {
    if (this.nodes.length >= MAX_NODES) return null;
    const raw = makeNode(kind, x, y, extra);
    const flow = toFlow({ nodes: [raw], edges: [] }).nodes[0];
    this.nodes = [...this.nodes.map((n) => (n.selected ? { ...n, selected: false } : n)), { ...flow, selected: true }];
    if (commit) this.commit();
    return flow;
  }

  patchData(id, patch) {
    this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...patch } } : n));
  }

  // Поле узла. Набор текста склеивается в один шаг отмены: коммит через паузу.
  setField(id, key, value) {
    this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, fields: { ...n.data.fields, [key]: value } } } : n));
    const tag = `${id}:${key}`;
    if (this.#typingKey && this.#typingKey !== tag) this.commit();
    this.#typingKey = tag;
    clearTimeout(this.#typingTimer);
    this.#typingTimer = setTimeout(() => this.commit(), 600);
  }

  // Несколько полей сразу (выбор вида триггера, цвет стикера): отдельный шаг.
  setFields(id, patch) {
    this.#flush();
    this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, fields: { ...n.data.fields, ...patch } } } : n));
    this.commit();
  }

  setSchedule(id, schedule) {
    this.#flush();
    this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, schedule } } : n));
    this.commit();
  }

  setSize(id, width, height) {
    this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, width, height } : n));
  }

  toggleLock(id) {
    this.nodes = this.nodes.map((n) =>
      n.id === id ? { ...n, draggable: n.data.locked ? undefined : false, data: { ...n.data, locked: !n.data.locked } } : n,
    );
    this.commit();
  }

  get selectedIds() {
    return this.nodes.filter((n) => n.selected).map((n) => n.id);
  }

  selectAll() {
    this.nodes = this.nodes.map((n) => (n.selected ? n : { ...n, selected: true }));
  }

  clearSelection() {
    if (!this.nodes.some((n) => n.selected) && !this.edges.some((e) => e.selected)) return;
    this.nodes = this.nodes.map((n) => (n.selected ? { ...n, selected: false } : n));
    this.edges = this.edges.map((e) => (e.selected ? { ...e, selected: false } : e));
  }

  removeSelected() {
    const ids = new Set(this.selectedIds);
    const edgeIds = new Set(this.edges.filter((e) => e.selected).map((e) => e.id));
    if (!ids.size && !edgeIds.size) return false;
    this.nodes = this.nodes.filter((n) => !ids.has(n.id));
    this.edges = this.edges.filter((e) => !edgeIds.has(e.id) && !ids.has(e.source) && !ids.has(e.target));
    this.commit();
    return true;
  }

  duplicateSelected() {
    const ids = this.selectedIds;
    if (!ids.length || this.nodes.length + ids.length > MAX_NODES) return [];
    const copy = cloneSelection(this.nodes, this.edges, ids);
    this.nodes = [...this.nodes.map((n) => (n.selected ? { ...n, selected: false } : n)), ...copy.nodes];
    this.edges = [...this.edges, ...copy.edges];
    this.commit();
    return copy.nodes.map((n) => n.id);
  }

  // ---- рёбра ----

  // Соединение из холста. Возвращает {edge} или {problem}.
  connect(conn) {
    const src = this.nodes.find((n) => n.id === conn.source);
    const dst = this.nodes.find((n) => n.id === conn.target);
    if (!src || !dst) return { problem: 'unknown' };
    const why = connectionProblem(kindOf(src), kindOf(dst));
    if (why) return { problem: why };
    if (!canConnect(this.nodes, this.edges, conn)) return { problem: conn.source === conn.target ? 'self' : 'duplicate' };
    if (this.edges.length >= MAX_EDGES) return { problem: 'limit' };
    const branch = conn.sourceHandle && conn.sourceHandle !== 'out' ? conn.sourceHandle : undefined;
    const edge = {
      id: newId('e'),
      source: conn.source,
      target: conn.target,
      type: 'wf',
      sourceHandle: conn.sourceHandle || 'out',
      data: branch ? { branch } : {},
    };
    return { edge };
  }

  addEdge(edge, { commit = true } = {}) {
    this.edges = [...this.edges, edge];
    if (commit) this.commit();
  }

  removeEdge(id) {
    this.edges = this.edges.filter((e) => e.id !== id);
    this.commit();
  }

  // Узел, найденный по тексту: для поиска на холсте.
  search(query) {
    const q = String(query || '').trim().toLowerCase();
    if (!q) return [];
    return this.nodes.filter((n) => {
      const f = n.data.fields || {};
      const hay = [n.data.kind, ...Object.values(f).map(String)].join(' ').toLowerCase();
      return hay.includes(q);
    });
  }
}
