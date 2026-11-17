// Сцена оркестратора: чистые правила без DOM. Что значит «фаза» разговора и какое настроение у шара.

// Фазы разговора: человек говорит/пишет, оркестратор думает, пишет, спрашивает, команда собрана.
export const PHASES = ['listening', 'thinking', 'typing', 'asking', 'done'];

// Запас, пока в программе нет настроений listen/think/talk/curious/proud (ветка эмоций ещё не влита).
// Для фаз без запасного настроения шар остаётся как есть.
const FALLBACK = { asking: 'star', done: 'joy' };

// Настроение шара для фазы: сначала то, что знает avatar-states.js, потом запас.
export function moodForPhase(phase, states = typeof window !== 'undefined' ? window.CrewAvatarStates : null) {
  const own = states?.stageMood?.(phase);
  if (own) return own;
  return FALLBACK[phase] || null;
}

// Фаза по состоянию сцены. Приоритет: команда собрана, человек говорит, вопрос ждёт ответа, идёт текст, ход идёт, иначе слушаем.
export function phaseOf({ stage, hearing = false, openCards = 0, streaming = false, busy = false }) {
  if (stage === 'ready') return 'done';
  if (hearing) return 'listening';
  if (openCards > 0) return 'asking';
  if (streaming) return 'typing';
  if (busy) return 'thinking';
  return 'listening';
}

// Сообщение из истории разговора в пузырь; служебные и пустые отбрасываем.
export function bubbleOf(msg) {
  if (!msg || (msg.role !== 'user' && msg.role !== 'assistant')) return null;
  const text = String(msg.text ?? '').trim();
  if (!text) return null;
  return { kind: msg.role === 'user' ? 'user' : 'bot', text, error: Boolean(msg.error) };
}
