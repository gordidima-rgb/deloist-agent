// Помощник «Что нового»: разбор CHANGELOG.md и безопасный вывод пунктов.
// HTML из markdown экранируется; разрешены только **жирный**, `код` и ссылки http(s).

export function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function markdownToText(markdown) {
  // Экранируем сначала, чтобы никакой HTML из markdown не прошёл.
  let html = escapeHtml(markdown);
  // `код` прячем за метки: внутри него жирный и ссылки не работают.
  const codes = [];
  html = html.replace(/`([^`\n]+)`/g, (_, code) => `\u0000${codes.push(`<code>${code}</code>`) - 1}\u0000`);
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  // Адрес уже экранирован (кавычки стали &quot;), из атрибута он не выйдет.
  html = html.replace(/\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  return html.replace(/\u0000(\d+)\u0000/g, (_, i) => codes[Number(i)] ?? '');
}

/** Картинка раздела: только `docs/images/whats-new/<файл>`, без подкаталогов и «..». */
export function imageUrl(path) {
  const m = String(path ?? '').trim().match(/^(?:\.\/)?docs\/images\/whats-new\/([A-Za-z0-9][A-Za-z0-9._-]*)$/);
  if (!m || m[1].includes('..')) return '';
  return `/api/whats-new/image/${encodeURIComponent(m[1])}`;
}

const GROUPS = [
  { key: 'new', names: ['добавлено', 'added', 'new'] },
  { key: 'improved', names: ['изменено', 'changed', 'improved'] },
  { key: 'fixed', names: ['исправлено', 'fixed'] },
];
const ORDER = { new: 0, improved: 1, fixed: 2 };

function groupKey(title) {
  const t = String(title ?? '').trim().toLowerCase();
  const known = GROUPS.find((g) => g.names.includes(t));
  return known ? known.key : `other:${title.trim()}`;
}

/** «**Заголовок.** текст» → { lead: 'Заголовок.', text: 'текст' }. */
export function splitLead(item) {
  const m = String(item ?? '').match(/^\*\*(.+?)\*\*\s*([\s\S]*)$/);
  return m ? { lead: m[1].trim(), text: m[2].trim() } : { lead: '', text: String(item ?? '').trim() };
}

/** Раздел одной версии → { image, groups: [{ key, title, items: [{ lead, text }] }] }. */
export function parseSection(markdown) {
  const lines = String(markdown ?? '').split(/\r?\n/);
  const groups = [];
  let current = null;
  let item = null;
  let image = '';
  const group = (title) => {
    const key = groupKey(title);
    let g = groups.find((x) => x.key === key);
    if (!g) groups.push((g = { key, title: title.trim(), items: [] }));
    return g;
  };
  const flush = () => {
    if (item && item.trim()) (current ?? group('Добавлено')).items.push(splitLead(item.trim()));
    item = null;
  };
  for (const line of lines) {
    const img = line.match(/^\s*!\[[^\]]*\]\(([^)\s]+)\)\s*$/);
    if (img) {
      flush();
      image ||= imageUrl(img[1]);
      continue;
    }
    const head = line.match(/^###\s+(.+?)\s*$/);
    if (head) {
      flush();
      current = group(head[1]);
      continue;
    }
    const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
    if (bullet) {
      flush();
      item = bullet[1];
      continue;
    }
    if (item !== null && line.trim() && /^\s/.test(line)) {
      item += ` ${line.trim()}`;
      continue;
    }
    if (!line.trim()) flush();
  }
  flush();
  groups.sort((a, b) => (ORDER[a.key] ?? 9) - (ORDER[b.key] ?? 9));
  return { image, groups };
}

/** Страницы окна: одна на категорию, пункты всех версий подряд (новые версии выше). */
export function buildPages(sections) {
  const pages = [];
  let image = '';
  for (const section of sections || []) {
    const parsed = parseSection(section?.markdown);
    image ||= parsed.image;
    for (const g of parsed.groups) {
      if (!g.items.length) continue;
      let page = pages.find((p) => p.key === g.key);
      if (!page) pages.push((page = { key: g.key, title: g.title, items: [] }));
      for (const it of g.items) page.items.push({ ...it, version: section.version });
    }
  }
  pages.sort((a, b) => (ORDER[a.key] ?? 9) - (ORDER[b.key] ?? 9));
  return { pages, image };
}

/** Версии для полного списка: заголовок и категории каждой. */
export function buildFullList(sections) {
  return (sections || []).map((s) => ({ version: s.version, date: s.date || '', groups: parseSection(s.markdown).groups.filter((g) => g.items.length) }));
}

/**
 * Заметки из подписанного описания версии. Это либо раздел CHANGELOG целиком (с заголовком `## [x] — дата` или без),
 * либо старая строка в одну фразу. Возвращает список разделов того же вида, что отдаёт /api/whats-new.
 */
export function parseNotes(notes, version = '') {
  const text = String(notes ?? '').trim();
  if (!text) return [];
  if (!/^#{2,3}\s/m.test(text)) return [{ version, date: '', markdown: `### Добавлено\n\n- ${text.replace(/\s*\n\s*/g, ' ')}` }];
  const out = [];
  let head = null;
  let body = [];
  const push = () => {
    if (head || body.join('').trim()) out.push({ version: head?.version ?? version, date: head?.date ?? '', markdown: body.join('\n').trim() });
  };
  for (const line of text.split(/\r?\n/)) {
    const m = line.match(/^##\s+\[([^\]]+)\]\s*(?:[—–-]\s*)?(.*)$/);
    if (m) {
      push();
      head = { version: m[1].trim(), date: m[2].trim() };
      body = [];
    } else {
      body.push(line);
    }
  }
  push();
  return out;
}

/** Склейка без повторов по версии: сначала `first`, потом остальные. */
export function mergeSections(first, rest) {
  const seen = new Set();
  return [...(first || []), ...(rest || [])].filter((s) => {
    const key = s?.version;
    if (!key || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
