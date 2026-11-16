// Время и числа для экранов ящика бота: так же, как в web/app.js (whenText, numberRu).
// Сегодня — только часы, раньше — день, месяц и часы.
const locale = () => (typeof window !== 'undefined' && window.LOCALE) || undefined;

export function whenText(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return '';
  const d = new Date(n);
  const time = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === new Date().toDateString()) return time;
  return `${d.toLocaleDateString(locale(), { day: 'numeric', month: 'short' })} ${time}`;
}

export const numberRu = (n) => Number(n || 0).toLocaleString(locale());
