// Рабочие процессы: чистые помощники без DOM и без Svelte. Договор данных — notes/workflows-spec.md.
// Граф на диске: { nodes, edges, strokes }. Холст держит узлы Svelte Flow и переводит туда и обратно здесь.

// Ключи вида узла. inputs/outputs — сколько портов (0 или 1), stage — этап, на котором узел оживает.
export const KINDS = {
  trigger: { inputs: 0, outputs: 1, stage: 1, group: 'inputs', tone: 'tangerine' },
  prompt: { inputs: 0, outputs: 1, stage: 1, group: 'inputs', tone: 'sky' },
  asset: { inputs: 0, outputs: 1, stage: 1, group: 'inputs', tone: 'tangerine' },
  bot: { inputs: 1, outputs: 1, stage: 1, group: 'steps', tone: 'mint' },
  llm: { inputs: 1, outputs: 1, stage: 1, group: 'steps', tone: 'violet' },
  image: { inputs: 1, outputs: 1, stage: 1, group: 'steps', tone: 'rose' },
  condition: { inputs: 1, outputs: 1, stage: 1, group: 'flow', tone: 'violet', branches: ['yes', 'no'] },
  approval: { inputs: 1, outputs: 1, stage: 1, group: 'flow', tone: 'tangerine' },
  output: { inputs: 1, outputs: 0, stage: 1, group: 'flow', tone: 'sky' },
  note: { inputs: 0, outputs: 0, stage: 1, group: 'annot', annotation: true },
  text: { inputs: 0, outputs: 0, stage: 1, group: 'annot', annotation: true },
  shape: { inputs: 0, outputs: 0, stage: 1, group: 'annot', annotation: true },
};

// Типы данных на портах: text (серый), image (розовый), file (оранжевый). 'any' на входе принимает всё.
export const PORT_TYPES = ['text', 'image', 'file'];
const IN_TYPES = {
  bot: ['text', 'file', 'image'],
  llm: ['text', 'file', 'image'],
  image: ['text', 'image', 'file'],
  condition: ['text'],
  approval: 'any',
  output: 'any',
};

// Что выдаёт узел. Ресурс: картинка или текст по типу файла, иначе файл. Одобрение и условие не меняют тип: 'any'.
export function outTypeOf(kind, fields = {}) {
  switch (kind) {
    case 'asset': {
      const mime = String(fields.mime || '');
      if (mime.startsWith('image/')) return 'image';
      if (!mime && !fields.path && fields.text) return 'text';
      return fields.path || fields.name ? 'file' : 'text';
    }
    case 'image':
      return 'image';
    case 'approval':
      return 'any';
    default:
      return KINDS[kind]?.outputs ? 'text' : null;
  }
}

export const inTypesOf = (kind) => IN_TYPES[kind] ?? (KINDS[kind]?.inputs ? ['text'] : []);

// Тип данных не подходит к входу (null — подходит).
export function typeProblem(sourceKind, sourceFields, targetKind) {
  const out = outTypeOf(sourceKind, sourceFields);
  const ins = inTypesOf(targetKind);
  if (!out || out === 'any' || ins === 'any') return null;
  return ins.includes(out) ? null : 'type';
}

export const SHAPES = ['rect', 'circle', 'diamond', 'triangle', 'oval', 'star'];
// Кнопки выбора фигуры: перевёрнутый треугольник хранится как треугольник с флагом flip (бэкенд знает шесть фигур).
export const SHAPE_CHOICES = [
  { id: 'rect', shape: 'rect' },
  { id: 'circle', shape: 'circle' },
  { id: 'diamond', shape: 'diamond' },
  { id: 'triangle', shape: 'triangle' },
  { id: 'triangle_down', shape: 'triangle', flip: true },
  { id: 'oval', shape: 'oval' },
  { id: 'star', shape: 'star' },
];
export const NOTE_COLORS = ['lemon', 'tangerine', 'mint', 'sky', 'violet', 'rose', 'ice'];
export const BRUSH_COLORS = ['ink', 'sky', 'mint', 'lemon', 'tangerine', 'rose', 'violet'];
export const BRUSHES = { pen: { width: 3 }, marker: { width: 14 } };

export const ANNOTATION_COLORS = ['lemon', 'mint', 'sky', 'rose', 'violet', 'tangerine', 'ice'];
export const MAX_NODES = 200;
export const MAX_EDGES = 400;
export const MAX_FIELD = 20000;
export const UNDO_LIMIT = 100;

export const isAnnotation = (kind) => Boolean(KINDS[kind]?.annotation);
export const isEnabled = (kind) => Boolean(KINDS[kind]) && KINDS[kind].stage <= 1;

