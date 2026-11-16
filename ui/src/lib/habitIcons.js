// Значки привычек: имена из src/habits.rs (ICON_NAMES) -> имя значка в lib/icons.js (штрих, без заливки).
// Эмодзи вместо имени сервер тоже принимает: его показываем как текст.
export const HABIT_ICON_NAMES = ['drop', 'sun', 'moon', 'book', 'pen', 'leaf', 'coffee', 'heart', 'brain', 'check', 'star', 'home', 'bed', 'apple', 'dumbbell', 'run', 'music', 'phone'];

// Одинаковые рисунки берутся из общих значков.
const SHARED = { moon: 'moon', check: 'check' };

/// Имя значка в реестре для имени привычки; пустая строка, если имя не из набора (эмодзи).
export function habitIconName(name) {
  if (!HABIT_ICON_NAMES.includes(name)) return '';
  return SHARED[name] ?? `habit-${name}`;
}
