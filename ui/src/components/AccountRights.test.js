import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import AccountRights from './AccountRights.svelte';

let instance;
let host;

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(AccountRights, { target: host, props });
  flushSync();
}

async function drain() {
  for (let i = 0; i < 10; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

const button = (text) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

function siteError(message, code) {
  const error = new Error(message);
  error.status = 409;
  error.code = code;
  return error;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('AccountRights', () => {
  it('asks for a refund, exports data and links the prices page', async () => {
    let asked = false;
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/account/details') return { refundableKop: 5000, bonusKop: 10000, refundRequest: asked ? { requested_kop: 5000 } : null };
      if (path === '/api/account/refund' && opts.method === 'POST') {
        asked = true;
        return { requestedKop: 5000, days: 10 };
      }
      if (path === '/api/account/export' && opts.method === 'POST') return { path: 'C:\\Downloads\\deloist-data.json' };
      throw new Error(`unexpected ${path}`);
    });
    const onnotice = vi.fn();
    render({ api, siteUrl: 'https://deloist.ru/', onnotice });
    await drain();
    expect(host.querySelector('a').getAttribute('href')).toBe('https://deloist.ru/tseny');

    button('Вернуть деньги').click();
    await drain();
    expect(onnotice.mock.calls[0][0]).toContain('10');
    expect(button('Вернуть деньги')).toBeUndefined();
    expect(host.textContent).toContain('Заявка на возврат в работе');

    button('Скачать мои данные').click();
    await drain();
    expect(onnotice.mock.calls[1][0]).toContain('deloist-data.json');
  });

  it('deletes only after confirmation and asks to forfeit a paid balance first', async () => {
    const sent = [];
    const api = vi.fn(async (path, opts = {}) => {
      if (path === '/api/account/details') return { refundableKop: 0 };
      if (path === '/api/account/delete') {
        sent.push(opts.body);
        if (!opts.body.forfeitBalance) throw siteError('На балансе остались оплаченные 50 ₽', 'paid_balance_left');
        return { deleted: true };
      }
      throw new Error(`unexpected ${path}`);
    });
    const ondeleted = vi.fn();
    render({ api, siteUrl: 'https://deloist.ru/', ondeleted });
    await drain();
    expect(button('Удалить навсегда')).toBeUndefined();

    button('Удалить аккаунт').click();
    flushSync();
    button('Удалить навсегда').click();
    await drain();
    expect(sent).toEqual([{ confirm: 'delete', forfeitBalance: false }]);
    expect(host.textContent).toContain('оплаченные 50 ₽');
    expect(ondeleted).not.toHaveBeenCalled();
    expect(button('Удалить навсегда').disabled).toBe(true);

    host.querySelector('input[type="checkbox"]').click();
    flushSync();
    button('Удалить навсегда').click();
    await drain();
    expect(sent[1]).toEqual({ confirm: 'delete', forfeitBalance: true });
    expect(ondeleted).toHaveBeenCalledTimes(1);
  });

  it('hides the refund when the site has no account details yet', async () => {
    const api = vi.fn(async () => {
      throw new Error('Сайт пока этого не умеет');
    });
    render({ api, siteUrl: '' });
    await drain();
    expect(button('Вернуть деньги')).toBeUndefined();
    expect(host.querySelector('a')).toBeNull();
    expect(button('Скачать мои данные')).toBeDefined();
  });
});
