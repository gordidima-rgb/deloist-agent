// Чистые правила экрана «Поручения» и раздела «Сделано, пока тебя не было» (данные — GET /api/tasks).
const WEEK = 7 * 24 * 3600 * 1000;
export const OPEN_STATES = ['queued', 'running', 'paused'];

export const isOpen = (task) => OPEN_STATES.includes(task.state);

// 'open' — то, что ещё идёт; 'week' — всё, что выдано или закрыто за последние 7 дней. Новые сверху.
export function filterTasks(tasks, mode, now = Date.now()) {
  const keep = mode === 'open' ? isOpen : (task) => isOpen(task) || Math.max(task.created || 0, task.finished || 0) >= now - WEEK;
  return tasks.filter(keep).sort((a, b) => (b.created || 0) - (a.created || 0));
}

// Закрытые задачи после отметки `since` (мс). 0 — человек уже всё посмотрел; не задана — берём последние 7 дней.
export function awayTasks(tasks, since, now = Date.now()) {
  if (since === 0) return [];
  const from = since ?? now - WEEK;
  return tasks.filter((task) => !isOpen(task) && task.finished > from).sort((a, b) => b.finished - a.finished);
}

// Окно скрылось: отметка «ушёл» ставится, только если прошлую уже просмотрели (0); непросмотренная остаётся самой ранней.
export const leftAt = (stored, now = Date.now()) => (stored === 0 ? now : stored);
