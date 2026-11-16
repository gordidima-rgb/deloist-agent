// Раскладка «Экрана бота» (web/screen.js): ширина панели рядом с чатом, углы островка чата,
// переходы между режимами. Чистые функции без DOM, чтобы их можно было проверить тестом.

export const MIN_PANEL = 360; // экран бота не уже
export const MIN_CHAT = 420; // чат рядом с ним не уже
export const GAP = 12;
export const DEFAULT_PANEL = 560;
export const KEY = 'deloist.screen.layout';

// Ширина панели в режиме «рядом»: container — ширина области справа от списка ботов.
// Если места не хватает на оба минимума, чат уступает: панель остаётся читаемой.
export function clampPanel(width, container) {
  const max = Math.max(MIN_PANEL, container - MIN_CHAT - GAP * 3);
  const w = typeof width === 'number' && !Number.isNaN(width) ? width : DEFAULT_PANEL;
  return Math.round(Math.min(Math.max(w, MIN_PANEL), max));
}

// Ширина панели по положению указателя: расстояние от правого края окна минус поля.
export function widthFromPointer(clientX, viewport, container) {
  return clampPanel(viewport - clientX - GAP, container);
}

// Шаг с клавиатуры: стрелка влево расширяет панель (разделитель уходит влево).
export function widthByKey(width, key, shift, container) {
  const step = shift ? 96 : 24;
  if (key === 'ArrowLeft') return clampPanel(width + step, container);
  if (key === 'ArrowRight') return clampPanel(width - step, container);
  if (key === 'Home') return clampPanel(MIN_PANEL, container);
  if (key === 'End') return clampPanel(Infinity, container);
  return width;
}

export const CORNERS = ['tl', 'tr', 'bl', 'br'];

// Ближайший угол к центру островка; bounds — область, где островок может стоять.
export function nearestCorner(rect, bounds) {
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  const right = cx > bounds.left + bounds.width / 2;
  const bottom = cy > bounds.top + bounds.height / 2;
  return (bottom ? 'b' : 't') + (right ? 'r' : 'l');
}

export function validCorner(value, fallback = 'br') {
  return CORNERS.includes(value) ? value : fallback;
}

// Режимы панели: split (рядом с чатом), max (на всю область, чат — островок), min (таблетка у края).
// Возврат из min — в то, что было до сворачивания.
export function nextMode(state, action) {
  const { mode, before } = state;
  switch (action) {
    case 'maximize':
      return mode === 'max' ? { mode: 'split', before: 'split' } : { mode: 'max', before: 'max' };
    case 'back': // Esc
      return mode === 'max' ? { mode: 'split', before: 'split' } : state;
    case 'minimize':
      return mode === 'min' ? state : { mode: 'min', before: mode };
    case 'restore':
      return mode === 'min' ? { mode: before === 'max' ? 'max' : 'split', before: before === 'max' ? 'max' : 'split' } : state;
    default:
      return state;
  }
}

export function badge(count) {
  if (!count || count < 1) return '';
  return count > 9 ? '9+' : String(count);
}

// Сохранённая раскладка живёт в localStorage окна; хранилище может не открыться.
export function loadLayout(storage) {
  try {
    const data = JSON.parse(storage?.getItem(KEY) || '{}');
    return {
      width: Number.isFinite(data.width) ? data.width : DEFAULT_PANEL,
      corner: validCorner(data.corner),
      mode: data.mode === 'max' ? 'max' : 'split',
    };
  } catch {
    return { width: DEFAULT_PANEL, corner: 'br', mode: 'split' };
  }
}

export function saveLayout(storage, layout) {
  try {
    storage?.setItem(KEY, JSON.stringify({ width: layout.width, corner: layout.corner, mode: layout.mode }));
  } catch {
    // без хранилища раскладка живёт до закрытия окна
  }
}
