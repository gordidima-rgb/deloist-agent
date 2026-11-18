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

// Правило сервера → состояние редактора { preset, weekdays, monthday, by, nth, nthDay, month }.
// by: 'date' (число месяца) или 'nth' (N-й день недели месяца). Всё, что не из списка, остаётся 'custom'.
export function editorFromRecurrence(rule) {
  const base = { weekdays: [], monthday: null, by: 'date', nth: 1, nthDay: null, month: null };
  if (!rule) return { ...base, preset: 'none' };
  const weekdays = sortedDays(rule.weekdays);
  const interval = Number(rule.interval) || 1;
  const monthday = rule.monthday ?? null;
  if (rule.freq === 'daily' && interval === 1) return { ...base, preset: 'daily' };
  if (rule.freq === 'weekly' && interval === 1 && sameDays(weekdays, WORKDAYS)) return { ...base, preset: 'weekdays', weekdays };
  if (rule.freq === 'weekly' && interval === 1) return { ...base, preset: 'weekly', weekdays };
  if (rule.freq === 'weekly' && interval === 2) return { ...base, preset: 'biweekly', weekdays };
  if ((rule.freq === 'monthly' || rule.freq === 'yearly') && interval === 1) {
    const nth = rule.nth ? { by: 'nth', nth: rule.nth, nthDay: weekdays[0] ?? null } : { monthday };
    return { ...base, ...nth, preset: rule.freq, month: rule.month ?? null };
  }
  return { ...base, preset: 'custom', weekdays, monthday, rule };
}

// Состояние редактора → правило для сервера (или null). У недельных без выбранных дней берётся день срока (1 = пн … 7 = вс).
// Месяц и год без уточнений остаются простыми: число или день недели берётся со срока.
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
  const freq = preset === 'monthly' ? 'monthly' : 'yearly';
  const month = freq === 'yearly' ? { month: editor.month ?? monthOf(dueDate) } : {};
  if (editor.by === 'nth') {
    return { freq, interval: 1, weekdays: [editor.nthDay ?? weekdayOf(dueDate)], monthday: null, nth: editor.nth || 1, ...month };
  }
  const day = Number(editor.monthday);
  const monthday = day >= 1 && day <= 31 ? day : null;
  if (freq === 'monthly') return { freq, interval: 1, weekdays: [], monthday };
  if (editor.month == null && monthday == null) return { freq, interval: 1, weekdays: [], monthday: null };
  return { freq, interval: 1, weekdays: [], monthday: monthday ?? dayOf(dueDate), ...month };
}

// Месяц (1..12) и число даты ГГГГ-ММ-ДД; без даты — сегодняшние.
const dateOrToday = (iso) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(iso ?? ''));
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : new Date();
};
export const monthOf = (iso) => dateOrToday(iso).getMonth() + 1;
export const dayOf = (iso) => dateOrToday(iso).getDate();

// День недели даты ГГГГ-ММ-ДД: 1 = понедельник … 7 = воскресенье; без даты — сегодняшний.
export const weekdayOf = (iso) => ((dateOrToday(iso).getDay() + 6) % 7) + 1;

export function toggleWeekday(days, day) {
  const has = days.includes(day);
  return sortedDays(has ? days.filter((d) => d !== day) : [...days, day]);
}

const monthName = (month, locale, style) =>
  new Intl.DateTimeFormat(locale, style === 'of' ? { day: 'numeric', month: 'long' } : { month: 'long' })
    .formatToParts(new Date(2026, month - 1, 15))
    .find((p) => p.type === 'month').value;
// «сентябрь» для списка и «сентября» для фразы.
export const monthLabel = (month, locale) => monthName(month, locale, 'name');

// Род дня недели решает, как согласуются «каждый/каждую/каждое» и порядковое: 3-й вторник, 3-ю субботу, 3-е воскресенье.
const WEEKDAY_GENDER = ['m', 'm', 'f', 'm', 'f', 'f', 'n'];

// Фраза про правило: «каждую 3-ю субботу сентября». t — tr(); locale — для названий месяцев.
export function recurrenceSummary(rule, t, locale) {
  if (!rule) return '';
  const interval = Number(rule.interval) || 1;
  const days = sortedDays(rule.weekdays);
  const names = [t('Пн'), t('Вт'), t('Ср'), t('Чт'), t('Пт'), t('Сб'), t('Вс')];
  const list = days.map((d) => names[d - 1]).join(', ');
  if (rule.freq === 'daily') return interval === 1 ? t('каждый день') : t('каждые {n} дн.', { n: interval });
  if (rule.freq === 'weekly') {
    if (interval === 1 && sameDays(days, WORKDAYS)) return t('по будням');
    if (interval === 1 && days.length === 1) {
      return [t('каждый понедельник'), t('каждый вторник'), t('каждую среду'), t('каждый четверг'), t('каждую пятницу'), t('каждую субботу'), t('каждое воскресенье')][days[0] - 1];
    }
    return interval === 1 ? t('каждую неделю: {days}', { days: list }) : t('раз в {n} нед.: {days}', { n: interval, days: list });
  }
  const yearly = rule.freq === 'yearly';
  const month = yearly && rule.month ? monthName(rule.month, locale, 'of') : '';
  if (rule.nth && days.length === 1) {
    const g = WEEKDAY_GENDER[days[0] - 1];
    const each = { m: t('каждый'), f: t('каждую'), n: t('каждое') }[g];
    const ords = {
      m: [t('1-й'), t('2-й'), t('3-й'), t('4-й')],
      f: [t('1-ю'), t('2-ю'), t('3-ю'), t('4-ю')],
      n: [t('1-е'), t('2-е'), t('3-е'), t('4-е')],
    }[g];
    const nth = rule.nth === -1 ? { m: t('последний'), f: t('последнюю'), n: t('последнее') }[g] : ords[rule.nth - 1];
    const day = [t('понедельник'), t('вторник'), t('среду'), t('четверг'), t('пятницу'), t('субботу'), t('воскресенье')][days[0] - 1];
    return month
      ? t('{each} {nth} {day} {month}', { each, nth, day, month })
      : t('{each} {nth} {day} месяца', { each, nth, day });
  }
  if (rule.freq === 'monthly') {
    if (interval !== 1) return t('раз в {n} мес.', { n: interval });
    return rule.monthday ? t('каждый месяц, {day}-го числа', { day: rule.monthday }) : t('каждый месяц');
  }
  if (rule.freq === 'yearly') {
    if (interval !== 1) return t('раз в {n} г.', { n: interval });
    return month && rule.monthday ? t('каждый год, {day} {month}', { day: rule.monthday, month }) : t('каждый год');
  }
  return t('повторяется');
}
