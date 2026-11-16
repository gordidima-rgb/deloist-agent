// Проверка перевода интерфейса. Запуск из корня репозитория: node tools/check_i18n.mjs
// Без зависимостей. Выходит с кодом 1 и печатает найденное, если перевод неполон.
//
//  a) каждый ключ tr('…') из app.js, updates.js и i18n.js есть в словаре EN;
//  b) в app.js не осталось русских строк вне комментариев, ключей tr(), регулярных выражений;
//     исключения проверяются отдельно: слова plural() (формы one и many) и строки ERROR_KINDS — они обязаны быть в EN;
//  c) весь видимый русский текст и placeholder/title/aria-label в index.html есть в EN (исключение: «Русский»);
//  e) компоненты ui/src/**/*.svelte: ключи tr() есть в EN, русского текста вне tr() нет;
//  d) плейсхолдеры {name} у ключа и у значения совпадают.
//
// Словарь EN читается из web/i18n.js: литерал между `const EN = {` и первой строкой `  };`.

import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (rel) => readFileSync(join(root, rel), 'utf8').replace(/\r\n/g, '\n');
const CYR = /[Ѐ-ӿ]/;
const problems = [];
const fail = (msg) => problems.push(msg);

// ---------------------------------------------------------------------------
// Словарь
// ---------------------------------------------------------------------------

const i18nSrc = read('web/i18n.js');
const enStart = i18nSrc.indexOf('const EN = {');
const enEnd = i18nSrc.indexOf('\n  };', enStart);
if (enStart < 0 || enEnd < 0) {
  console.log('Не нашёл литерал `const EN = {` … `  };` в web/i18n.js');
  process.exit(1);
}
const enLiteral = i18nSrc.slice(enStart + 'const EN = '.length, enEnd + '\n  }'.length);
const EN = new Function(`return (${enLiteral});`)();

// ---------------------------------------------------------------------------
// Разбор JS: комментарии и регулярки гасим, ключи tr() вынимаем
// ---------------------------------------------------------------------------

const REGEX_AFTER = new Set(['', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '<', '>', '~', '^']);
const REGEX_AFTER_WORD = new Set(['return', 'typeof', 'case', 'in', 'of', 'delete', 'void', 'throw', 'new', 'else', 'do']);

