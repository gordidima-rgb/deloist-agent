// Пример ответа GET /api/usage — только для разработки и тестов.
// Экран берёт его, когда сервер отвечает 404 (ручки ещё нет), и тогда честно пишет об этом.
import { daysInMonth } from './usage.js';

export function usageFixture(month) {
  const n = daysInMonth(month);
  const days = [];
  for (let i = 1; i <= Math.min(n, 12); i += 1) {
    days.push({
      date: `${month}-${String(i).padStart(2, '0')}`,
      byBot: { analyst: 40000 + i * 3500, writer: i % 3 ? 22000 : 90000, helper: i % 2 ? 6000 : 0 },
    });
  }
  const sum = (id) => days.reduce((a, d) => a + (d.byBot[id] || 0), 0);
  const bots = [
    { id: 'analyst', name: 'Аналитик', engine: 'claude', model: 'claude-sonnet-5-5', replies: 41, tokens: sum('analyst'), monthlyLimit: 800000 },
    { id: 'writer', name: 'Редактор', engine: 'codex', model: 'gpt-6.1-terra', replies: 28, tokens: sum('writer'), monthlyLimit: null },
    { id: 'helper', name: 'Помощник', engine: 'deloist', model: 'deloist-fast', replies: 17, tokens: sum('helper'), monthlyLimit: 60000 },
  ];
  const botsTotal = bots.reduce((a, b) => a + b.tokens, 0);
  const behind = { router: 18000, memory: 31000, helper: 9000 };
  return {
    month,
    totalTokens: botsTotal + behind.router + behind.memory + behind.helper,
    replies: 86,
    avgSeconds: 14.6,
    days,
    bots,
    models: [
      { id: 'claude-sonnet-5-5', provider: 'claude', tokens: sum('analyst') },
      { id: 'gpt-6.1-terra', provider: 'codex', tokens: sum('writer') },
      { id: 'deloist-fast', provider: 'deloist', tokens: sum('helper') },
    ],
    behind,
    receipts: [
      { id: 'r1', ts: `${month}-12T09:41:00Z`, botId: 'analyst', botName: 'Аналитик', where: 'chat', model: 'claude-sonnet-5-5', seconds: 12.4, tokens: 8400, outcome: 'ok', preview: 'Сводка по продажам за неделю готова', threadId: 'analyst' },
      { id: 'r2', ts: `${month}-11T18:05:00Z`, botId: 'writer', botName: 'Редактор', where: 'telegram', model: 'gpt-6.1-terra', seconds: 31, tokens: 22100, outcome: 'ok', preview: 'Переписал текст письма клиенту', threadId: 'writer' },
    ],
  };
}
