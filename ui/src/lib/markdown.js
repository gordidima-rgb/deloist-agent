// Разметка ответов бота: абзацы, заголовки, списки, таблицы GFM, код, жирный, ссылки.
// Чистая функция «текст -> безопасный HTML»: весь ввод экранируется до разбора, поэтому
// теги из ответа модели не доходят до страницы. Старый чат (web/app.js) зовёт её через
// window.DeloistUI.markdown(); пока ответ печатается, она вызывается на каждом куске.

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export const escapeHtml = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ESC[c]);
// Значок перед ссылкой по её адресу. Новый сервис — новая строка: hosts (без www.), schemes, svg.
// Значок перед ссылкой по её адресу. Новый сервис — новая строка: hosts (без www., поддомены тоже),
// schemes и glyph (содержимое svg 24x24 в currentColor: линия 1.8 без заливки, закрашенное — fill="currentColor").
// Рисуется в 14 px (класс .md-link-ico в web/style.css).
const icon = (glyph) =>
  `<svg class="md-link-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${glyph}</svg>`;
const FILL = 'fill="currentColor" stroke="none"';
export const LINK_ICONS = [
  {
    name: 'telegram',
    hosts: ['t.me', 'telegram.me', 'web.telegram.org'],
    schemes: ['tg:'],
    svg: icon(`<path ${FILL} d="M21.4 3.3 2.9 10.4c-.9.4-.9 1.6 0 1.9l4.6 1.5 1.8 5.6c.2.7 1.1.9 1.6.4l2.6-2.4 4.7 3.5c.6.4 1.4.1 1.6-.6l3.2-15.4c.2-.9-.7-1.6-1.6-1.2zM9.8 14.1l8-5-6.2 6.2-.3 3.2-1.5-4.4z"/>`),
  },
  // Same glyph as web/icons/apps/x.svg, inset inside the badge.
  { name: 'x', hosts: ['x.com', 'twitter.com'], svg: `<svg class="md-link-ico md-link-ico-x" viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path fill="currentColor" transform="translate(4 4) scale(.667)" d="M14.234 10.162 22.977 0h-2.072l-7.591 8.824L7.251 0H.258l9.168 13.343L.258 24H2.33l8.016-9.318L16.749 24h6.993zm-2.837 3.299-.929-1.329L3.076 1.56h3.182l5.965 8.532.929 1.329 7.754 11.09h-3.182z"/></svg>` },
  {
    name: 'discord',
    hosts: ['discord.com', 'discord.gg'],
    svg: icon(`<path d="M5.5 6.5c2-1 4-1.300 4-1.300l.5 1c1.300-.2 2.700-.2 4 0l.5-1s2 .3 4 1.300c1.800 2.700 2.400 5.600 2.200 8.800-1.400 1.100-3 1.700-4.200 2l-1-1.600M5.500 6.500C3.700 9.200 3.100 12.100 3.300 15.300c1.400 1.100 3 1.700 4.200 2l1-1.600"/><circle ${FILL} cx="9.200" cy="12" r="1.300"/><circle ${FILL} cx="14.800" cy="12" r="1.300"/>`),
  },
  { name: 'youtube', hosts: ['youtube.com', 'youtu.be'], svg: icon(`<rect x="2.500" y="5.500" width="19" height="13" rx="4"/><path ${FILL} d="M10 9.300v5.400l4.600-2.700z"/>`) },
  {
    name: 'reddit',
    hosts: ['reddit.com', 'redd.it'],
    svg: icon(`<ellipse cx="12" cy="14" rx="8" ry="5.500"/><circle ${FILL} cx="9" cy="13.500" r="1.200"/><circle ${FILL} cx="15" cy="13.500" r="1.200"/><path d="M9.500 16.500c1.500 1 3.500 1 5 0M12 8.500l1-4.500 4 1"/><circle ${FILL} cx="18" cy="5.500" r="1.300"/>`),
  },
  { name: 'vk', hosts: ['vk.com', 'vk.ru'], svg: icon(`<path ${FILL} d="M3 7h3c.5 4 2.500 7 4.500 7.500V7H13v4.500c1.500-.2 3-2 3.800-4.500H20c-.6 2.500-2 4.300-3.200 5 1.400.7 3 2.300 3.700 5h-3.300c-.7-1.800-2-3-3.700-3.300V17h-1C6.500 17 3.500 12.500 3 7z"/>`) },
  {
    name: 'github',
    hosts: ['github.com'],
    svg: icon('<path d="M9 19c-4 1.200-4-2-6-2.500M15 21v-3.500c0-1 .1-1.400-.5-2 2.800-.3 5.500-1.400 5.500-6 0-1.200-.5-2.300-1.300-3.200.3-1 .3-2.200-.1-3.200 0 0-1-.3-3.300 1.300a11.500 11.500 0 0 0-6 0C6.500 2.300 5.500 2.600 5.500 2.600c-.4 1-.4 2.200-.1 3.200A4.600 4.600 0 0 0 4 9c0 4.600 2.700 5.700 5.500 6-.6.600-.6 1.200-.5 2V21"/>'),
  },
  { name: 'habr', hosts: ['habr.com'], svg: icon('<rect x="3.500" y="3.500" width="17" height="17" rx="4"/><path d="M8.500 8v8M15.500 8v8M8.500 12h7"/>') },
];

