// Сцена оркестратора: чистые правила без DOM. Что значит «фаза» разговора и какое настроение у шара.

// Фазы разговора: человек говорит/пишет, оркестратор думает, пишет, спрашивает, команда собрана.
export const PHASES = ['listening', 'thinking', 'typing', 'asking', 'done'];

// Вид шара для фазы — из готовых видов (avatar-states.js stageLook): состояние, значок занятия и короткая эмоция.
// Своих настроений сцена не заводит. Нет avatar-states — шар остаётся как есть.
export function lookForPhase(phase, states = typeof window !== 'undefined' ? window.CrewAvatarStates : null) {
  return states?.stageLook?.(phase) ?? null;
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
  let text = String(msg.text ?? '').trim();
  // Поздний ответ на вопрос (src/turn.rs late_answer_text): человеку показываем только его ответ, без служебной цитаты.
  if (msg.role === 'user' && text.startsWith('[ответ на вопрос бота]')) {
    const at = text.lastIndexOf('Ответ человека:');
    if (at >= 0) text = text.slice(at + 'Ответ человека:'.length).trim();
  }
  if (!text) return null;
  return { kind: msg.role === 'user' ? 'user' : 'bot', text, error: Boolean(msg.error) };
}
