import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import TeamScreen from './TeamScreen.svelte';

let instance;
let host;
function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(TeamScreen, { target: host, props });
  flushSync();
}
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const room = { id: 'room_1', name: 'Запуск', lead: 'b2', mode: 'lead', instructions: 'Говорим коротко', pictureUrl: '' };
const members = [
  { id: 'b1', name: 'Рен', role: 'Редактор', busy: false, needs: false, error: false },
  { id: 'b2', name: 'Ива', role: 'Ведущая', busy: true, needs: false, error: false },
  { id: 'b3', name: 'Ким', role: 'Аналитик', busy: false, needs: true, error: false },
];

describe('TeamScreen', () => {
  it('draws you, the lead and the other members with live status', () => {
    render({ room, members, tab: 'structure' });
    expect(host.querySelector('.ts-you .ts-title').textContent).toBe('Ты');
    const lead = host.querySelector('.ts-bot.is-lead');
    expect(lead.dataset.bot).toBe('b2');
    expect(lead.querySelector('.ts-badge').textContent).toBe('Ведущий');
    expect(lead.querySelector('.ts-state').textContent).toContain('Работает');
    const kids = [...host.querySelectorAll('.ts-kid .ts-bot')].map((el) => el.dataset.bot);
    expect(kids).toEqual(['b1', 'b3']);
    expect(host.querySelector('.ts-bot[data-bot="b3"] .ts-state').textContent).toContain('Нужен ты');
    expect(host.querySelector('.ts-bot[data-bot="b1"] .ts-state').textContent).toContain('Свободен');
  });

  it('falls back to the first member when the lead is not in the team', () => {
    render({ room: { ...room, lead: 'gone' }, members, tab: 'structure' });
    expect(host.querySelector('.ts-bot.is-lead').dataset.bot).toBe('b1');
  });

  it('opens a bot, the tabs and the team editor from their own controls', () => {
    const onopenbot = vi.fn();
    const ontab = vi.fn();
    const onedit = vi.fn();
    render({ room, members, tab: 'structure', onopenbot, ontab, onedit });
    host.querySelector('.ts-bot[data-bot="b3"]').click();
    expect(onopenbot).toHaveBeenCalledWith('b3');
    host.querySelector('.k-tab[data-id="chat"]').click();
    expect(ontab).toHaveBeenCalledWith('chat');
    [...host.querySelectorAll('.ts-about button')].find((b) => b.textContent.includes('Изменить команду')).click();
    expect(onedit).toHaveBeenCalled();
  });

  it('shows the mode and the rules, and the picture in the bar', () => {
    render({ room: { ...room, mode: 'auto', pictureUrl: '/api/rooms/room_1/picture?v=1' }, members, tab: 'structure' });
    expect(host.querySelector('.ts-about').textContent).toContain('Модель выбирает, кто ответит');
    expect(host.querySelector('.ts-about').textContent).toContain('Говорим коротко');
    expect(host.querySelector('.ts-pic img').getAttribute('src')).toBe('/api/rooms/room_1/picture?v=1');
  });

  it('keeps only the tab bar on the chat tab', () => {
    render({ room, members, tab: 'chat' });
    expect(host.querySelector('.ts-bar')).not.toBeNull();
    expect(host.querySelector('.ts-body')).toBeNull();
  });
});