let counter = 0;
export function newId(prefix = 'n') {
  counter += 1;
  const rnd =
    typeof crypto !== 'undefined' && crypto.getRandomValues
      ? Array.from(crypto.getRandomValues(new Uint8Array(5)), (b) => b.toString(36).padStart(2, '0')).join('')
      : Math.random().toString(36).slice(2, 12);
  return `${prefix}_${Date.now().toString(36)}${counter.toString(36)}${rnd}`;
}

export function defaultFields(kind) {
  switch (kind) {
    case 'trigger':
      return { trigger: 'manual' };
    case 'prompt':
      return { text: '' };
    case 'bot':
      return { bot_id: '', task: '' };
    case 'approval':
      return { question: '' };
    case 'output':
      return { target: 'notify' };
    case 'llm':
      return { provider: '', model: '', prompt: '', system: '' };
    case 'asset':
      return { name: '', path: '', mime: '', size: 0 };
    case 'image':
      return { prompt: '', provider: '', model: '', size: '1024x1024' };
    case 'condition':
      return { rule: '', provider: '', model: '' };
    case 'note':
      return { text: '', color: 'lemon' };
    case 'text':
      return { text: '' };
    case 'shape':
      return { shape: 'rect', fill: 'sky', text: '' };
    default:
      return {};
  }
}

export function defaultSchedule() {
  return { kind: 'daily', time: '09:00' };
}

export function makeNode(kind, x, y, extra = {}) {
  const node = { id: newId(), kind, x: Math.round(x), y: Math.round(y), fields: defaultFields(kind), ...extra };
  if (kind === 'note') Object.assign(node, { w: 220, h: 160 });
  if (kind === 'shape') Object.assign(node, { w: 160, h: 120 });
  return node;
}

// Можно ли соединить источник с приёмником (по таблице видов договора).
// Возвращает null, если можно, иначе код причины.
export function connectionProblem(sourceKind, targetKind) {
  const s = KINDS[sourceKind];
  const t = KINDS[targetKind];
  if (!s || !t) return 'unknown';
  if (s.annotation || t.annotation) return 'annotation';
  if (!s.outputs) return 'no_output';
  if (!t.inputs) return 'no_input';
  return null;
}

export function canConnect(nodes, edges, conn) {
  if (!conn?.source || !conn?.target) return false;
  if (conn.source === conn.target) return false;
  const src = nodes.find((n) => n.id === conn.source);
  const dst = nodes.find((n) => n.id === conn.target);
  if (!src || !dst) return false;
  if (connectionProblem(kindOf(src), kindOf(dst))) return false;
  if (typeProblem(kindOf(src), src.data?.fields, kindOf(dst))) return false;
  const branch = conn.sourceHandle && conn.sourceHandle !== 'out' ? conn.sourceHandle : undefined;
  if (kindOf(src) === 'condition' && !branch) return false;
  return !edges.some((e) => e.source === conn.source && e.target === conn.target && (e.data?.branch || undefined) === branch);
}

export const kindOf = (node) => node?.data?.kind ?? node?.kind;

// Какие виды можно создать, если линию потянули от узла `fromKind`
// (side = 'source': тянули с выхода, новый узел получит вход; 'target': наоборот).
export function kindsFor(fromKind, side, fromFields = {}) {
  return Object.keys(KINDS).filter((k) => {
    if (!isEnabled(k) || isAnnotation(k)) return false;
    if (side === 'source') return !connectionProblem(fromKind, k) && !typeProblem(fromKind, fromFields, k);
    return !connectionProblem(k, fromKind) && !typeProblem(k, {}, fromKind);
  });
}

// Что можно вставить в ребро между двумя видами: нужен и вход, и выход.
export function kindsBetween(sourceKind, targetKind) {
  return Object.keys(KINDS).filter((k) => isEnabled(k) && !isAnnotation(k) && !connectionProblem(sourceKind, k) && !connectionProblem(k, targetKind) && !typeProblem(sourceKind, {}, k) && !typeProblem(k, {}, targetKind));
}

// ---- перевод графа <-> узлы и рёбра Svelte Flow ----

export function toFlow(graph) {
  const g = graph || {};
  const nodes = (g.nodes || []).map((n) => {
    const node = {
      id: n.id,
      type: n.kind === 'shape' ? 'wfshape' : isAnnotation(n.kind) ? 'wfnote' : 'wfnode',
      position: { x: Number(n.x) || 0, y: Number(n.y) || 0 },
      data: { kind: n.kind, fields: { ...(n.fields || {}) }, schedule: n.schedule ? { ...n.schedule } : undefined, locked: Boolean(n.locked) },
    };
    if (n.w) node.width = n.w;
    if (n.h) node.height = n.h;
    if (n.locked) node.draggable = false;
    if (n.kind === 'note' || n.kind === 'shape') node.zIndex = -1;
    return node;
  });
  const edges = (g.edges || []).map((e) => ({
    id: e.id,
    source: e.from,
    target: e.to,
    type: 'wf',
    sourceHandle: e.branch || 'out',
    data: e.branch ? { branch: e.branch } : {},
  }));
  return { nodes, edges };
}

