// Уведомления раздела «Рабочие процессы»: отдельная сборка не заводит свой хост, а зовёт общий из kit.js.
// Без него (в тестах) молча ничего не показывает.
export function toast(message, options) {
  try {
    if (typeof window !== 'undefined' && window.DeloistUI?.toast) return window.DeloistUI.toast(message, options);
  } catch {}
  return 0;
}
