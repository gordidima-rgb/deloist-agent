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
  typeProblem,
  kindsBetween,
} from './graph.js';
import { MAX_STROKES, limitPoints } from './strokes.js';

export class CanvasModel {
  nodes = $state.raw([]);
  edges = $state.raw([]);
  strokes = $state.raw([]);
  // Буфер обмена внутри окна: копия узлов и связей между ними.
  clipboard = null;
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
    // Уходящие узлы (затухают после чужой правки) в граф не попадают.
    return toGraph(
      this.nodes.filter((n) => !n.data?.leaving),
      this.edges,
      this.strokes,
    );
  }

  /**
   * Чужая правка (бот или другое окно): заменить граф на месте. Новые узлы появляются с анимацией входа,
   * изменённые коротко подсвечиваются, удалённые остаются на 380 мс и тают. Возвращает {added, changed, removed}.
   */
  applyRemote(graph, { leaveMs = 380, flashMs = 1600 } = {}) {
    const next = toFlow(graph);
    const before = new Map(this.nodes.filter((n) => !n.data?.leaving).map((n) => [n.id, n]));
    const keep = new Set(this.nodes.filter((n) => n.selected).map((n) => n.id));
    const gone = [...before.values()].filter((n) => !next.nodes.some((m) => m.id === n.id));
    const stamp = (n) => JSON.stringify([n.position, n.data.fields, n.data.schedule, n.data.locked]);
    const changed = [];
    const nodes = next.nodes.map((n) => {
      const old = before.get(n.id);
      const out = keep.has(n.id) ? { ...n, selected: true } : n;
      if (old && stamp(old) !== stamp(n)) {
        changed.push(n.id);
        return { ...out, data: { ...out.data, flash: true } };
      }
      return out;
    });
    const ghosts = gone.map((n) => ({ ...n, selected: false, draggable: false, selectable: false, deletable: false, connectable: false, data: { ...n.data, leaving: true } }));
    this.nodes = [...nodes, ...ghosts];
    this.edges = next.edges.filter((e) => !ghosts.some((g) => g.id === e.source || g.id === e.target));
    this.strokes = graph?.strokes || [];
    this.#history.reset(this.key());
    this.#flags();
    const drop = () => {
      if (ghosts.length) this.nodes = this.nodes.filter((n) => !n.data?.leaving);
    };
    const calm = () => {
      if (changed.length) this.nodes = this.nodes.map((n) => (n.data?.flash ? { ...n, data: { ...n.data, flash: false } } : n));
    };
    setTimeout(drop, leaveMs);
    setTimeout(calm, flashMs);
    return { added: next.nodes.filter((n) => !before.has(n.id)).map((n) => n.id), changed, removed: gone.map((n) => n.id) };
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
    const mismatch = typeProblem(kindOf(src), src.data.fields, kindOf(dst));
    if (mismatch) return { problem: mismatch };
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

  // ---- штрихи ----

  addStroke(stroke) {
    if (this.strokes.length >= MAX_STROKES) return false;
    this.strokes = [...this.strokes, { ...stroke, points: limitPoints(stroke.points) }];
    this.commit();
    return true;
  }

  removeStrokes(ids) {
    const gone = new Set(ids);
    if (!this.strokes.some((s) => gone.has(s.id))) return;
    this.strokes = this.strokes.filter((s) => !gone.has(s.id));
    this.commit();
  }

  // ---- буфер обмена ----

  copy() {
    const ids = this.selectedIds;
    if (!ids.length) return false;
    const c = cloneSelection(this.nodes, this.edges, ids, 0, 0);
    this.clipboard = { nodes: c.nodes, edges: c.edges, pasted: 0 };
    return true;
  }

  cut() {
    if (!this.copy()) return false;
    this.removeSelected();
    return true;
  }

  // Вставка: если задано `at` (точка потока), левый верхний угол группы встаёт туда, иначе сдвиг на 40 px при каждой вставке.
  paste(at = null) {
    const clip = this.clipboard;
    if (!clip || !clip.nodes.length) return [];
    if (this.nodes.length + clip.nodes.length > MAX_NODES) return [];
    clip.pasted += 1;
    const minX = Math.min(...clip.nodes.map((n) => n.position.x));
    const minY = Math.min(...clip.nodes.map((n) => n.position.y));
    const dx = at ? at.x - minX : 40 * clip.pasted;
    const dy = at ? at.y - minY : 40 * clip.pasted;
    const ids = new Map();
    const nodes = clip.nodes.map((n) => {
      const id = newId();
      ids.set(n.id, id);
      return { ...n, id, selected: true, position: { x: n.position.x + dx, y: n.position.y + dy }, data: { ...n.data, fields: { ...n.data.fields }, schedule: n.data.schedule ? { ...n.data.schedule } : undefined } };
    });
    const edges = clip.edges.map((e) => ({ ...e, id: newId('e'), source: ids.get(e.source), target: ids.get(e.target), selected: false }));
    this.nodes = [...this.nodes.map((n) => (n.selected ? { ...n, selected: false } : n)), ...nodes];
    this.edges = [...this.edges, ...edges];
    this.commit();
    return nodes.map((n) => n.id);
  }

  // ---- порядок, размер, вставка в ребро ----

  bringToFront(id) {
    const n = this.nodes.find((x) => x.id === id);
    if (!n) return;
    this.nodes = [...this.nodes.filter((x) => x.id !== id), n];
    this.commit();
  }

  sendToBack(id) {
    const n = this.nodes.find((x) => x.id === id);
    if (!n) return;
    this.nodes = [n, ...this.nodes.filter((x) => x.id !== id)];
    this.commit();
  }

  resetSize(id) {
    this.nodes = this.nodes.map((n) => {
      if (n.id !== id) return n;
      const { width, height, ...rest } = n;
      return { ...rest, ...(n.data.kind === 'note' ? { width: 220, height: 160 } : n.data.kind === 'shape' ? { width: 160, height: 120 } : {}) };
    });
    this.commit();
  }

  // Новый узел посреди ребра: ребро превращается в два (ветка условия остаётся у первого).
  insertIntoEdge(edgeId, kind) {
    const e = this.edges.find((x) => x.id === edgeId);
    const a = e && this.nodes.find((n) => n.id === e.source);
    const b = e && this.nodes.find((n) => n.id === e.target);
    if (!a || !b || !kindsBetween(kindOf(a), kindOf(b)).includes(kind)) return null;
    if (this.nodes.length >= MAX_NODES || this.edges.length + 1 > MAX_EDGES) return null;
    const mid = (n) => ({ x: n.position.x + (n.measured?.width || n.width || 288) / 2, y: n.position.y + (n.measured?.height || n.height || 120) / 2 });
    const ma = mid(a);
    const mb = mid(b);
    const node = this.addNode(kind, (ma.x + mb.x) / 2 - 144, (ma.y + mb.y) / 2 - 60, {}, { commit: false });
    const first = { ...e, target: node.id };
    const second = { id: newId('e'), source: node.id, target: b.id, type: 'wf', sourceHandle: kind === 'condition' ? 'yes' : 'out', data: kind === 'condition' ? { branch: 'yes' } : {} };
    this.edges = [...this.edges.filter((x) => x.id !== e.id), first, second];
    this.commit();
    return node;
  }

  // Следующий узел для Tab: слева направо, сверху вниз (только то, что связано шагами, аннотации тоже).
  neighbour(dir = 1) {
    const list = [...this.nodes].sort((p, q) => p.position.x - q.position.x || p.position.y - q.position.y);
    if (!list.length) return null;
    const at = list.findIndex((n) => n.selected);
    if (at < 0) return dir > 0 ? list[0] : list[list.length - 1];
    return list[(at + dir + list.length) % list.length];
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