export function toGraph(nodes, edges, strokes = []) {
  const outNodes = nodes.map((n) => {
    const kind = kindOf(n);
    const out = {
      id: n.id,
      kind,
      x: Math.round(n.position.x),
      y: Math.round(n.position.y),
      fields: { ...(n.data.fields || {}) },
    };
    const w = n.width ?? n.measured?.width;
    const h = n.height ?? n.measured?.height;
    if (kind === 'note' || kind === 'text' || kind === 'shape') {
      if (w) out.w = Math.round(w);
      if (h) out.h = Math.round(h);
    }
    if (n.data.locked) out.locked = true;
    if (n.data.schedule) out.schedule = { ...n.data.schedule };
    return out;
  });
  const outEdges = edges.map((e) => {
    const edge = { id: e.id, from: e.source, to: e.target };
    const branch = e.data?.branch;
    if (branch) edge.branch = branch;
    return edge;
  });
  return { nodes: outNodes, edges: outEdges, strokes: strokes || [] };
}

// Строка для сравнения «изменилось ли что-то, что надо сохранять»: порядок и размеры стабильны.
export function graphKey(graph) {
  return JSON.stringify({ n: graph.nodes, e: graph.edges, s: graph.strokes });
}

// Те же проверки, что на сервере, но заранее: возвращает список {code, node_id?, edge_id?}.
export function validateGraph(graph) {
  const problems = [];
  const byId = new Map();
  for (const n of graph.nodes || []) {
    if (byId.has(n.id)) problems.push({ code: 'duplicate_node', node_id: n.id });
    byId.set(n.id, n);
  }
  if ((graph.nodes || []).length > MAX_NODES) problems.push({ code: 'too_many_nodes' });
  if ((graph.edges || []).length > MAX_EDGES) problems.push({ code: 'too_many_edges' });
  const seen = new Set();
  for (const e of graph.edges || []) {
    const s = byId.get(e.from);
    const t = byId.get(e.to);
    if (!s || !t) {
      problems.push({ code: 'dangling_edge', edge_id: e.id });
      continue;
    }
    if (connectionProblem(s.kind, t.kind)) problems.push({ code: 'bad_edge', edge_id: e.id });
    if (s.kind === 'condition' && !e.branch) problems.push({ code: 'missing_branch', edge_id: e.id });
    if (s.kind !== 'condition' && e.branch) problems.push({ code: 'unexpected_branch', edge_id: e.id });
    const key = `${e.from}>${e.to}>${e.branch || ''}`;
    if (seen.has(key)) problems.push({ code: 'duplicate_edge', edge_id: e.id });
    seen.add(key);
  }
  return problems;
}

// ---- копирование и вставка ----

// Копия выбранных узлов и рёбер между ними с новыми id и сдвигом.
export function cloneSelection(nodes, edges, ids, dx = 40, dy = 40) {
  const pick = new Set(ids);
  const map = new Map();
  const outNodes = [];
  for (const n of nodes) {
    if (!pick.has(n.id)) continue;
    const id = newId();
    map.set(n.id, id);
    outNodes.push({
      ...n,
      id,
      selected: true,
      position: { x: n.position.x + dx, y: n.position.y + dy },
      data: { ...n.data, fields: { ...n.data.fields }, schedule: n.data.schedule ? { ...n.data.schedule } : undefined },
    });
  }
  const outEdges = edges
    .filter((e) => map.has(e.source) && map.has(e.target))
    .map((e) => ({ ...e, id: newId('e'), source: map.get(e.source), target: map.get(e.target), selected: false }));
  return { nodes: outNodes, edges: outEdges };
}

// ---- запуск ----

export const RUN_OPEN = ['running', 'approval', 'stopping'];
export const isRunOpen = (run) => Boolean(run) && RUN_OPEN.includes(run.status);

// Время с сервера: секунды, миллисекунды или строка ISO.
export function toMs(v) {
  if (v == null || v === '') return 0;
  if (typeof v === 'number') return v < 1e12 ? v * 1000 : v;
  const n = Number(v);
  if (Number.isFinite(n) && String(v).trim() !== '') return n < 1e12 ? n * 1000 : n;
  const t = Date.parse(String(v));
  return Number.isNaN(t) ? 0 : t;
}

