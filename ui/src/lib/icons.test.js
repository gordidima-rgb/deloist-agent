import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { ICONS, icon, paintIcons } from './icons.js';

// Тест идёт из папки ui/, поэтому корень репозитория — на уровень выше.
const ROOT = join(process.cwd(), '..');

describe('icon registry', () => {
  it('keeps only drawing markup: no fill, stroke or colours', () => {
    for (const [name, body] of Object.entries(ICONS)) {
      expect(body, name).not.toMatch(/\b(?:fill|stroke)\s*=/);
      expect(body, name).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
      expect(body, name).not.toMatch(/<svg|<\/svg|<g\b|<style|<script/);
      expect(body, name).toMatch(/^<(?:path|circle|rect|ellipse)\b/);
    }
  });

  it('names are kebab-case and one drawing has one name', () => {
    const seen = new Map();
    for (const [name, body] of Object.entries(ICONS)) {
      expect(name, name).toMatch(/^[a-z0-9]+(?:-[a-z0-9]+)*$/);
      expect(seen.get(body), `${name} repeats ${seen.get(body)}`).toBeUndefined();
      seen.set(body, name);
    }
  });

  it('icon() writes the kit attributes for every name', () => {
    for (const name of Object.keys(ICONS)) {
      const svg = icon(name);
      expect(svg, name).toContain('viewBox="0 0 24 24"');
      expect(svg, name).toContain('fill="none"');
      expect(svg, name).toContain('stroke="currentColor"');
      expect(svg, name).toContain('stroke-width="2"');
      expect(svg, name).toContain('stroke-linecap="round"');
      expect(svg, name).toContain('stroke-linejoin="round"');
      expect(svg, name).toContain('aria-hidden="true"');
      expect(svg, name).toContain(ICONS[name]);
    }
  });

  it('icon() takes size, stroke, class and label; an unknown name throws', () => {
    const svg = icon('close', { size: 12, stroke: 2.4, className: 'i t-chev' });
    expect(svg).toContain('class="i t-chev"');
    expect(svg).toContain('width="12" height="12"');
    expect(svg).toContain('stroke-width="2.4"');
    expect(icon('close')).not.toContain(' width=');
    const named = icon('close', { label: 'Закрыть' });
    expect(named).toContain('role="img" aria-label="Закрыть"');
    expect(named).not.toContain('aria-hidden');
    expect(() => icon('no-such-icon')).toThrow('unknown icon: no-such-icon');
  });

  it('paintIcons() fills the static markup of the pages', () => {
    document.body.innerHTML = '<svg class="i" stroke-width="2.2" data-icon="send"></svg><svg data-icon="plus"></svg>';
    paintIcons();
    const [first, second] = document.querySelectorAll('svg');
    expect(first.getAttribute('stroke-width')).toBe('2.2');
    expect(first.getAttribute('class')).toBe('i');
    expect(first.querySelectorAll('path')).toHaveLength(2);
    expect(second.getAttribute('stroke-width')).toBe('2');
    expect(second.firstElementChild.namespaceURI).toBe('http://www.w3.org/2000/svg');
  });
});

// Что не значок и поэтому остаётся рисунком на месте: [файл, подстрока в svg или в теге]. '*' — весь файл.
const NOT_ICONS = [
  // Кольца лимитов и использования
  ['ui/src/components/kit/status/LimitRing.svelte', '*'],
  ['ui/src/components/MicButton.svelte', 'k-mic__ring'],
  ['web/app.js', 'class="ring"'],
  ['web/providers.js', 'usage-ring'],
  ['web/providers.js', 'usage-track'],
  ['web/providers.js', 'usage-fill'],
  ['web/team.js', 'tm-ring'],
  // Ручка дока
  ['ui/src/components/Dock.svelte', 'k-grip'],
  // Логотипы брендов
  ['web/index.html', 'brand-mark'],
  ['web/signin.js', 'viewBox="0 0 64 64"'],
  ['web/signin.js', 'path fill="currentColor"'],
  ['web/providers.js', 'usage-mark'],
  ['ui/src/lib/markdown.js', '*'],
  // Рисунки холста рабочих процессов
  ['ui/src/components/workflows/StrokesLayer.svelte', '*'],
  ['ui/src/components/workflows/ToolArt.svelte', '*'],
  ['ui/src/components/workflows/ToolOptions.svelte', 'sh.id'],
  ['ui/src/components/workflows/WfShape.svelte', '*'],
  ['ui/src/components/workflows/WfEdge.svelte', '*'],
];

