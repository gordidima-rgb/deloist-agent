// Чистые помощники экрана «Использование и лимиты»: месяцы, доли, прогноз, цвета рядов.

export function monthKey(date = new Date()) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;
}

// Сдвиг месяца 'YYYY-MM' на delta месяцев.
export function shiftMonth(month, delta) {
  const [y, m] = month.split('-').map(Number);
  const d = new Date(y, m - 1 + delta, 1);
  return monthKey(d);
}

export function daysInMonth(month) {
  const [y, m] = month.split('-').map(Number);
  return new Date(y, m, 0).getDate();
}

export function monthLabel(month, locale) {
  const [y, m] = month.split('-').map(Number);
  const text = new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric' }).format(new Date(y, m - 1, 1));
  return text.charAt(0).toUpperCase() + text.slice(1);
}

// 1 234 → «1,2K», 2 500 000 → «2,5M». Точность до трёх значащих цифр.
export function fmtTokens(n, locale = 'ru') {
  const v = Number(n) || 0;
  const f = (x) => new Intl.NumberFormat(locale, { maximumFractionDigits: x >= 100 ? 0 : x >= 10 ? 1 : 2 }).format(x);
  if (v >= 1e9) return `${f(v / 1e9)}B`;
  if (v >= 1e6) return `${f(v / 1e6)}M`;
  if (v >= 1e3) return `${f(v / 1e3)}K`;
  return String(Math.round(v));
}

export function pct(part, whole) {
  return whole > 0 ? (part / whole) * 100 : 0;
}

export function fmtPct(value) {
  if (value > 0 && value < 1) return '<1%';
  return `${Math.round(value)}%`;
}

export function fmtSeconds(sec) {
  const s = Number(sec) || 0;
  const en = window?.LANG === 'en';
  if (s < 60) {
    const n = s < 10 ? s.toLocaleString(en ? 'en-US' : 'ru-RU', { minimumFractionDigits: 1, maximumFractionDigits: 1 }) : String(Math.round(s));
    return `${n} ${en ? 's' : 'с'}`;
  }
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.round(s % 60)).padStart(2, '0')}`;
}

// Куда придёт расход к концу месяца при нынешнем темпе, в процентах от лимита.
// Только для текущего месяца: у прошлого темп уже не нужен. Нужен хотя бы один день данных.
export function paceForecast({ tokens, limit, month, today = new Date() }) {
  if (!limit || limit <= 0) return null;
  if (month !== monthKey(today)) return null;
  const day = Math.max(1, today.getDate());
  const projected = (tokens / day) * daysInMonth(month);
  return (projected / limit) * 100;
}

// Цвета рядов: тянутся из токенов кита (одинаково читаются в обеих темах); сверх восьми — серый «Прочие».
export const SERIES = [
  'var(--k-brand, #025bff)',
  'var(--k-bot-tangerine, #ff8a3d)',
  'var(--k-bot-mint, #3ddc97)',
  'var(--k-ai, #6d5dfb)',
  'var(--k-bot-rose, #ff6fb5)',
  'var(--k-bot-sky, #4fb4ff)',
  'var(--k-bot-lemon, #ffd23f)',
  'var(--k-bot-coral, #ff6a4d)',
];
export const OTHER_COLOR = 'var(--k-text-3, #8d8f94)';

// id → цвет по порядку убывания расхода; одинаковый цвет у бота в графике и в легенде.
export function colorMap(ids) {
  const map = new Map();
  ids.forEach((id, i) => map.set(id, i < SERIES.length ? SERIES[i] : OTHER_COLOR));
  return map;
}

// Дни месяца для столбцов: все дни, у отсутствующих пусто.
export function dayRows(month, days) {
  const byDate = new Map((days || []).map((d) => [d.date, d.byBot || {}]));
  const total = daysInMonth(month);
  const rows = [];
  for (let i = 1; i <= total; i += 1) {
    const date = `${month}-${String(i).padStart(2, '0')}`;
    const byBot = byDate.get(date) || {};
    const sum = Object.values(byBot).reduce((a, b) => a + b, 0);
    rows.push({ date, day: i, byBot, sum });
  }
  return rows;
}

// Ответ API нужной формы? Иначе экран не рисуем частично.
export function validUsage(d) {
  return !!d && typeof d.totalTokens === 'number' && Array.isArray(d.bots) && Array.isArray(d.days) && Array.isArray(d.models);
}
