#!/usr/bin/env node
// Проверка правки интерфейса Deloist Боты по киту «Угли» (Ember, docs/design/kit.md). Копия для 0.4.1: при слиянии заменяет скилл bots-kit-check.
// Один источник: вне ui/src/components/kit/** и web/kit.css нельзя рисовать свои кнопки, поля, цвета, радиусы, размеры шрифта.
// Старые нарушения — файлы из docs/design/ember/duplicates.md («Файлы-исключения»): там предупреждения, в остальных ошибки.
// Только правила, записанные в kit.md; у каждой находки — раздел кита, откуда правило.
// Смотрит строки, которые ветка добавила к основе (merge-base с origin/main) и незакоммиченные правки.
//
//   node check.mjs [--repo <путь>] [--base <ref>] [--all <файл…>] [--json]
//
// Выход: 0 — ошибок нет (предупреждения возможны), 1 — есть ошибки, 2 — сбой запуска.
import { execFileSync } from 'node:child_process';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const args = process.argv.slice(2);
const opt = (name, fallback = '') => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] && !args[i + 1].startsWith('--') ? args[i + 1] : fallback;
};
const repo = opt('--repo', process.cwd());
const asJson = args.includes('--json');
const allIndex = args.indexOf('--all');
const allFiles = allIndex >= 0 ? args.slice(allIndex + 1).filter((a) => !a.startsWith('--')) : null;
const git = (...a) => execFileSync('git', ['-C', repo, ...a], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });

