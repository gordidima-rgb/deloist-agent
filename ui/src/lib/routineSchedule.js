// Расписание рутины: форма ↔ поля сервера и человеческая подпись. Чистые функции; правило повтора — то же, что у задач (todos.js).
import { recurrenceSummary, todayIso } from './todos.js';

export const KINDS = ['at', 'every', 'once'];

const isTime = (v) => typeof v === 'string' && /^\d{2}:\d{2}$/.test(v);

// Миллисекунды местного момента «ГГГГ-ММ-ДД» + «ЧЧ:ММ»; 0, если чего-то не хватает.
export function onceMoment(date, time) {
  if (!date || !isTime(time)) return 0;
  const ms = new Date(`${date}T${time}:00`).getTime();
  return Number.isFinite(ms) ? ms : 0;
}

// Что не заполнено: '' — расписание готово. Тексты строит экран через tr().
export function scheduleProblem(s, now = Date.now()) {
  if (s.kind === 'at' && !isTime(s.at)) return 'time';
  if (s.kind === 'once') {
    const ms = onceMoment(s.date, s.at);
    if (!ms) return 'time';
    if (ms <= now) return 'past';
  }
  return '';
}

// Поля расписания для POST/PATCH /api/workflows и /api/reminders: взаимоисключающие виды обнуляют чужие поля.
export function scheduleFields(s) {
  if (s.kind === 'at') return { at: s.at, recurrence: s.recurrence, startOn: s.startOn || '', onceAt: 0, days: [] };
  if (s.kind === 'once') return { at: '', recurrence: null, startOn: '', onceAt: onceMoment(s.date, s.at), days: [] };
  return { at: '', recurrence: null, startOn: '', onceAt: 0 };
}

// Рутина с сервера → состояние редактора (для показа правила и будущей правки).
export function scheduleFromRoutine(r) {
  if (r.onceAt > 0) {
    const d = new Date(r.onceAt);
    const pad = (n) => String(n).padStart(2, '0');
    return { kind: 'once', date: todayIso(d), at: `${pad(d.getHours())}:${pad(d.getMinutes())}`, recurrence: null, startOn: '' };
  }
  if (isTime(r.at)) return { kind: 'at', at: r.at, recurrence: r.recurrence ?? null, startOn: r.startOn || '', date: '' };
  return { kind: 'every', at: '', recurrence: null, startOn: '', date: '' };
}

// Подпись расписания по времени и разовых запусков: «в 09:00, каждую 3-ю субботу сентября, с 1 декабря».
// Интервальные рутины подписывает экран (окно, дни недели). t — tr(); fmtDate — как показывать дату.
export function timedScheduleText(r, t, locale, fmtDate = (iso) => iso) {
  if (r.onceAt > 0) {
    const s = scheduleFromRoutine(r);
    return t('один раз, {date} в {time}', { date: fmtDate(s.date), time: s.at });
  }
  if (!isTime(r.at)) return '';
  const parts = [t('в {time}', { time: r.at }), recurrenceSummary(r.recurrence, t, locale) || t('каждый день')];
  if (r.startOn) parts.push(t('с {date}', { date: fmtDate(r.startOn) }));
  return parts.join(', ');
}
