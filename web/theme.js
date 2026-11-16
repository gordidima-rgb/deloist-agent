// Тема ставится до первой отрисовки: системная, пока человек не выбрал свою.
(() => {
  let saved = null;
  try {
    saved = localStorage.getItem('crew-theme');
  } catch {}
  const dark = saved ? saved === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  // Шрифт тоже до отрисовки, чтобы текст не прыгал при запуске.
  try {
    const font = localStorage.getItem('deloist-font');
    if (font) document.documentElement.dataset.font = font;
  } catch {}
  // Второй чат в разделённом окне: та же страница во фрейме, окно-родитель задаёт, что открыть.
  try {
    const q = new URLSearchParams(location.search);
    if (q.get('pane') === '1' && parent !== window) {
      document.documentElement.classList.add('pane-mode');
      window.DELOIST_PANE = { bot: q.get('bot') || '', room: q.get('room') || '' };
      // Тему и шрифт выбирает родитель: событие storage приходит во все окна этого адреса.
      addEventListener('storage', (e) => {
        if (e.key === 'crew-theme' && e.newValue) document.documentElement.dataset.theme = e.newValue;
        if (e.key === 'deloist-font') {
          if (e.newValue) document.documentElement.dataset.font = e.newValue;
          else delete document.documentElement.dataset.font;
        }
      });
    }
  } catch {}
})();

// Системная шапка окна красится в цвет рамки страницы (главное окно на Windows 11).
// Окна, которым это не нужно (док, подсказка), сообщение просто пропускают.
(() => {
  const rgb = (value) => {
    const n = (value.match(/[\d.]+/g) || []).map(Number);
    if (n.length < 3 || (n.length > 3 && n[3] === 0)) return null;
    return n.slice(0, 3).map((x) => Math.round(x)).join(',');
  };
  const send = () => {
    const ipc = window.ipc;
    // Шапку окна красит только само окно, не его вторая панель.
    if (!ipc || !document.body || window.DELOIST_PANE) return;
    const body = getComputedStyle(document.body);
    const bg = rgb(body.backgroundColor) || rgb(getComputedStyle(document.documentElement).backgroundColor);
    const fg = rgb(body.color);
    if (!bg || !fg) return;
    const dark = document.documentElement.dataset.theme === 'dark' ? 1 : 0;
    ipc.postMessage(`titlebar:${bg};${fg};${dark}`);
  };
  // После плавной смены цвета, а не посреди неё.
  let timer = 0;
  const later = () => { clearTimeout(timer); timer = setTimeout(send, 260); };
  addEventListener('DOMContentLoaded', later);
  addEventListener('load', later);
  new MutationObserver(later).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
})();
