// Сообщение боту из просмотрщика файла: выделенный кусок и комментарий к нему.
// Текст уходит в поле ввода чата как обычное сообщение; бот видит путь, строки и сам кусок.
const QUOTE_LIMIT = 4000;

/** Забор «```» длиннее любой серии обратных кавычек внутри цитаты, чтобы цитата не ломала разметку. */
function fenceFor(text) {
  const longest = Math.max(0, ...(text.match(/`+/g) || []).map((run) => run.length));
  return '`'.repeat(Math.max(3, longest + 1));
}

/**
 * `{ path, from, to, quote, comment, language }` → блок для чата.
 * `from`/`to` — номера строк (с 1) или `null`, если выделено в отрисованном тексте без строк.
 */
export function formatQuote({ path, from = null, to = null, quote, comment = '', language = '' }) {
  const cut = quote.length > QUOTE_LIMIT ? `${quote.slice(0, QUOTE_LIMIT)}\n…` : quote;
  const where = from == null ? '' : to == null || to === from ? `, строка ${from}` : `, строки ${from}–${to}`;
  const fence = fenceFor(cut);
  const head = `Файл \`${path}\`${where}:`;
  const body = `${fence}${language}\n${cut}\n${fence}`;
  const note = comment.trim();
  return note ? `${head}\n${body}\nКомментарий: ${note}` : `${head}\n${body}`;
}

/** Добавляет блок к тексту в поле ввода через пустую строку, не затирая черновик. */
export function appendBlock(current, block) {
  const base = String(current ?? '').replace(/\s+$/, '');
  return base ? `${base}\n\n${block}\n` : `${block}\n`;
}
