// Логика доски задач (TodosTab.svelte): группировка, фильтры, перенос, правило повтора.
// Чистые функции без DOM и сети; тексты строит компонент через tr(). Даты — ГГГГ-ММ-ДД, сравнение строкой хронологическое.
import { isoOf } from './composer.js';

export const BUCKETS = ['key', 'active', 'someday'];
export const FILTERS = ['all', 'today', 'overdue', 'nodue'];

// Чьи задачи показывать: свои (без исполнителя), ботов (исполнитель — бот) или все.
export const OWNERS = ['mine', 'bots', 'all'];

export function filterByOwner(todos, owner) {
  if (owner === 'mine') return (todos ?? []).filter((t) => !t.assignee);
  if (owner === 'bots') return (todos ?? []).filter((t) => t.assignee);
  return todos ?? [];
}

export const todayIso = (now = new Date()) => isoOf(now);

export function addDays(iso, days) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(iso ?? ''));
  if (!m) return '';
  return isoOf(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + days));
}

export const isOpen = (todo) => !todo.done && !todo.archived;
export const isOverdue = (todo, today) => isOpen(todo) && Boolean(todo.due_date) && todo.due_date < today;

export function matchesFilter(todo, filter, today) {
  if (filter === 'today') return todo.due_date === today;
  if (filter === 'overdue') return Boolean(todo.due_date) && todo.due_date < today;
  if (filter === 'nodue') return !todo.due_date;
  return true;
}

// Поиск по названию, заметке и пунктам чек-листа без учёта регистра.
export function matchesQuery(todo, query) {
  const needle = String(query ?? '').trim().toLowerCase();
  if (!needle) return true;
  const hay = [todo.title, todo.notes, ...(todo.items ?? []).map((i) => i.text)].join('\n').toLowerCase();
  return hay.includes(needle);
}

// Три раздела доски; в каждом открытые задачи и сворачиваемый список выполненных. Порядок внутри — как отдал сервер.
export function groupBoard(todos, { filter = 'all', query = '', today = todayIso() } = {}) {
  const board = { key: { open: [], done: [] }, active: { open: [], done: [] }, someday: { open: [], done: [] } };
  for (const todo of todos ?? []) {
    if (todo.archived) continue;
    if (!matchesFilter(todo, filter, today) || !matchesQuery(todo, query)) continue;
    const section = board[todo.bucket] ?? board.active;
    (todo.done ? section.done : section.open).push(todo);
  }
  return board;
}

// Числа на кнопках фильтров считаются по открытым задачам.
export function filterCounts(todos, today = todayIso()) {
  const open = (todos ?? []).filter(isOpen);
  return Object.fromEntries(FILTERS.map((f) => [f, open.filter((t) => matchesFilter(t, f, today)).length]));
}

// Что перенесёт «Перенести на сегодня»: открытые просроченные задачи без повтора (как carry_over_candidate на сервере).
export function carryOverCandidates(todos, today = todayIso()) {
  return (todos ?? []).filter((t) => isOpen(t) && !t.recurrence && isOverdue(t, today));
}

export function nextBucket(bucket) {
  return BUCKETS[(BUCKETS.indexOf(bucket) + 1) % BUCKETS.length] ?? 'active';
}

export function checklistProgress(todo) {
  const items = todo.items ?? [];
  return { done: items.filter((i) => i.done).length, total: items.length };
}

// Срок для чипа: когда относительно сегодняшнего дня и время, если есть.
export function dueInfo(todo, today = todayIso()) {
  if (!todo.due_date) return null;
  let when = 'date';
  if (todo.due_date === today) when = 'today';
  else if (todo.due_date === addDays(today, 1)) when = 'tomorrow';
  else if (todo.due_date === addDays(today, -1)) when = 'yesterday';
  return { when, date: todo.due_date, time: todo.due_time || '', overdue: isOverdue(todo, today) };
}

// ---- повтор ----

export const PRESETS = ['none', 'daily', 'weekdays', 'weekly', 'biweekly', 'monthly', 'yearly'];

const WORKDAYS = [1, 2, 3, 4, 5];

const sameDays = (a, b) => a.length === b.length && a.every((d, i) => d === b[i]);
const sortedDays = (days) => [...new Set((days ?? []).map(Number).filter((d) => d >= 1 && d <= 7))].sort((a, b) => a - b);

// Правило сервера → состояние редактора { preset, weekdays, monthday }. Всё, что не из списка, остаётся 'custom'.
export function editorFromRecurrence(rule) {
  if (!rule) return { preset: 'none', weekdays: [], monthday: null };
  const weekdays = sortedDays(rule.weekdays);
  const interval = Number(rule.interval) || 1;
  const monthday = rule.monthday ?? null;
  if (rule.freq === 'daily' && interval === 1) return { preset: 'daily', weekdays: [], monthday: null };
  if (rule.freq === 'weekly' && interval === 1 && sameDays(weekdays, WORKDAYS)) return { preset: 'weekdays', weekdays, monthday: null };
  if (rule.freq === 'weekly' && interval === 1) return { preset: 'weekly', weekdays, monthday: null };
  if (rule.freq === 'weekly' && interval === 2) return { preset: 'biweekly', weekdays, monthday: null };
  if (rule.freq === 'monthly' && interval === 1) return { preset: 'monthly', weekdays: [], monthday };
  if (rule.freq === 'yearly' && interval === 1) return { preset: 'yearly', weekdays: [], monthday: null };
  return { preset: 'custom', weekdays, monthday, rule };
}

// Состояние редактора → правило для сервера (или null). У недельных без выбранных дней берётся день срока (1 = пн … 7 = вс).
export function recurrenceFromEditor(editor, dueDate) {
  const { preset } = editor;
  if (preset === 'none') return null;
  if (preset === 'custom') return editor.rule ?? null;
  if (preset === 'daily') return { freq: 'daily', interval: 1, weekdays: [], monthday: null };
  if (preset === 'weekdays') return { freq: 'weekly', interval: 1, weekdays: WORKDAYS, monthday: null };
  if (preset === 'weekly' || preset === 'biweekly') {
    let weekdays = sortedDays(editor.weekdays);
    if (!weekdays.length) weekdays = [weekdayOf(dueDate)];
    return { freq: 'weekly', interval: preset === 'weekly' ? 1 : 2, weekdays, monthday: null };
  }
  if (preset === 'monthly') {
    const day = Number(editor.monthday);
    return { freq: 'monthly', interval: 1, weekdays: [], monthday: day >= 1 && day <= 31 ? day : null };
  }
  return { freq: 'yearly', interval: 1, weekdays: [], monthday: null };
}

// День недели даты ГГГГ-ММ-ДД: 1 = понедельник … 7 = воскресенье; без даты — сегодняшний.
export function weekdayOf(iso) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(iso ?? ''));
  const date = m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : new Date();
  return ((date.getDay() + 6) % 7) + 1;
}

export function toggleWeekday(days, day) {
  const has = days.includes(day);
  return sortedDays(has ? days.filter((d) => d !== day) : [...days, day]);
}
