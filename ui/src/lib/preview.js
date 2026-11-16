// Тексты для строк бокового списка: превью без разметки и подпись команды.

// Превью приходит обрезанным с сервера, поэтому `**` может остаться без пары: убираем знаки, а не разбираем пары.
export function plainPreview(text) {
  return String(text ?? '')
    .replace(/```[^\n]*\n?/g, ' ')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/`+/g, '')
    .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])\s+/gm, '')
    .replace(/(\*\*|__|~~)/g, '')
    .replace(/\*(?=\S)|(?<=\S)\*/g, '')
    .replace(/(^|\s)_(?=\S)|(?<=\S)_(?=\s|$|[.,;:!?])/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

// Подпись команды: последнее сообщение; пока в команде не писали — кто в ней состоит; «пока пусто» только без ботов.
export function teamSubtitle(preview, memberNames, empty) {
  const text = plainPreview(preview);
  if (text) return text;
  const names = (memberNames || []).filter(Boolean);
  return names.length ? names.join(', ') : empty;
}
