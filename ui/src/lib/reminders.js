// Из разобранной фразы (Composer, /api/dates/parse) собирает поля напоминания для /api/routines.
// Ошибка — код ('when' | 'time' | 'past' | 'repeat'); текст по коду подбирает вкладка через tr().
import { localMs, isoOf } from './composer.js';

const NEED_WHEN = 'when';
const NEED_TIME = 'time';
const PAST = 'past';
const REPEAT_UNSUPPORTED = 'repeat';

function isoWeekday(iso) {
  const [y, m, d] = iso.split('-').map(Number);
  const day = new Date(y, m - 1, d).getDay();
  return day === 0 ? 7 : day;
}

// -> { error: код } или { fields: { prompt, at?, days?, onceAt } }
export function planReminder(parsed, now = new Date()) {
  const text = String(parsed?.title || parsed?.text || '').trim();
  if (!text) return { error: NEED_WHEN };
  const rule = parsed.recurrence;
  if (rule) {
    if (!['daily', 'weekly'].includes(rule.freq) || (rule.interval ?? 1) !== 1) return { error: REPEAT_UNSUPPORTED };
    if (!parsed.due_time) return { error: NEED_TIME };
    let days = [];
    if (rule.freq === 'weekly') {
      days = [...(rule.weekdays ?? [])];
      if (!days.length && parsed.due_date) days = [isoWeekday(parsed.due_date)];
    }
    return { fields: { prompt: text, at: parsed.due_time, days, onceAt: null } };
  }
  if (!parsed.due_time && !parsed.due_date) return { error: NEED_WHEN };
  if (!parsed.due_time) return { error: NEED_TIME };
  let iso = parsed.due_date || isoOf(now);
  let ms = localMs(iso, parsed.due_time);
  if (!parsed.due_date && ms <= now.getTime()) {
    const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
    iso = isoOf(next);
    ms = localMs(iso, parsed.due_time);
  }
  if (ms <= now.getTime()) return { error: PAST };
  return { fields: { prompt: text, onceAt: ms } };
}
