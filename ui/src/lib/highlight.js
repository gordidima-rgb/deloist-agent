// Подсветка кода в просмотрщике файлов: @speed-highlight/core (CC0, без зависимостей, разбирает по регулярным выражениям).
// Все языки лежат в одном бандле (сборка — одна IIFE), поэтому берём только нужные, ~17 КБ минифицированных.
// Возвращает по одной HTML-строке на строку файла: токен, растянутый на несколько строк, закрывается и открывается заново.
import { tokenizeWith } from '@speed-highlight/core/tokenize';
import bash from '@speed-highlight/core/languages/bash.js';
import c from '@speed-highlight/core/languages/c.js';
import asm from '@speed-highlight/core/languages/asm.js';
import css from '@speed-highlight/core/languages/css.js';
import diff from '@speed-highlight/core/languages/diff.js';
import go from '@speed-highlight/core/languages/go.js';
import html from '@speed-highlight/core/languages/html.js';
import ini from '@speed-highlight/core/languages/ini.js';
import java from '@speed-highlight/core/languages/java.js';
import js from '@speed-highlight/core/languages/js.js';
import jsdoc from '@speed-highlight/core/languages/jsdoc.js';
import json from '@speed-highlight/core/languages/json.js';
import lua from '@speed-highlight/core/languages/lua.js';
import make from '@speed-highlight/core/languages/make.js';
import md from '@speed-highlight/core/languages/md.js';
import py from '@speed-highlight/core/languages/py.js';
import regex from '@speed-highlight/core/languages/regex.js';
import rs from '@speed-highlight/core/languages/rs.js';
import sql from '@speed-highlight/core/languages/sql.js';
import todo from '@speed-highlight/core/languages/todo.js';
import toml from '@speed-highlight/core/languages/toml.js';
import ts from '@speed-highlight/core/languages/ts.js';
import xml from '@speed-highlight/core/languages/xml.js';
import yaml from '@speed-highlight/core/languages/yaml.js';

// Языки, на которые ссылаются вложенные правила (html → css и js, js → jsdoc и regex, c → asm, make → bash).
const LANGUAGES = { bash, c, asm, css, diff, go, html, ini, java, js, jsdoc, json, lua, make, md, py, regex, rs, sql, todo, toml, ts, xml, yaml };

const BY_EXTENSION = {
  html: 'html', htm: 'html', svelte: 'html', vue: 'html',
  css: 'css', scss: 'css',
  js: 'js', mjs: 'js', cjs: 'js', jsx: 'js',
  ts: 'ts', tsx: 'ts',
  json: 'json', jsonl: 'json',
  py: 'py', rs: 'rs', go: 'go', lua: 'lua', sql: 'sql',
  java: 'java', kt: 'java', cs: 'java', swift: 'java',
  c: 'c', h: 'c', cpp: 'c', hpp: 'c',
  sh: 'bash', bash: 'bash', zsh: 'bash', ps1: 'bash', bat: 'bash', cmd: 'bash',
  md: 'md', markdown: 'md',
  yaml: 'yaml', yml: 'yaml', toml: 'toml',
  xml: 'xml', svg: 'xml',
  diff: 'diff', patch: 'diff',
  ini: 'ini', cfg: 'ini', conf: 'ini', env: 'ini',
  mk: 'make',
};

/** Язык подсветки по имени файла или `null`, если для формата правил нет. */
export function languageOf(fileName) {
  const base = String(fileName ?? '').split(/[\\/]/).pop().toLowerCase();
  if (base === 'makefile') return 'make';
  if (base === 'dockerfile') return 'bash';
  const dot = base.lastIndexOf('.');
  return BY_EXTENSION[dot >= 0 ? base.slice(dot + 1) : base] ?? null;
}

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;' };
const escape = (s) => s.replace(/[&<>]/g, (ch) => ESC[ch]);

/**
 * Подсвеченные строки файла: массив HTML (текст экранирован, токены — `<span class="shj-syn-…">`).
 * Для неизвестного языка или при ошибке разбора — те же строки без разметки.
 */
export function highlightLines(source, language) {
  const text = String(source ?? '').replace(/\r\n?/g, '\n').replace(/\n$/, '');
  const lang = LANGUAGES[language];
  const plain = () => text.split('\n').map(escape);
  if (!lang) return plain();
  const lines = [''];
  try {
    tokenizeWith(text, lang, (chunk, type) => {
      if (!chunk) return;
      chunk.split('\n').forEach((piece, i) => {
        if (i > 0) lines.push('');
        if (!piece) return;
        const safe = escape(piece);
        lines[lines.length - 1] += type ? `<span class="shj-syn-${type}">${safe}</span>` : safe;
      });
    }, { languages: LANGUAGES });
  } catch {
    return plain();
  }
  return lines;
}