const hostMatches = (host, known) => host === known || host.endsWith(`.${known}`);

export function linkIcon(href) {
  const m = /^([a-z][a-z0-9+.-]*:)(?:\/\/)?([^/?#:@\s]*)/i.exec(String(href || ''));
  if (!m) return '';
  const scheme = m[1].toLowerCase();
  const host = m[2].toLowerCase().replace(/^www\./, '');
  const hit = LINK_ICONS.find((i) => i.schemes?.includes(scheme) || i.hosts?.some((h) => hostMatches(host, h)));
  return hit ? hit.svg : '';
}

// Ссылка на файл в папке бота: `C:/Users/…/workspaces/<бот>/<путь>`, `C:\…` или `file:///…`. Возвращает
// { owner, rel } либо null: остальные папки диска страница не открывает. Файл открывает app.js через ящик
// «Файлы» (так же, как человек), а не навигацией по file:.
export function workspaceRef(target) {
  let raw = String(target ?? '').trim();
  if (!/^(?:file:\/\/\/)?[a-z]:[\\/]/i.test(raw)) return null;
  raw = raw.replace(/^file:\/\/\//i, '');
  try {
    raw = decodeURIComponent(raw);
  } catch {
    // Не процентная запись: путь как есть.
  }
  const m = /(?:^|\/)workspaces\/([^/]+)\/(.+?)\/?$/.exec(raw.replace(/\\/g, '/'));
  if (!m || m[2].split('/').some((part) => part === '..' || part === '')) return null;
  return { owner: m[1], rel: m[2] };
}

// Настоящий адрес файла для href: «Копировать ссылку» в меню WebView берёт href, а с `#` копировался адрес окна.
// Щелчок по-прежнему перехватывает app.js (preventDefault) и открывает «Файлы», навигации по file: нет.
export function fileHref(path) {
  let raw = String(path ?? '').trim().replace(/^file:\/\/\//i, '');
  try {
    raw = decodeURIComponent(raw);
  } catch {
    // Не процентная запись: путь как есть.
  }
  return `file:///${encodeURI(raw.replace(/\\/g, '/')).replace(/#/g, '%23').replace(/\?/g, '%3F')}`;
}

function fileAnchor(path, label) {
  const attr = path.replace(/"/g, '&quot;');
  return `<a href="${fileHref(path)}" class="md-file" data-file="${attr}" title="${attr}">${label}</a>`;
}

// Режим «чужой текст» (ответы ботов другого ПК): ссылки на файлы не оживают, а у каждой ссылки рядом виден настоящий адрес
// и пометка источника: подпись ссылки текстом ответа может быть любой. Метку подставляет вызывающий (tr).
let remote = null;

function anchor(href, label) {
  if (remote) {
    return `<a href="${href}" target="_blank" rel="noreferrer noopener" class="md-remote-link">${label}</a><span class="md-remote-url"> (${href} — ${remote.mark})</span>`;
  }
  const icon = linkIcon(href);
  return `<a href="${href}" target="_blank" rel="noreferrer"${icon ? ' class="has-ico"' : ''}>${icon}${label}</a>`;
}

const BARE_URL = /(?:(?:https?|tg):\/\/|\b(?:t\.me|telegram\.me|web\.telegram\.org)\/)[^\s<>"\u0000-\u0002]+/gi;

// Строка текста (уже экранированная) -> HTML. Код и готовые ссылки прячем в заглушки,
// чтобы жирный и автоссылки не лезли внутрь.
function inline(text) {
  const keep = [];
  const hold = (html) => `\u0001${keep.push(html) - 1}\u0001`;
  let s = text;
  s = s.replace(/`([^`\n]+)`/g, (_, code) => hold(`<code>${code}</code>`));
  s = s.replace(/\[([^\]\n]+)\]\(((?:https?|tg):\/\/[^)\s]+)\)/gi, (_, label, href) => hold(anchor(href, label)));
  s = s.replace(/\[([^\]\n]+)\]\(((?:file:\/\/\/)?[A-Za-z]:[\\/][^)\n]*)\)/g, (whole, label, path) =>
    !remote && workspaceRef(path) ? hold(fileAnchor(path, label)) : whole);
  s = s.replace(BARE_URL, (url) => {
    const tail = /[.,;:!?)\]]+$/.exec(url)?.[0] || '';
    const bare = tail ? url.slice(0, -tail.length) : url;
    const href = /^[a-z]+:\/\//i.test(bare) ? bare : `https://${bare}`;
    return hold(anchor(href, bare)) + tail;
  });
  s = s.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
  // @Имя в тексте: оранжевая отметка кита (.k-mention). Почта (a@b.c) не трогается: перед @ нужен пробел или скобка.
  s = s.replace(/(^|[\s(])@([\p{L}\p{N}_][\p{L}\p{N}_-]*)/gu, '$1<span class="k-mention">@$2</span>');
  return s.replace(/\u0001(\d+)\u0001/g, (_, i) => keep[Number(i)]);
}

const SEPARATOR = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

function cells(line) {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, '|'));
}

function isTableStart(lines, i) {
  return lines[i].includes('|') && i + 1 < lines.length && lines[i + 1].includes('-') && SEPARATOR.test(lines[i + 1]) && cells(lines[i]).length === cells(lines[i + 1]).length;
}

function table(lines, i) {
  const head = cells(lines[i]);
  const align = cells(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'c' : c.endsWith(':') ? 'r' : ''));
  const cls = (n) => (align[n] ? ` class="al-${align[n]}"` : '');
  let j = i + 2;
  const rows = [];
  while (j < lines.length && lines[j].trim() && lines[j].includes('|')) {
    rows.push(cells(lines[j]));
    j += 1;
  }
  const th = head.map((c, n) => `<th scope="col"${cls(n)}>${inline(c)}</th>`).join('');
  const body = rows
    .map((r) => `<tr>${head.map((_, n) => `<td${cls(n)}>${inline(r[n] ?? '')}</td>`).join('')}</tr>`)
    .join('');
  return { html: `<div class="md-table-wrap"><table class="md-table"><thead><tr>${th}</tr></thead><tbody>${body}</tbody></table></div>`, next: j };
}

function blocksOf(lines) {
  const out = [];
  let para = [];
  const flush = () => {
    if (para.length) out.push(`<p>${para.map(inline).join('<br>')}</p>`);
    para = [];
  };
  for (let i = 0; i < lines.length; ) {
    const line = lines[i];
    const heading = /^\s{0,3}(#{1,4})\s+(.+?)\s*#*\s*$/.exec(line);
    if (heading) {
      flush();
      const level = heading[1].length + 1; // # в ответе = h2: страница уже имеет свои h1
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      i += 1;
    } else if (/^\u0000\d+\u0000$/.test(line)) {
      flush();
      out.push(line);
      i += 1;
    } else if (isTableStart(lines, i)) {
      flush();
      const t = table(lines, i);
      out.push(t.html);
      i = t.next;
    } else if (/^\s*[-*]\s+/.test(line) || /^\s*\d+[.)]\s+/.test(line)) {
      flush();
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const item = ordered ? /^\s*\d+[.)]\s+/ : /^\s*[-*]\s+/;
      const items = [];
      while (i < lines.length && item.test(lines[i])) {
        items.push(`<li>${inline(lines[i].replace(item, ''))}</li>`);
        i += 1;
      }
      const tag = ordered ? 'ol' : 'ul';
      out.push(`<${tag}>${items.join('')}</${tag}>`);
    } else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush();
      out.push('<hr>');
      i += 1;
    } else {
      para.push(line);
      i += 1;
    }
  }
  flush();
  return out;
}

// options.remote: { mark } включает режим чужого текста (см. anchor). По умолчанию разметка прежняя.
export function markdown(text, options = {}) {
  remote = options.remote ? { mark: escapeHtml(options.remote.mark || 'from another PC') } : null;
  try {
    return render(text);
  } finally {
    remote = null;
  }
}

function render(text) {
  const blocks = [];
  let s = escapeHtml(String(text ?? ''));
  s = s.replace(/```[a-z]*\n([\s\S]*?)```/gi, (_, code) => {
    blocks.push(`<pre><code>${code.replace(/\n$/, '')}</code></pre>`);
    return `\u0000${blocks.length - 1}\u0000`;
  });
  const html = s
    .split(/\n{2,}/)
    .map((chunk) => blocksOf(chunk.split('\n')).join(''))
    .join('');
  return html.replace(/\u0000(\d+)\u0000/g, (_, i) => blocks[Number(i)]);
}
