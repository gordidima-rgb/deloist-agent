// Тонкая обёртка над window.tr из web/i18n.js: словарь EN один на всё приложение.
// Без i18n.js (например, в тесте) возвращает исходный русский текст.
export function tr(text, vars) {
  if (typeof window !== 'undefined' && typeof window.tr === 'function') return window.tr(text, vars);
  return vars ? text.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : text;
}

// Строки, которые пришли от сервера (ошибки, статусы): window.trServer знает их шаблоны.
export function trServer(text) {
  if (typeof window !== 'undefined' && typeof window.trServer === 'function') return window.trServer(text);
  return String(text ?? '');
}
