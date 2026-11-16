// Сколько длился ход: от первого сообщения человека (или служебной отметки времени) после прошлого
// готового ответа до этого ответа. Метка хода в треде не хранится, но у каждого сообщения есть `at`.
import { tr } from './i18n.js';

// Сообщение, под которым рисуется строка действий (то же условие, что в answerRow у web/app.js).
const isAnswer = (m) =>
  m?.role === 'assistant' && !m.error && Boolean(m.text) && !(Array.isArray(m.toolCalls) && m.toolCalls.length);

// Миллисекунды хода для каждого ответа: Map id -> ms. Нет данных — ответа в карте нет.
export function turnDurations(messages) {
  const out = new Map();
  let start = 0;
  for (const m of messages || []) {
    const at = Number(m?.at);
    if (!Number.isFinite(at) || at <= 0) continue;
    if (m.role === 'user' || m.role === 'time') {
      if (!start) start = at;
    } else if (isAnswer(m)) {
      if (start && m.id && at - start >= 1000) out.set(m.id, at - start);
      start = 0;
    }
  }
  return out;
}

// «45 с», «9 мин», «1 ч 5 мин». Секунды после минуты не показываем: это итог, а не секундомер.
export function durationText(ms) {
  const total = Math.round(Number(ms) / 1000);
  if (!Number.isFinite(total) || total < 1) return '';
  if (total < 60) return tr('{n} с', { n: total });
  const minutes = Math.round(total / 60);
  if (minutes < 60) return tr('{n} мин', { n: minutes });
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${tr('{n} ч', { n: h })} ${tr('{n} мин', { n: m })}` : tr('{n} ч', { n: h });
}