// Последний шаг каждого узла (по попытке, затем по времени старта).
export function latestSteps(run) {
  const map = new Map();
  for (const s of run?.steps || []) {
    const prev = map.get(s.node_id);
    if (!prev || (s.attempt || 0) > (prev.attempt || 0) || ((s.attempt || 0) === (prev.attempt || 0) && toMs(s.started_at) >= toMs(prev.started_at))) {
      map.set(s.node_id, s);
    }
  }
  return map;
}

export function stepSeconds(step, now = Date.now()) {
  const a = toMs(step?.started_at);
  if (!a) return 0;
  const b = toMs(step?.ended_at) || now;
  return Math.max(0, Math.round((b - a) / 100) / 10);
}

// Ребро «течёт», если из него вышел шаг в работе, или оно ведёт в работающий шаг.
export function edgeFlow(edge, steps) {
  const from = steps.get(edge.source);
  // Условие выбрало другую ветку: эта линия в запуске не участвовала.
  if (from?.branch && edge.branch && from.status === 'done' && from.branch !== edge.branch) return 'off';
  const to = steps.get(edge.target);
  if (to && (to.status === 'running' || to.status === 'approval') && from && from.status === 'done') return 'live';
  if (from && from.status === 'done' && to && to.status === 'done') return 'done';
  if (from && from.status === 'done') return 'pending';
  return 'idle';
}

// ---- расписание ----

export const SCHEDULE_KINDS = ['hourly', 'daily', 'weekdays', 'weekly', 'monthly', 'interval', 'once'];

export function scheduleDefaults(kind) {
  switch (kind) {
    case 'hourly':
      return { kind, minute: 0 };
    case 'daily':
    case 'weekdays':
      return { kind, time: '09:00' };
    case 'weekly':
      return { kind, time: '09:00', weekday: 1 };
    case 'monthly':
      return { kind, time: '09:00', day: 1 };
    case 'interval':
      return { kind, amount: 30, unit: 'minutes' };
    case 'once': {
      const d = new Date(Date.now() + 86400000);
      return { kind, time: '09:00', date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}` };
    }
    default:
      return { kind: 'daily', time: '09:00' };
  }
}

// Интервал не короче 3 минут (договор).
export function normalizeSchedule(s) {
  const out = { ...s };
  if (out.kind === 'interval') {
    let amount = Math.max(1, Math.round(Number(out.amount) || 1));
    if (out.unit === 'minutes' && amount < 3) amount = 3;
    out.amount = amount;
    if (!['minutes', 'hours', 'days'].includes(out.unit)) out.unit = 'minutes';
  }
  if (out.kind === 'hourly') out.minute = Math.min(59, Math.max(0, Math.round(Number(out.minute) || 0)));
  if (out.kind === 'monthly') out.day = Math.min(31, Math.max(1, Math.round(Number(out.day) || 1)));
  if (out.kind === 'weekly') out.weekday = Math.min(7, Math.max(1, Math.round(Number(out.weekday) || 1)));
  return out;
}

// Заготовка нового процесса: триггер → бот → уведомление, чтобы холст не был пустым.
export function starterGraph() {
  const t = makeNode('trigger', 0, 0);
  const b = makeNode('bot', 400, -20);
  const o = makeNode('output', 800, 0);
  return {
    nodes: [t, b, o],
    edges: [
      { id: newId('e'), from: t.id, to: b.id },
      { id: newId('e'), from: b.id, to: o.id },
    ],
    strokes: [],
  };
}

// Виды запуска и места вывода: порядок и значки для узлов «Триггер» и «Вывод».
export const TRIGGER_KINDS = [
  { id: 'manual', icon: 'manual' },
  { id: 'schedule', icon: 'calendar' },
  { id: 'webhook', icon: 'webhook' },
  { id: 'telegram', icon: 'telegram' },
  { id: 'discord', icon: 'discord' },
  { id: 'slack', icon: 'slack' },
];
export const OUTPUT_TARGETS = [
  { id: 'notify', icon: 'bell' },
  { id: 'chat', icon: 'chat' },
  { id: 'telegram', icon: 'telegram' },
  { id: 'discord', icon: 'discord' },
  { id: 'slack', icon: 'slack' },
  { id: 'file', icon: 'file' },
];
export const MESSENGERS = ['telegram', 'discord', 'slack'];
export const isMessenger = (id) => MESSENGERS.includes(id);

/** Подключён ли мессенджер: connections {telegram, discord, slack: bool}. Для остальных видов — всегда да. */
export function isLinked(id, connections = {}) {
  return !isMessenger(id) || Boolean(connections[id]);
}

export const triggerOf = (fields = {}) => fields.trigger || 'manual';
export const targetOf = (fields = {}) => fields.target || 'notify';