// Исходники интерфейса; собранное (web/ui, web/vendor) и тесты не смотрим.
const LOOKED = /^(web\/[^/]+\.(js|css|html)|ui\/src\/.+\.(svelte|js|css))$/;
const SKIPPED = /(^web\/ui\/|^web\/vendor\/|\.test\.js$|\.spec\.js$|\.test\.svelte$)/;
// «Внутри кита»: вся папка ui/src/components/kit/** и web/kit.css. Остальное — экраны.
const inKit = (file) => file.startsWith('ui/src/components/kit/') || file === 'web/kit.css';
const DUP_FILE = 'docs/design/ember/duplicates.md';
const legacy = new Set();
try {
  const md = readFileSync(join(repo, DUP_FILE), 'utf8');
  const at = md.indexOf('## Файлы-исключения');
  if (at >= 0) for (const m of md.slice(at).matchAll(/^- `([^`]+)`/gm)) legacy.add(m[1]);
} catch {
  /* списка нет — всё ошибки */
}
const wanted = (file) => LOOKED.test(file) && !SKIPPED.test(file);

function addedLines() {
  if (allFiles) {
    const out = {};
    for (const f of allFiles) {
      const rel = f.replace(/\\/g, '/');
      out[rel] = new Set(readFileSync(join(repo, rel), 'utf8').split('\n').map((_, i) => i + 1));
    }
    return out;
  }
  let base = opt('--base');
  if (!base) {
    try {
      base = git('merge-base', 'HEAD', 'origin/main').trim();
    } catch {
      base = 'origin/main';
    }
  }
  const diff = git('diff', '--unified=0', '--no-color', '--no-ext-diff', base, '--');
  const out = {};
  let file = null;
  for (const line of diff.split('\n')) {
    if (line.startsWith('+++ ')) {
      const name = line.slice(4).replace(/^b\//, '');
      file = name === '/dev/null' || !wanted(name) ? null : name;
      if (file) out[file] ??= new Set();
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (hunk && file) {
      const start = Number(hunk[1]);
      const count = hunk[2] === undefined ? 1 : Number(hunk[2]);
      for (let n = start; n < start + count; n++) out[file].add(n);
    }
  }
  return out;
}

// Строка, где отступление осознанное, помечается комментарием «kit-check: allow — причина».
const ALLOW = /kit-check:\s*allow/;
const TOKEN_DEF = /^\s*--[\w-]+\s*:/;
const HEX = /#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})\b/gi;
const FUNC_COLOR = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(\s*[^)]*\)/gi;
const COLOR_PROP = /(color|background|fill|stroke|border|outline|shadow|accent|caret)/i;

// Отступы: «кратны 4: 2, 4, 8, 12, 16, 20, 24, 32, 48»; 6 — внутренний отступ меню (раздел «Меню»).
const SPACING = new Set([0, 1, 2, 4, 6, 8, 12, 16, 20, 24, 32, 48]);
// Радиусы: 6, 12, 18, 26, 34, 28, 999 и концентричные из разделов «Меню» и «Отступы» (14, 20).
// Угли: 6 галочки · 10 кнопки, фишки, пункты · 14 поля, строки, меню · 20 врезки · 24 карточки · 30 окно; 999 и 50% — пилюля и круг.
const RADII = new Set([0, 6, 10, 14, 20, 24, 30, 999]);

function lineFindings(file, text, n, ctx) {
  const out = [];
  if (ALLOW.test(text)) return out;
  const kit = inKit(file);
  // Новое вне кита — ошибка; файл из списка дублей — предупреждение.
  const add = (level, rule, where, message) =>
    out.push({ file, line: n, level: level === 'error' && legacy.has(file) ? 'warn' : level, rule, where, message, text: text.trim().slice(0, 160) });
  const code = text.replace(/\/\*.*?\*\//g, '').replace(/(^|\s)\/\/.*$/, '$1');

  // «Цвет»: все цвета — токены --k-* в web/kit.css.
  if (!TOKEN_DEF.test(code) && file !== 'web/kit.css') {
    const inStyle = ctx.css || /style=|\.style\.|COLOR|color|background|fill|stroke|border|shadow/i.test(code);
    if (inStyle) {
      for (const h of code.match(HEX) || []) {
        const i = code.indexOf(h);
        if (/[\w/&?=]$/.test(code.slice(0, i)) || /href=|url\(|#[a-z-]+\)/i.test(code.slice(Math.max(0, i - 8), i + h.length + 1))) continue;
        if (!ctx.css && !COLOR_PROP.test(code)) continue;
        add('error', 'color-token', 'Цвет', `цвет ${h} вписан в код: все цвета — токены --k-* из web/kit.css`);
      }
      for (const f of code.match(FUNC_COLOR) || []) {
        if (/var\(/.test(f) && !/\d\s*,\s*\d/.test(f.replace(/var\([^)]*\)/g, ''))) continue;
        add('error', 'color-token', 'Цвет', `цвет ${f.slice(0, 40)} вписан в код: все цвета — токены --k-* (тени — --k-shadow-*)`);
      }
    }
  }

  // «Цвет»: нет градиентного текста.
  if (/(?:-webkit-)?background-clip\s*:\s*text/.test(code)) add('error', 'gradient-text', 'Цвет', 'градиентный текст');

  // «Шрифт».
  if (/\b(Inter|Unbounded)\b/.test(code) && /font/i.test(code)) add('error', 'font', 'Шрифт', 'Inter и Unbounded не используем нигде');
  const weight = /font-weight\s*:\s*(\d{3})/.exec(code) || /font\s*:\s*(\d{3})\s/.exec(code);
  if (weight && !['400', '600', '700', '800'].includes(weight[1])) add('error', 'font', 'Шрифт', `начертание ${weight[1]}: только 400, 600, 700 и 800 (700/800 — заголовки Onest)`);
  if (ctx.css && /text-transform\s*:\s*uppercase/.test(code)) add('warn', 'font', 'Шрифт', 'капс: нет надписям капсом над заголовками');

  // «Отступы, скругления, глубина».
  if (ctx.css) {
    const odd = [];
    for (const m of code.matchAll(/(?:^|[\s;{])(?:margin|padding|gap|row-gap|column-gap)(?:-[a-z]+)?\s*:\s*([^;}]+)/g)) {
      for (const v of m[1].match(/(?<![\w.-])(\d+(?:\.\d+)?)px/g) || []) {
        const px = parseFloat(v);
        if (!SPACING.has(px) && !odd.includes(px)) odd.push(px);
      }
    }
    if (odd.length) add('warn', 'spacing', 'Отступы', `отступ ${odd.map((v) => `${v}px`).join(', ')}: в ките 2, 4, 8, 12, 16, 20, 24, 32, 48 (--k-space-*)`);
    const radius = /border-radius\s*:\s*([^;}]+)/.exec(code);
    if (radius && !/var\(/.test(radius[1])) {
      const bad = (radius[1].match(/(\d+(?:\.\d+)?)px/g) || []).map(parseFloat).filter((v) => !RADII.has(v));
      if (!kit && !/^\s*(0|50%|inherit)\s*$/.test(radius[1])) add('error', 'radius-token', 'Скругления', 'border-radius вне кита только токеном --k-radius-* (6, 10, 14, 20, 24, 30)');
      else if (bad.length) add('warn', 'radius', 'Скругления', `радиус ${bad.join(', ')}px: в «Углях» 6, 10, 14, 20, 24, 30`);
    }
    if (!kit && /(?:^|[\s;{])font(?:-size)?\s*:[^;]*\d(?:px|rem|em)\b/.test(code) && !/var\(--k-text/.test(code)) add('error', 'font-token', 'Шрифт', 'размер шрифта вне кита только токеном --k-text-*');
    if (!kit && ctx.selector && /(?:^|[\s,>+~])(?:button|\.[\w-]*(?:btn|button|primary|ghost)[\w-]*)(?![\w-]*__)/.test(ctx.selector) && /(?:^|[\s;{])(?:min-)?(?:height|padding)\s*:\s*[^;}]*\d/.test(code) && !/var\(--k-/.test(code)) add('error', 'button-size', 'Кнопка', 'свои height/padding у кнопки: размер даёт Button (size sm/md)');
    if (!kit && /^\s*(?:[^@{}]*[\s,>+~])?\.(?:btn|button|primary|ghost-btn|cta)\b[^{]*\{/.test(code)) add('error', 'own-button-class', 'Кнопка', 'самодельный класс кнопки: используй Button / IconButton (.k-btn)');
    if (/border-left\s*:\s*[2-9]px\s+solid/.test(code)) add('warn', 'side-stripe', 'Отступы', 'цветная полоса слева у карточки');
    if (/backdrop-filter\s*:\s*blur/.test(code)) add('warn', 'glass', 'Отступы', 'стекло и размытие фона — только если без них не читается');
  }

  // Один источник: кнопки и выбор вне кита берутся из кита.
  if (!kit && !ctx.css) {
    for (const m of code.matchAll(/<button\b[^>]*>/g)) {
      const cls = /class=(?:"([^"]*)"|'([^']*)')/.exec(m[0]);
      if (cls && !/\bk-/.test(cls[1] ?? cls[2] ?? '')) add('error', 'own-button-class', 'Кнопка', 'кнопка с самодельным классом: используй Button / IconButton (.k-btn)');
    }
    if (/<select\b/.test(code)) add('error', 'native-select', 'Выбор', 'нативный <select>: используй Select из кита');
  }

  // «Кнопка»: глагол действия; нет «ОК» и стрелки в тексте кнопки.
  const label = />\s*([^<>{}]{1,40}?)\s*<\/button>/.exec(code)?.[1] || '';
  if (/^(ок|ok)$/i.test(label.trim())) add('error', 'button-text', 'Кнопка', 'кнопка «ОК»: подпись — глагол действия');
  if (/[→←↗➜➔]|-->|->/.test(label)) add('error', 'button-text', 'Кнопка', 'стрелка в тексте кнопки');

  // «Поле»: нет «Неверное значение».
  if (/Неверное значение/.test(code)) add('error', 'field-error', 'Поле', 'ошибка должна говорить, что случилось и что сделать');
  return out;
}

// «Иконки»: Lucide, viewBox 24, fill none, stroke currentColor, round; толщина 2 в навигации и у поиска/справки.
// Реестр значков (ui/src/lib/icons.js) сам собирает svg по правилу и стережётся своим тестом.
const ICON_REGISTRY = /(^|\/)ui\/src\/lib\/icons\.js$/;
// Атрибут задан строкой или выражением Svelte: stroke-width="2" или stroke-width={stroke}.
const has = (tag, attr, value) => (value ? new RegExp(`${attr}=["']${value}["']`).test(tag) || new RegExp(`${attr}=\\{`).test(tag) : new RegExp(`${attr}=["'{]`).test(tag));

function iconFindings(file, text, added) {
  const out = [];
  if (ICON_REGISTRY.test(file)) return out;
  const rows = text.split('\n');
  for (const m of text.matchAll(/<svg\b[^>]*>/g)) {
    const line = text.slice(0, m.index).split('\n').length;
    if (!added.has(line) || ALLOW.test(rows[line - 1] || '')) continue;
    // svg в комментарии — не разметка.
    if (/^\s*(\/\/|\*|<!--)/.test(rows[line - 1] || '')) continue;
    const tag = m[0];
    // Картинка, а не иконка (лого, график, кольцо): role="img".
    if (/role=["']img["']/.test(tag)) continue;
    // Рисунок кита (кольцо, график), помеченный data-art, — не иконка.
    if (/\sdata-art[\s>=]/.test(tag)) continue;
    const add = (level, message) => out.push({ file, line, level, rule: 'icon', where: 'Иконки', message, text: tag.slice(0, 160) });
    if (!has(tag, 'viewBox', '0 0 24 24')) add('error', 'нужна сетка viewBox="0 0 24 24"');
    if (!has(tag, 'fill', 'none')) add('error', 'нужен fill="none": иконки линейные');
    if (!has(tag, 'stroke', 'currentColor')) add('error', 'нужен stroke="currentColor": цвет иконка берёт у текста рядом');
    if (!has(tag, 'stroke-linecap', 'round') || !has(tag, 'stroke-linejoin', 'round')) add('error', 'нужны скруглённые концы и углы: stroke-linecap="round" stroke-linejoin="round"');
    // Толщина выражением ({stroke}) задаётся вызывающим: проверяем её там, где стоит число.
    if (/stroke-width=\{/.test(tag)) continue;
    const sw = /stroke-width=["']([\d.]+)["']/.exec(tag)?.[1];
    const near = text.slice(Math.max(0, m.index - 400), m.index);
    const nav = /sidebar|BotList|SideNav|DrawerHead|drawer-rail|k-tab|nav\b|<nav|search|help|поиск|справк/i.test(`${file}\n${near}`);
    if (nav && sw !== '2') add('error', `толщина линии ${sw || 'не задана'}: в навигации и у кнопок поиска и справки — 2`);
  }
  return out;
}

function check() {
  const findings = [];
  for (const [file, lines] of Object.entries(addedLines())) {
    if (!lines.size || !existsSync(join(repo, file))) continue;
    const text = readFileSync(join(repo, file), 'utf8');
    const isCss = file.endsWith('.css');
    let inStyle = isCss;
    let selector = '';
    text.split('\n').forEach((row, i) => {
      if (/<style[\s>]/.test(row)) inStyle = true;
      if (inStyle && /\{/.test(row) && !/^\s*@/.test(row) && !/^\s*--/.test(row)) selector = row.split('{')[0];
      if (lines.has(i + 1)) findings.push(...lineFindings(file, row, i + 1, { css: inStyle, selector }));
      if (/<\/style>/.test(row)) inStyle = isCss;
    });
    if (/\.(svelte|js|html)$/.test(file)) findings.push(...iconFindings(file, text, lines));
  }
  const seen = new Set();
  return findings.filter((f) => {
    const key = `${f.file}:${f.line}:${f.message}`;
    return seen.has(key) ? false : (seen.add(key), true);
  });
}

try {
  const findings = check();
  const errors = findings.filter((f) => f.level === 'error');
  const warns = findings.filter((f) => f.level === 'warn');
  if (asJson) {
    console.log(JSON.stringify({ errors: errors.length, warnings: warns.length, findings }, null, 1));
  } else {
    for (const f of [...errors, ...warns]) {
      console.log(`${f.level === 'error' ? 'ОШИБКА' : 'предупр'}  ${f.file}:${f.line}  [${f.rule}, kit.md → ${f.where}] ${f.message}`);
      console.log(`         ${f.text}`);
    }
    console.log(`\nbots-kit-check: ошибок ${errors.length}, предупреждений ${warns.length}`);
  }
  process.exit(errors.length ? 1 : 0);
} catch (err) {
  console.error(`bots-kit-check: ${err.message}`);
  process.exit(2);
}