function scanJs(src) {
  let out = '';
  const keys = []; // {key, line}
  const dynamic = []; // номера строк с tr(`…${…}`)
  const n = src.length;
  let line = 1;

  const lineAt = (idx) => src.slice(0, idx).split('\n').length;

  function readString(i, quote) {
    let j = i + 1;
    while (j < n && src[j] !== quote) j += src[j] === '\\' ? 2 : 1;
    return j; // индекс закрывающей кавычки
  }

  function unescape(raw) {
    return raw.replace(/\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|[\s\S])/g, (_, e) => {
      if (e[0] === 'u') return String.fromCodePoint(parseInt(e.replace(/[u{}]/g, ''), 16));
      if (e[0] === 'x') return String.fromCharCode(parseInt(e.slice(1), 16));
      return { n: '\n', t: '\t', r: '\r' }[e] ?? e;
    });
  }

  // Шаблон: текст копируем, `${…}` разбираем как код.
  function parseTemplate(i) {
    out += '`';
    let j = i + 1;
    while (j < n) {
      const c = src[j];
      if (c === '\\') { out += src.slice(j, j + 2); j += 2; continue; }
      if (c === '`') { out += '`'; return j + 1; }
      if (c === '$' && src[j + 1] === '{') {
        out += '${';
        j = parseCode(j + 2, true);
        out += '}';
        continue;
      }
      out += c;
      j += 1;
    }
    return j;
  }

  function parseCode(i, stopAtBrace) {
    let depth = 0;
    let last = '';
    let word = '';
    while (i < n) {
      const c = src[i];
      const d = src[i + 1];
      if (c === '/' && d === '/') {
        while (i < n && src[i] !== '\n') i += 1;
        continue;
      }
      if (c === '/' && d === '*') {
        const end = src.indexOf('*/', i + 2);
        const stop = end < 0 ? n : end + 2;
        out += src.slice(i, stop).replace(/[^\n]/g, ' ');
        i = stop;
        continue;
      }
      if (c === "'" || c === '"') {
        const end = readString(i, c);
        out += src.slice(i, end + 1);
        i = end + 1;
        last = 'S';
        word = '';
        continue;
      }
      if (c === '`') {
        i = parseTemplate(i);
        last = 'S';
        word = '';
        continue;
      }
      if (c === '/' && (REGEX_AFTER.has(last) || REGEX_AFTER_WORD.has(word))) {
        let j = i + 1;
        let inClass = false;
        while (j < n && (inClass || src[j] !== '/')) {
          if (src[j] === '\\') j += 1;
          else if (src[j] === '[') inClass = true;
          else if (src[j] === ']') inClass = false;
          j += 1;
        }
        j += 1;
        while (j < n && /[a-z]/i.test(src[j])) j += 1;
        out += '/R/';
        i = j;
        last = 'R';
        word = '';
        continue;
      }
      if (stopAtBrace) {
        if (c === '{') depth += 1;
        if (c === '}') {
          if (depth === 0) return i + 1;
          depth -= 1;
        }
      }
      if (/[A-Za-z_$]/.test(c)) {
        let j = i;
        while (j < n && /[\w$]/.test(src[j])) j += 1;
        const id = src.slice(i, j);
        // tr('…'): ключ вынимаем и подменяем пустой строкой.
        if (id === 'tr' && last !== '.' && word !== 'function') {
          const m = /^\s*\(\s*(['"`])/.exec(src.slice(j, j + 40));
          if (m) {
            const qStart = j + m[0].length - 1;
            const quote = m[1];
            const end = readString(qStart, quote);
            const raw = src.slice(qStart + 1, end);
            if (quote === '`' && raw.includes('${')) dynamic.push(lineAt(i));
            else keys.push({ key: unescape(raw), line: lineAt(i) });
            out += `tr(''`;
            i = end + 1;
            last = 'S';
            word = '';
            continue;
          }
        }
        out += id;
        i = j;
        last = 'W';
        word = id;
        continue;
      }
      if (!/\s/.test(c)) {
        last = c;
        word = '';
      }
      out += c;
      i += 1;
    }
    return i;
  }

  parseCode(0, false);
  void line;
  return { code: out, keys, dynamic };
}

const placeholders = (s) => [...String(s).matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');
const lineOf = (text, idx) => text.slice(0, idx).split('\n').length;
const inEn = (key) => Object.prototype.hasOwnProperty.call(EN, key);

// ---------------------------------------------------------------------------
// a) ключи tr() и b) остатки русского в app.js
// ---------------------------------------------------------------------------

const appSrc = read('web/app.js');
const files = ['web/app.js', 'web/updates.js', 'web/feedback.js', 'web/attach.js', 'web/signin.js', 'web/providers.js', 'web/screen.js', 'web/team.js', 'web/pet.js', 'web/dock.js', 'web/voice.js', 'web/stt.js', 'web/snap.js', 'web/i18n.js'];
let keyCount = 0;
let scanned;
for (const rel of files) {
  const res = rel === 'web/app.js' ? (scanned = scanJs(appSrc)) : scanJs(read(rel));
  keyCount += res.keys.length;
  for (const line of res.dynamic) fail(`${rel}:${line}: в tr(\`…\`) есть \${…}: ключ должен быть статичным, значения идут через vars`);
  for (const { key, line } of res.keys) {
    if (CYR.test(key) && !inEn(key)) fail(`${rel}:${line}: нет перевода для ключа «${key}»`);
  }
}

let code = scanned.code;
const wordsToEn = new Set();

// plural(): три слова допустимы, но one и many должны быть в EN.
code = code.replace(
  /plural\(((?:[^,'"()]|\([^()]*\))*),\s*'([^']*)',\s*'([^']*)',\s*'([^']*)'\s*\)/g,
  (whole, arg, one, few, many, offset) => {
    for (const [form, word] of [['one', one], ['many', many]]) {
      if (!inEn(word)) fail(`web/app.js:${lineOf(code, offset)}: plural(): нет перевода формы ${form} «${word}»`);
      wordsToEn.add(word);
    }
    return `plural(${arg}, '', '', '')`;
  },
);

// ERROR_KINDS: заголовки и подсказки переводятся при показе, регулярки уже погашены.
const ekStart = code.indexOf('const ERROR_KINDS = [');
if (ekStart < 0) fail('web/app.js: не нашёл ERROR_KINDS');
else {
  const ekEnd = code.indexOf('\n];', ekStart);
  const block = code.slice(ekStart, ekEnd);
  let cleaned = block;
  for (const m of block.matchAll(/'([^']*)'/g)) {
    if (CYR.test(m[1]) && !inEn(m[1])) fail(`web/app.js:${lineOf(code, ekStart + m.index)}: ERROR_KINDS: нет перевода «${m[1]}»`);
  }
  cleaned = block.replace(/'([^']*)'/g, "''");
  code = code.slice(0, ekStart) + cleaned + code.slice(ekEnd);
}

code.split('\n').forEach((text, i) => {
  if (CYR.test(text)) fail(`web/app.js:${i + 1}: русский текст вне tr(): ${text.trim().slice(0, 140)}`);
});

// Названия внешности приходят из avatar.js по-русски и показываются через tr(item.name).
const avatarSrc = existsSync(join(root, 'private/web/avatar.js')) ? read('private/web/avatar.js') : '';
for (const m of avatarSrc.matchAll(/name:\s*'([^']*)'/g)) {
  if (CYR.test(m[1]) && !inEn(m[1])) fail(`private/web/avatar.js: нет перевода названия «${m[1]}»`);
}

// ---------------------------------------------------------------------------
// c) index.html
// ---------------------------------------------------------------------------

const ALLOW_HTML = new Set(['Русский']);
const decode = (s) => s.replace(/&(amp|lt|gt|quot|#39|nbsp);/g, (_, e) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'", nbsp: ' ' }[e]));
const html = read('web/index.html').replace(/<!--[\s\S]*?-->/g, (c) => c.replace(/[^\n]/g, ' ')).replace(/<(script|style)[\s\S]*?<\/\1>/g, (c) => c.replace(/[^\n]/g, ' '));
let htmlCount = 0;
for (const m of html.matchAll(/<[^>]*>|[^<]+/g)) {
  const piece = m[0];
  if (piece.startsWith('<')) {
    for (const a of piece.matchAll(/\s(placeholder|title|aria-label)="([^"]*)"/g)) {
      const value = decode(a[2]).trim();
      if (!CYR.test(value) || ALLOW_HTML.has(value)) continue;
      htmlCount += 1;
      if (!inEn(value)) fail(`web/index.html:${lineOf(html, m.index)}: нет перевода атрибута ${a[1]}="${value}"`);
    }
  } else {
    const value = decode(piece).trim();
    if (!CYR.test(value) || ALLOW_HTML.has(value)) continue;
    htmlCount += 1;
    if (!inEn(value)) fail(`web/index.html:${lineOf(html, m.index)}: нет перевода текста «${value}»`);
  }
}

// ---------------------------------------------------------------------------
// e) компоненты Svelte (ui/src/**/*.svelte): те же правила, что для app.js
// ---------------------------------------------------------------------------

function svelteFiles(dir) {
  const found = [];
  let names = [];
  try {
    names = readdirSync(join(root, dir));
  } catch {
    return found;
  }
  for (const name of names) {
    const rel = `${dir}/${name}`;
    if (statSync(join(root, rel)).isDirectory()) found.push(...svelteFiles(rel));
    else if (name.endsWith('.svelte')) found.push(rel);
  }
  return found;
}

let svelteKeys = 0;
for (const rel of svelteFiles('ui/src')) {
  // Комментарии гасим, оставляя переводы строк (номера строк в сообщениях сохраняются).
  const blank = (c) => c.replace(/[^\n]/g, ' ');
  let text = read(rel)
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/(^|\s)\/\/\s.*$/gm, blank);
  text = text.replace(/\btr\(\s*(['"`])((?:\\.|(?!\1)[^\\])*)\1/g, (whole, quote, raw, offset) => {
    if (quote === '`' && raw.includes('${')) fail(`${rel}:${lineOf(text, offset)}: в tr(\`…\`) есть \${…}: ключ должен быть статичным`);
    else {
      svelteKeys += 1;
      if (CYR.test(raw) && !inEn(raw)) fail(`${rel}:${lineOf(text, offset)}: нет перевода для ключа «${raw}»`);
    }
    return "tr('')";
  });
  text.split('\n').forEach((line, i) => {
    if (CYR.test(line)) fail(`${rel}:${i + 1}: русский текст вне tr(): ${line.trim().slice(0, 140)}`);
  });
}

// ---------------------------------------------------------------------------
// d) плейсхолдеры (и повторы ключей: литерал молча оставил бы последний)
// ---------------------------------------------------------------------------

const seenKeys = new Set();
for (const m of enLiteral.matchAll(/^\s{4}'((?:[^'\\]|\\.)*)':/gm)) {
  if (seenKeys.has(m[1])) fail(`EN: ключ повторяется: «${m[1]}»`);
  seenKeys.add(m[1]);
}

for (const [key, value] of Object.entries(EN)) {
  if (typeof value !== 'string') fail(`EN «${key}»: значение не строка`);
  else if (placeholders(key) !== placeholders(value)) {
    fail(`EN «${key}»: плейсхолдеры {${placeholders(key)}} не совпадают с {${placeholders(value)}} у «${value}»`);
  }
}

// ---------------------------------------------------------------------------

if (problems.length) {
  console.log(problems.join('\n'));
  console.log(`\nПроблем: ${problems.length}`);
  process.exit(1);
}
console.log(`i18n OK: ключей tr() ${keyCount}, слов plural() ${wordsToEn.size}, строк index.html ${htmlCount}, записей в EN ${Object.keys(EN).length}, ключей в Svelte ${svelteKeys}`);
