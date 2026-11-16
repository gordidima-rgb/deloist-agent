// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
const html = readFileSync('../web/index.html', 'utf8');
const script = readFileSync('../web/providers.js', 'utf8').slice(readFileSync('../web/providers.js', 'utf8').lastIndexOf('(() => {'));
describe('sidebar footer', () => {
  beforeEach(() => { document.body.innerHTML = html.match(/<body>([\s\S]*)<\/body>/)[1]; window.tr = (v) => v; window.eval(script); document.dispatchEvent(new Event('DOMContentLoaded')); });
  it('opens and closes outside or with Escape', () => {
    const t = document.getElementById('btn-side-menu'), m = document.getElementById('side-menu');
    t.click(); expect(document.activeElement.id).toBe('btn-settings'); expect(m.hidden).toBe(false); expect(t.getAttribute('aria-expanded')).toBe('true'); document.body.click(); expect(m.hidden).toBe(true);
    t.click(); document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); expect(m.hidden).toBe(true); expect(document.activeElement).toBe(t);
  });
  it('preserves action and provider dispatch', () => {
    const t = document.getElementById('btn-side-menu');
    for (const id of ['btn-settings', 'btn-account', 'btn-feedback', 'btn-theme-side']) { const b = document.getElementById(id), f = vi.fn(); b.addEventListener('click', f); t.click(); b.click(); expect(f).toHaveBeenCalledOnce(); expect(document.getElementById('side-menu').hidden).toBe(true); }
    const r = document.getElementById('usage-rings'), f = vi.fn(); r.innerHTML = '<button data-provider="test"></button>'; r.addEventListener('click', f); r.firstChild.click(); expect(f).toHaveBeenCalledOnce();
  });
  it('keeps provider rings visible outside the closed menu and retains low state', () => { const r = document.getElementById('usage-rings'), m = document.getElementById('side-menu'); r.hidden = false; r.innerHTML = '<button data-provider="test"><svg><circle class="usage-fill low"></circle></svg></button>'; expect(m.hidden).toBe(true); expect(m.contains(r)).toBe(false); expect(r.hidden).toBe(false); expect(r.querySelector('.usage-fill.low')).not.toBeNull(); });
  it('labels the all-bots view and search separately from rooms', () => { expect(document.getElementById('btn-team').textContent.trim()).toBe('Все боты'); expect(document.querySelector('#team-head .head-name').textContent).toBe('Все боты'); expect(document.getElementById('search').placeholder).toBe('Поиск ботов и команд'); expect(html).toContain('id="room-title"'); });
});


it('opens search from its icon and Ctrl+K, preserving a nonempty filter on Escape', () => {
  document.body.innerHTML = html.match(/<body>([\s\S]*)<\/body>/)[1];
  const source = readFileSync('../web/app.js', 'utf8');
  const functions = source.slice(source.indexOf('function setSearchOpen('), source.indexOf('// New rows ('));
  const bindings = source.slice(source.indexOf("  $('#btn-search-toggle').addEventListener"), source.indexOf("  $('#search').addEventListener('input'"));
  const run = new Function('$', 'sidebarCollapsed', 'setSidebarCollapsed', 'PANE', `${functions}\n${bindings}\nreturn arcShortcuts;`);
  const shortcut = run((q) => document.querySelector(q), () => false, () => {}, false);
  const toggle = document.getElementById('btn-search-toggle'), field = document.getElementById('sidebar-search'), input = document.getElementById('search');
  toggle.click(); expect(field.hidden).toBe(false); expect(document.activeElement).toBe(input);
  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); expect(field.hidden).toBe(true);
  shortcut(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true })); expect(field.hidden).toBe(false);
  input.value = 'bot'; input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); expect(field.hidden).toBe(false);
});
