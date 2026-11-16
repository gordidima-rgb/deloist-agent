// Помощники поля ввода с подсветкой дат (Composer.svelte). Чистые функции: без DOM и без сети.
// Номера в spans — номера символов (кодовых точек) в тексте, как их отдаёт /api/dates/parse.

const KINDS = new Set(['date', 'time', 'repeat']);

// Текст, разрезанный на куски: { text, kind } — kind null у обычного текста, иначе 'date' | 'time' | 'repeat'.
export function splitByMarks(text, spans) {
  const chars = Array.from(String(text ?? ''));
  const marks = (Array.isArray(spans) ? spans : [])
    .filter((s) => s && KINDS.has(s.kind) && Number.isInteger(s.start) && Number.isInteger(s.end) && s.start < s.end)
    .map((s) => ({ start: Math.max(0, s.start), end: Math.min(chars.length, s.end), kind: s.kind }))
    .filter((s) => s.start < s.end)
    .sort((a, b) => a.start - b.start);
  const parts = [];
  let at = 0;
  for (const mark of marks) {
    if (mark.start < at) continue; // перекрытие: берём первый кусок
    if (mark.start > at) parts.push({ text: chars.slice(at, mark.start).join(''), kind: null });
    parts.push({ text: chars.slice(mark.start, mark.end).join(''), kind: mark.kind });
    at = mark.end;
  }
  if (at < chars.length) parts.push({ text: chars.slice(at).join(''), kind: null });
  return parts;
}

// Есть ли в разборе кусок такого вида.
export function hasKind(spans, kind) {
  return (Array.isArray(spans) ? spans : []).some((s) => s?.kind === kind);
}

// Текст без кусков одного вида: так «убрать чип» стирает слова из поля, и следующий разбор чипа уже не находит.
export function removeKind(text, spans, kind) {
  const parts = splitByMarks(text, spans);
  const kept = parts.filter((p) => p.kind !== kind).map((p) => p.text);
  return kept.join('').replace(/\s{2,}/g, ' ').trim();
}

// «пн, 5 марта» для даты ГГГГ-ММ-ДД. Часовой пояс не нужен: дата собирается из частей.
export function dateLabel(iso, locale) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(iso ?? ''));
  if (!m) return '';
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  try {
    return d.toLocaleDateString(locale || undefined, { weekday: 'short', day: 'numeric', month: 'long' });
  } catch {
    return String(iso);
  }
}

// Момент в мс из даты ГГГГ-ММ-ДД и времени ЧЧ:ММ по часам компьютера.
export function localMs(isoDate, hhmm) {
  const d = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(isoDate ?? ''));
  const t = /^(\d{1,2}):(\d{2})$/.exec(String(hhmm ?? ''));
  if (!d || !t) return 0;
  return new Date(Number(d[1]), Number(d[2]) - 1, Number(d[3]), Number(t[1]), Number(t[2])).getTime();
}

export function isoOf(date) {
  const p = (n) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())}`;
}
