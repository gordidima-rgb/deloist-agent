// Только для разработчика (см. ui-demo.html): CSP приложения запрещает встроенные скрипты.
// Витрина — Svelte-компонент KitGallery: один и тот же набор в двух темах.
(() => {
  const { mountComponent, KitGallery } = window.DeloistUI;
  const tr = (text) => window.tr(text);
  const host = document.getElementById('gallery');
  if (host && KitGallery) mountComponent(KitGallery, host, {});

  // Переключатель темы двигает корень документа: так проверяются и глобальные островки
  // (уведомления), которые живут вне колонок витрины.
  const root = document.documentElement;
  const label = document.getElementById('theme-label');
  const light = document.getElementById('theme-light');
  const dark = document.getElementById('theme-dark');

  if (label) label.textContent = tr('Тема');
  if (light) light.textContent = tr('Светлая тема');
  if (dark) dark.textContent = tr('Тёмная тема');

  function mark() {
    const isDark = root.dataset.theme === 'dark';
    light?.setAttribute('aria-pressed', String(!isDark));
    dark?.setAttribute('aria-pressed', String(isDark));
  }

  function setTheme(theme) {
    root.dataset.theme = theme;
    try {
      localStorage.setItem('crew-theme', theme);
    } catch {}
    mark();
  }

  light?.addEventListener('click', () => setTheme('light'));
  dark?.addEventListener('click', () => setTheme('dark'));
  mark();
})();

// Прежние компоненты дока: кольца лимитов, кнопка голоса, плашка шагов.
const colors = ['#22c55e', '#22c55e', '#eab308', '#eab308', '#f97316', '#ef4444', '#6366f1', '#06b6d4'];
const values = [0.42, 0.1, 0.55, 0.7, 0.83, 0.97, 0.25, 1];
const names = ['Аня', 'Борис', 'Вера', 'Глеб', 'Дина', 'Егор', 'Жанна', 'Зоя'];
const host = document.getElementById('rings');
const rings = values.map((value, i) => {
  const cell = document.createElement('div');
  host.appendChild(cell);
  return window.DeloistUI.mountLimitRing(cell, { left: value, size: i % 2 ? 48 : 52, label: names[i] });
});
document.getElementById('slider').addEventListener('input', (e) => rings[0].update({ value: e.target.value / 100 }));

// Кнопка голоса: все состояния; уровень и прогресс заданы, движения нет без событий.
const micStates = ['idle', 'downloading', 'loading', 'recording', 'transcribing', 'sending', 'done', 'error'];
const micHost = document.getElementById('mics');
micStates.forEach((state) => {
  const cell = document.createElement('div');
  micHost.appendChild(cell);
  window.DeloistUI.mountMicButton(cell, { state, level: state === 'recording' ? 0.7 : 0, progress: state === 'downloading' ? 0.42 : 0 });
});

// Плашка шагов: прошлый шаг сверху, текущий с девятью пикселями.
const pillHost = document.createElement('div');
document.getElementById('pills').appendChild(pillHost);
window.DeloistUI.mountStepPill(pillHost, { now: { key: 2, kind: 'read', text: window.tr('Читает {file}', { file: 'notes.txt' }), meta: '' }, was: { key: 1, text: window.tr('Думает'), meta: '' } });
