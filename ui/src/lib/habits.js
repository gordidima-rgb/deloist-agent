// Маска дней привычки: бит 0 — понедельник … бит 6 — воскресенье, 127 — каждый день (как в src/habits.rs).
export function daysFromMask(mask) {
  const m = Number(mask) || 0;
  return Array.from({ length: 7 }, (_, i) => Boolean(m & (1 << i)));
}

export function maskFromDays(days) {
  return (days ?? []).reduce((mask, on, i) => (on ? mask | (1 << i) : mask), 0);
}
