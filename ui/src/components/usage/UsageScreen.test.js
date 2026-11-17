import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import UsageScreen from './UsageScreen.svelte';
import { usageFixture } from '../../lib/usage.fixture.js';
import { paceForecast, shiftMonth, fmtTokens } from '../../lib/usage.js';

let host;
let instance;
async function drain() {
  for (let i = 0; i < 15; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}
async function render(api, props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(UsageScreen, { target: host, props: { api, today: new Date(2026, 10, 14), ...props } });
  await drain();
}
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const notFound = () => Object.assign(new Error('nf'), { status: 404 });

describe('UsageScreen', () => {
  it('shows fixture with a visible demo notice when /api/usage is 404', async () => {
    await render(async (path) => {
      if (path.startsWith('/api/usage')) throw notFound();
      return { providers: [] };
    });
    expect(host.textContent).toContain('цифры ниже не настоящие');
    expect(host.textContent).toContain('По агентам');
    expect(host.textContent).toContain('Аналитик');
  });

  it('draws real data without the notice, limit pace and plain number', async () => {
    const data = usageFixture('2026-11');
    await render(async (path) => (path.startsWith('/api/usage') ? data : { providers: [] }));
    expect(host.textContent).not.toContain('не настоящие');
    expect(host.textContent).toContain('При таком темпе к концу месяца');
    const writer = [...host.querySelectorAll('.us-row')].find((r) => r.textContent.includes('Редактор'));
    expect(writer.querySelector('.us-bar')).toBeNull();
  });

  it('shows the empty month state', async () => {
    const data = { ...usageFixture('2026-11'), totalTokens: 0, replies: 0, days: [], bots: [], models: [], receipts: [] };
    await render(async (path) => (path.startsWith('/api/usage') ? data : { providers: [] }));
    expect(host.textContent).toContain('В этом месяце расхода нет');
  });

  it('shows an error with retry on a server failure', async () => {
    await render(async (path) => {
      if (path.startsWith('/api/usage')) throw Object.assign(new Error('x'), { status: 500 });
      return { providers: [] };
    });
    expect(host.querySelector('[role=alert]')).not.toBeNull();
  });

  it('switches month and blocks going into the future', async () => {
    const asked = [];
    await render(async (path) => {
      asked.push(path);
      return path.startsWith('/api/usage') ? usageFixture(path.split('=')[1]) : { providers: [] };
    });
    const [prev, next] = host.querySelectorAll('.us-nav');
    expect(next.disabled).toBe(true);
    prev.click();
    await drain();
    expect(asked).toContain('/api/usage?month=2026-10');
    expect(host.querySelectorAll('.us-nav')[1].disabled).toBe(false);
  });

  it('opens the conversation from a receipt', async () => {
    let opened = null;
    await render(async (path) => (path.startsWith('/api/usage') ? usageFixture('2026-11') : { providers: [] }), { onOpen: (r) => (opened = r) });
    host.querySelector('.us-link').click();
    expect(opened.threadId).toBe('analyst');
  });

  it('lists subscription limits of connected agents with the time left to the reset', async () => {
    await render(async (path) => {
      if (path.startsWith('/api/usage')) return usageFixture('2026-11');
      if (path === '/api/providers/limits') {
        return { providers: [{ id: 'codex', name: 'ChatGPT', limits: [{ label: 'Weekly limit', left: 40, resetsAt: Date.now() + 3 * 3_600_000 + 60_000 }] }] };
      }
      return { providers: [
        { id: 'codex', kind: 'acp', name: 'ChatGPT', status: 'connected' },
        { id: 'deepseek', kind: 'key', name: 'DeepSeek API', status: 'connected' },
      ] };
    });
    expect(host.textContent).toContain('ChatGPT');
    expect(host.textContent).toContain('осталось 40%');
    expect(host.textContent).toContain('сброс через 3 ч');
    expect(host.textContent).not.toContain('DeepSeek API');
  });
});

describe('usage helpers', () => {
  it('forecasts only the current month', () => {
    const today = new Date(2026, 10, 15);
    expect(paceForecast({ tokens: 500, limit: 1000, month: '2026-11', today })).toBeCloseTo(100);
    expect(paceForecast({ tokens: 500, limit: 1000, month: '2026-10', today })).toBeNull();
    expect(paceForecast({ tokens: 500, limit: null, month: '2026-11', today })).toBeNull();
  });
  it('shifts months across years and formats tokens', () => {
    expect(shiftMonth('2026-01', -1)).toBe('2025-12');
    expect(shiftMonth('2026-12', 1)).toBe('2027-01');
    expect(fmtTokens(1500000, 'en-US')).toBe('1.5M');
  });
});
