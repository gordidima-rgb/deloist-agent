// Карточки предложений, которые сервер ещё держит (бот ждёт ответа), а на странице их нет: окно перезагрузили
// или открыли позже. Сервер отдаёт их в виде тех же событий, что и вживую (specialist_offer, team_proposal).
export function offerEventsToShow(botId, offers, isShown) {
  const out = [];
  const seen = new Set();
  for (const item of Array.isArray(offers) ? offers : []) {
    if (!item || typeof item !== 'object') continue;
    if (item.type !== 'specialist_offer' && item.type !== 'team_proposal') continue;
    const id = item.offer?.id || item.proposal?.id;
    if (!id || seen.has(id) || isShown(id)) continue;
    seen.add(id);
    out.push({ ...item, botId });
  }
  return out;
}
