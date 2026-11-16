// Отчёт по неделям: чистые помощники (даты, тон заголовка, множественное число) и демо-данные.
// Демо-данные показываются только если сервер ответил 404 (маршрут ещё не выпущен) и всегда с плашкой «Демо».

export function pad(n) {
  return String(n).padStart(2, '0');
}

export function isoDate(d) {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

// 'YYYY-MM-DD' -> локальная дата (полдень, чтобы переход на летнее время не сдвигал день).
export function parseIso(s) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(s || ''));
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12);
}

export function addDays(iso, days) {
  const d = parseIso(iso);
  if (!d) return iso;
  d.setDate(d.getDate() + days);
  return isoDate(d);
}

// Понедельник недели, в которую попадает дата.
export function mondayOf(date = new Date()) {
  const d = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 12);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return isoDate(d);
}

// 'ru' | 'en' -> «9–15 нояб.» / «Nov 9–15»
export function weekLabel(startIso, endIso, locale) {
  const a = parseIso(startIso);
  const b = parseIso(endIso) || (a && parseIso(addDays(startIso, 6)));
  if (!a || !b) return '';
  const loc = locale === 'en' ? 'en-US' : 'ru-RU';
  const day = new Intl.DateTimeFormat(loc, { day: 'numeric' });
  const full = new Intl.DateTimeFormat(loc, { day: 'numeric', month: 'short' });
  if (a.getMonth() === b.getMonth()) {
    return locale === 'en'
      ? `${new Intl.DateTimeFormat(loc, { month: 'short' }).format(a)} ${day.format(a)}–${day.format(b)}`
      : `${day.format(a)}–${full.format(b)}`;
  }
  return `${full.format(a)} – ${full.format(b)}`;
}

export function shortDay(startIso, locale) {
  const a = parseIso(startIso);
  if (!a) return '';
  return new Intl.DateTimeFormat(locale === 'en' ? 'en-US' : 'ru-RU', { day: 'numeric', month: 'short' }).format(a);
}

// Время сбоя: секунды или миллисекунды Unix, либо строка ISO.
export function formatTs(ts, locale) {
  let d;
  if (typeof ts === 'number') d = new Date(ts < 1e12 ? ts * 1000 : ts);
  else d = new Date(String(ts ?? ''));
  if (Number.isNaN(d.getTime())) return '';
  return new Intl.DateTimeFormat(locale === 'en' ? 'en-US' : 'ru-RU', {
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
  }).format(d);
}

// Форма слова для ru: 0 | 1 | 2 (один, несколько, много).
export function ruForm(n) {
  const a = Math.abs(Math.trunc(n)) % 100;
  const b = a % 10;
  if (a > 10 && a < 20) return 2;
  if (b === 1) return 0;
  if (b >= 2 && b <= 4) return 1;
  return 2;
}

// Тон недели по доле удачных ответов: 'great' | 'good' | 'rough'. Пустая неделя — 'empty'.
export function toneOf(replies, okPercent) {
  if (!replies) return 'empty';
  if (okPercent >= 90) return 'great';
  if (okPercent >= 75) return 'good';
  return 'rough';
}

export function formatNum(n, locale) {
  return new Intl.NumberFormat(locale === 'en' ? 'en-US' : 'ru-RU').format(Math.round(Number(n) || 0));
}

export function formatSeconds(s, locale) {
  const v = Number(s) || 0;
  const loc = locale === 'en' ? 'en-US' : 'ru-RU';
  return new Intl.NumberFormat(loc, { maximumFractionDigits: v < 10 ? 1 : 0 }).format(v);
}

// Демо-данные для 404 и тестов: форма та же, что у GET /api/report.
export function demoReport(weekStart) {
  const start = weekStart || mondayOf();
  const ago = Math.round((parseIso(mondayOf()) - parseIso(start)) / (7 * 86400000));
  const series = [
    [9, 1],
    [11, 2],
    [8, 0],
    [14, 3],
    [2, 1],
    [0, 0],
  ];
  const weeks = [];
  for (let i = 5; i >= 0; i -= 1) {
    const idx = Math.min(5, Math.max(0, 5 - (ago + i)));
    const off = ago + i > 5 ? [0, 0] : series[idx];
    weeks.push({ start: addDays(start, -7 * i), ok: off[0], failed: off[1] });
  }
  const last = weeks[5];
  const replies = last.ok + last.failed;
  const okPercent = replies ? Math.round((last.ok / replies) * 100) : 0;
  const t = Date.parse(start + 'T12:00:00');
  return {
    weekStart: start,
    weekEnd: addDays(start, 6),
    replies,
    okCount: last.ok,
    failedCount: last.failed,
    okPercent,
    weeks,
    bots: replies
      ? [
          { id: 'anya', name: 'Аня', replies: Math.ceil(replies / 2), okPercent: 100, avgTokens: 1840, avgSeconds: 6.2, failures: [] },
          {
            id: 'boris',
            name: 'Борис',
            replies: Math.floor(replies / 2),
            okPercent: Math.max(0, okPercent - 20),
            avgTokens: 5230,
            avgSeconds: 18.4,
            failures: last.failed
              ? [
                  { ts: t + 3600e3 * 30, preview: 'Не удалось получить ответ от модели: таймаут' },
                  { ts: t + 3600e3 * 55, preview: 'Лимит запросов исчерпан, повторите позже' },
                ]
              : [],
          },
        ]
      : [],
  };
}