function walk(dir, keep, out = []) {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) walk(path, keep, out);
    else if (keep(path)) out.push(path);
  }
  return out;
}

const rel = (path) => relative(ROOT, path).split(sep).join('/');
const SHAPE = '(?:path|circle|rect|line|polyline|polygon|ellipse)';

function stripComments(text) {
  return text
    .replace(/<!--[\s\S]*?-->/g, '')
    .split('\n')
    .filter((line) => !/^\s*(?:\/\/|\/\*|\*)/.test(line))
    .join('\n');
}

function allowed(file, text) {
  return NOT_ICONS.some(([name, marker]) => name === file && (marker === '*' || text.includes(marker)));
}

// Рисунки, написанные прямо в файле: <svg> с фигурами внутри и голая разметка фигур.
function inlineDrawings(file, source) {
  let text = stripComments(source);
  const found = [];
  text = text.replace(/<svg\b[\s\S]*?<\/svg\s*>/g, (block) => {
    if (new RegExp(`<${SHAPE}\\b`).test(block) && !allowed(file, block)) found.push(block.replace(/\s+/g, ' ').slice(0, 100));
    return '';
  });
  for (const tag of text.match(new RegExp(`<${SHAPE}\\b[^>]*>`, 'g')) ?? []) {
    if (!allowed(file, tag)) found.push(tag.slice(0, 100));
  }
  return found;
}

describe('no inline icons outside the registry', () => {
  const files = [
    ...walk(join(ROOT, 'web'), (p) => /\.(?:js|html)$/.test(p) && !p.slice(ROOT.length).includes(`${sep}ui${sep}`) && !p.slice(ROOT.length).includes(`${sep}vendor${sep}`) && !p.slice(ROOT.length).includes(`${sep}icons${sep}`)),
    ...walk(join(ROOT, 'ui', 'src'), (p) => /\.(?:svelte|js)$/.test(p) && !/\.test\.js$/.test(p)),
  ].filter((p) => !['ui/src/lib/icons.js', 'ui/src/lib/providerIcons.js'].includes(rel(p)));

  it('scans the pages and components', () => {
    expect(files.length).toBeGreaterThan(80);
    expect(files.map(rel)).toEqual(expect.arrayContaining(['web/app.js', 'web/index.html', 'web/dock.html', 'ui/src/components/Dock.svelte']));
  });

  it('finds an icon drawn inline (the guard itself works)', () => {
    expect(inlineDrawings('web/app.js', '<svg viewBox="0 0 24 24"><path d="M1 1"/></svg>')).toHaveLength(1);
    expect(inlineDrawings('web/app.js', "const X = '<path d=\"M1 1\"/>';")).toHaveLength(1);
    expect(inlineDrawings('web/app.js', '<svg data-icon="close"></svg>')).toHaveLength(0);
    expect(inlineDrawings('web/app.js', '// <svg><path d="M1 1"/></svg>')).toHaveLength(0);
  });

  it('keeps every icon in ui/src/lib/icons.js', () => {
    const offenders = [];
    for (const path of files) {
      for (const hit of inlineDrawings(rel(path), readFileSync(path, 'utf8'))) offenders.push(`${rel(path)}: ${hit}`);
    }
    expect(offenders).toEqual([]);
  });
});
