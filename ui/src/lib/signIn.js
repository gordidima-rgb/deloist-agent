// Показывать ли в пустом чате бота карточку «Войди в Google в браузере бота».
// Поля бота приходят из /api/state плоско: googleSignIn и browserMode лежат в extra бота.
export function shouldShowSignIn(bot, bots = []) {
  if (!bot || bot.googleSignIn) return false;
  if (bot.browserMode === 'mine') return false;
  const first = bots.reduce((min, b) => (min && (min.createdAt ?? Infinity) <= (b.createdAt ?? Infinity) ? min : b), null);
  const isFirst = !first || first.id === bot.id;
  const hasSkill = (bot.skills || []).includes('skill.free-models');
  return isFirst || hasSkill;
}
