// Помощник «Что нового»: безопасное превращение markdown-пунктов в текст.
// HTML из markdown экранируется; разрешён только **жирный**.
export function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function markdownToText(markdown) {
  const text = String(markdown ?? '');
  // экранируем сначала, чтобы никакой HTML из markdown не прошёл
  const escaped = escapeHtml(text);
  // **жирный** -> <strong>жирный</strong>
  return escaped.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
}

