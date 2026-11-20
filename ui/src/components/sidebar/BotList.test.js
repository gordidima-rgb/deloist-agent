import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import BotList from './BotList.svelte';

const bot = (id, extra = {}) => ({ id, name: id, rank: 'lead', pinned: false, busy: false, unread: false, time: '', preview: '', ...extra });
const room = (id, extra = {}) => ({ id, name: id, active: false, unread: false, time: '', preview: '', archived: false, ...extra });

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  localStorage.clear();
});

function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(BotList, { target: host, props: { bots: [], rooms: [], paint: () => {}, ...props } });
  flushSync();
  return host;
}

const ids = () => [...host.querySelectorAll('.sb-row[data-id]')].map((el) => el.dataset.id);
const heads = () => [...host.querySelectorAll('.sb-head')].map((el) => el.textContent.trim());

describe('BotList', () => {
  it('shows pinned first, then leads; idle specialists wait folded in reserve', () => {
    render({ bots: [bot('lead'), bot('spec', { rank: 'specialist' }), bot('pin', { rank: 'specialist', pinned: true })] });
    expect(ids()).toEqual(['pin', 'lead']);
    expect(heads()[0]).toBe('Закреплённые');
    const toggle = host.querySelector('.sb-toggle');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.textContent).toContain('1');
    toggle.click();
    flushSync();
    expect(ids()).toEqual(['pin', 'lead', 'spec']);
    expect(localStorage.getItem('deloist:sidebar-reserve-open')).toBe('1');
  });

  it('keeps a busy, unread or open specialist among the team', () => {
    render({
      currentId: 'open',
      bots: [bot('busy', { rank: 'specialist', busy: true }), bot('new', { rank: 'specialist', unread: true }), bot('open', { rank: 'specialist' })],
    });
    expect(ids()).toEqual(['busy', 'new', 'open']);
    expect(host.querySelector('.sb-toggle')).toBeNull();
  });

  it('search shows every match in one flat list', () => {
    render({ filter: 'x', bots: [bot('spec', { rank: 'specialist' }), bot('pin', { pinned: true })] });
    expect(ids()).toEqual(['spec', 'pin']);
    expect(heads()).toEqual([]);
  });

  it('pin button reports the new state and row click selects', () => {
    const onpin = vi.fn();
    const onselect = vi.fn();
    render({ bots: [bot('a')], onpin, onselect });
    host.querySelector('.sb-pin').click();
    expect(onpin).toHaveBeenCalledWith('a', true);
    host.querySelector('.k-list-item__main').click();
    expect(onselect).toHaveBeenCalledWith('a');
  });

  it('settings button and right click open the bot menu without selecting the bot', () => {
    const onmenu = vi.fn();
    const onselect = vi.fn();
    render({ bots: [bot('a')], onmenu, onselect });
    const gear = host.querySelector('.sb-gear');
    gear.click();
    expect(onmenu).toHaveBeenLastCalledWith('a', expect.objectContaining({ trigger: gear }));
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 50 });
    host.querySelector('.k-list-item__main').dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(onmenu).toHaveBeenLastCalledWith('a', { x: 40, y: 50, trigger: null, from: '' });
    expect(onselect).not.toHaveBeenCalled();
  });

  it('shows no settings button without a menu handler', () => {
    render({ bots: [bot('a')] });
    expect(host.querySelector('.sb-gear')).toBeNull();
  });

  it('paints each bot avatar once through the callback', () => {
    const paint = vi.fn();
    render({ bots: [bot('a'), bot('b')], paint });
    expect(paint.mock.calls.map((c) => c[1])).toEqual(['a', 'b']);
  });

  it('keeps archived rooms folded at the end of the list', () => {
    render({ rooms: [room('live'), room('old', { archived: true })] });
    expect(ids()).toEqual(['live']);
    const toggle = host.querySelector('.sb-toggle');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.getAttribute('aria-label')).toBe('Архив команд · 1');
    expect(toggle.textContent).toContain('Архив команд');
    expect(toggle.textContent).toContain('1');
    toggle.click();
    flushSync();
    expect(ids()).toEqual(['live', 'old']);
    expect(localStorage.getItem('deloist:sidebar-archive-open')).toBe('1');
  });

  it('says nobody was found when the lists are empty', () => {
    render();
    expect(host.textContent).toContain('Никого не нашлось.');
  });
  it('shows members instead of "пока пусто" for a team with bots, and strips markdown from previews', () => {
    render({
      bots: [bot('lead', { name: 'Редактор', preview: 'Передал данные **@Редакт' }), bot('a', { name: 'Картинка' })],
      rooms: [room('t1', { memberIds: ['lead', 'a'], lead: 'lead' }), room('t2', { memberIds: [] }), room('t3', { memberIds: ['a'], preview: 'Картинка: **готово**' })],
    });
    const sub = (id) => host.querySelector(`li[data-team="${id}"] .k-list-item__sub`)?.textContent.trim();
    expect(sub('t1')).toBe('Редактор, Картинка');
    expect(sub('t2')).toBe('пока пусто');
    expect(sub('t3')).toBe('Картинка: готово');
    expect(host.querySelector('.sb-row[data-id="lead"]:not(.sb-team) .k-list-item__sub').textContent.trim()).toBe('Передал данные @Редакт');
  });

  describe('team tree', () => {
    const tree = (extra = {}) => ({
      bots: [bot('lead'), bot('a'), bot('b'), bot('solo'), bot('both')],
      rooms: [
        room('t1', { memberIds: ['a', 'lead', 'both'], lead: 'lead', color: '#c33' }),
        room('t2', { memberIds: ['both', 'b'], lead: '' }),
      ],
      ...extra,
    });
    const rowEls = () => [...host.querySelectorAll('.sb-tree li[role="treeitem"]')];
    const level = (el) => Number(el.getAttribute('aria-level'));

    it('puts all bots of a team one level under it; free bots go under "Без команды"', () => {
      render(tree());
      const rows = rowEls().map((el) => [el.dataset.id, level(el)]);
      expect(rows).toEqual([['t1', 1], ['lead', 2], ['a', 2], ['both', 2], ['t2', 1], ['both', 2], ['b', 2]]);
      expect(heads()).toEqual(['Команды', 'Без команды']);
      expect([...host.querySelectorAll('.sb-list:not(.sb-tree) .sb-row')].map((el) => el.dataset.id)).toEqual(['solo']);
      expect(host.querySelector('.sb-tree').getAttribute('role')).toBe('tree');
    });

    it('lists the bots of a team on one level, the lead first and crowned', () => {
      render(tree());
      const rows = [...host.querySelectorAll('li[data-team="t1"] + .sb-branch .sb-row')];
      expect(rows.map((el) => el.dataset.id)).toEqual(['lead', 'a', 'both']);
      expect(rows.map((el) => el.getAttribute('aria-level'))).toEqual(['2', '2', '2']);
      expect(rows.map((el) => !!el.querySelector('.sb-crown'))).toEqual([true, false, false]);
      expect(host.querySelector('.sb-sub')).toBeNull();
    });

    it('collapses and expands with the chevron, remembers it, and keeps the chat click', () => {
      const onselectroom = vi.fn();
      render(tree({ onselectroom }));
      const team = host.querySelector('li[data-team="t1"]');
      expect(team.getAttribute('aria-expanded')).toBe('true');
      team.querySelector('.sb-chev-btn').click();
      flushSync();
      expect(team.getAttribute('aria-expanded')).toBe('false');
      expect(host.querySelector('li[data-id="lead"]').closest('[inert]')).not.toBeNull();
      expect(JSON.parse(localStorage.getItem('deloist:sidebar-teams-closed'))).toEqual(['t1']);
      // Щелчок по имени открывает чат и раскрывает команду.
      team.querySelector('.k-list-item__main').click();
      flushSync();
      expect(onselectroom).toHaveBeenCalledWith('t1');
      expect(team.getAttribute('aria-expanded')).toBe('true');
    });

    it('restores a collapsed team from storage and survives broken storage', () => {
      localStorage.setItem('deloist:sidebar-teams-closed', JSON.stringify(['t2']));
      render(tree());
      expect(host.querySelector('li[data-team="t2"]').getAttribute('aria-expanded')).toBe('false');
      unmount(instance);
      host.remove();
      localStorage.setItem('deloist:sidebar-teams-closed', '{не json');
      render(tree());
      expect(host.querySelector('li[data-team="t2"]').getAttribute('aria-expanded')).toBe('true');
    });

    it('shows a rollup badge on a collapsed team when a member needs the owner or failed', () => {
      localStorage.setItem('deloist:sidebar-teams-closed', JSON.stringify(['t1']));
      render(tree({ bots: [bot('lead', { needs: true }), bot('a', { error: true }), bot('b'), bot('solo'), bot('both')] }));
      const team = host.querySelector('li[data-team="t1"]');
      expect(team.querySelector('.sb-roll.is-needs').textContent).toBe('1');
      expect(host.querySelector('li[data-team="t2"] .sb-roll')).toBeNull();
      team.querySelector('.sb-chev-btn').click();
      flushSync();
      expect(team.querySelector('.sb-roll')).toBeNull();
    });

    it('search opens collapsed teams that have matches and hides the plain list duplicates', () => {
      localStorage.setItem('deloist:sidebar-teams-closed', JSON.stringify(['t1']));
      render(tree({ filter: 'a', bots: [bot('a')], rooms: [room('t1', { memberIds: ['a'], lead: 'a' })] }));
      expect(host.querySelector('li[data-team="t1"]').getAttribute('aria-expanded')).toBe('true');
      expect(host.querySelector('li[data-id="a"]').closest('[inert]')).toBeNull();
      expect(host.querySelectorAll('.sb-row[data-id="a"]').length).toBe(1);
    });

    it('arrow keys walk the tree: right opens, down enters, left goes back and closes', () => {
      render(tree());
      const mainOf = (id, n = 0) => host.querySelectorAll('li[data-id="' + id + '"] .k-list-item__main')[n];
      const press = (el, key) => {
        el.focus();
        el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
        flushSync();
      };
      const team = host.querySelector('li[data-team="t1"]');
      press(mainOf('t1'), 'ArrowLeft');
      expect(team.getAttribute('aria-expanded')).toBe('false');
      press(mainOf('t1'), 'ArrowRight');
      expect(team.getAttribute('aria-expanded')).toBe('true');
      press(mainOf('t1'), 'ArrowRight');
      expect(document.activeElement).toBe(mainOf('lead'));
      press(mainOf('lead'), 'ArrowDown');
      expect(document.activeElement).toBe(mainOf('a'));
      press(mainOf('a'), 'ArrowLeft');
      expect(document.activeElement).toBe(mainOf('t1'));
    });
  });

  it('reorders bots via drag-and-drop and Alt+Arrow and shows avatar status dot', () => {
    const onreorder = vi.fn();
    render({ bots: [bot('a', { busy: true }), bot('b', { error: true })], onreorder });
    const rows = [...host.querySelectorAll('.sb-row[data-id]')];
    expect(rows[0].querySelector('.sb-state-dot')?.dataset.state).toBe('busy');
    expect(rows[1].querySelector('.sb-state-dot')?.dataset.state).toBe('error');

    const start = new Event('dragstart', { bubbles: true, cancelable: true });
    start.dataTransfer = { setData: vi.fn(), effectAllowed: '' };
    rows[0].dispatchEvent(start);
    const over = new Event('dragover', { bubbles: true, cancelable: true });
    over.dataTransfer = { dropEffect: '' };
    rows[1].dispatchEvent(over);
    flushSync();
    expect(rows[1].classList.contains('is-drag-over')).toBe(true);
    const drop = new Event('drop', { bubbles: true, cancelable: true });
    drop.dataTransfer = { dropEffect: '' };
    rows[1].dispatchEvent(drop);
    flushSync();
    expect(onreorder).toHaveBeenCalledWith('main', ['b', 'a']);

    const mainBtn = rows[0].querySelector('.k-list-item__main');
    mainBtn.focus();
    mainBtn.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', altKey: true, bubbles: true, cancelable: true }));
    flushSync();
    expect(onreorder).toHaveBeenLastCalledWith('main', ['b', 'a']);
  });
});
