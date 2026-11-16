import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Report from './Report.svelte';
import { ruForm, toneOf, mondayOf, addDays } from '../../lib/report.js';

let host;
let instance;

async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

async function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Report, { target: host, props: { api, today: '2026-11-09' } });
  await drain();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const week = (over = {}) => ({
  weekStart: '2026-11-02',
  weekEnd: '2026-11-08',
  replies: 3,
  okCount: 2,
  failedCount: 1,
  okPercent: 67,
  weeks: [
    { start: '2026-09-28', ok: 0, failed: 0 },
    { start: '2026-10-05', ok: 0, failed: 0 },
    { start: '2026-10-12', ok: 0, failed: 0 },
    { start: '2026-10-19', ok: 0, failed: 0 },
    { start: '2026-10-26', ok: 0, failed: 0 },
    { start: '2026-11-02', ok: 2, failed: 1 },
  ],
  bots: [
    { id: 'a', name: 'Аня', replies: 3, okPercent: 67, avgTokens: 1200, avgSeconds: 4.5, failures: [{ ts: 1762600000, preview: 'таймаут модели' }] },
  ],
  ...over,
});

describe('Report', () => {
  it('shows a rough-week headline, first-week note and expandable failures', async () => {
    const api = vi.fn().mockResolvedValue(week());
    await render(api);
    expect(api).toHaveBeenCalledWith('/api/report');
    expect(host.querySelector('.rp-headline').textContent).toContain('Тяжёлая неделя: 67% из 3 ответов');
    expect(host.textContent).toContain('Первая неделя с данными');
    expect(host.querySelectorAll('.rp-col').length).toBe(6);
    expect(host.querySelector('.rp-demo')).toBeNull();
    expect(host.textContent).not.toContain('таймаут модели');
    host.querySelector('.rp-more').click();
    await drain();
    expect(host.textContent).toContain('таймаут модели');
  });

  it('navigates to the previous week and disables next on the current one', async () => {
    const api = vi.fn().mockResolvedValue(week({ weekStart: '2026-11-09', weekEnd: '2026-11-15' }));
    await render(api);
    const [prev, next] = host.querySelectorAll('.rp-step');
    expect(next.disabled).toBe(true);
    expect(host.textContent).toContain('Неделя ещё идёт');
    prev.click();
    await drain();
    expect(api).toHaveBeenLastCalledWith('/api/report?week=2026-11-02');
  });

  it('shows the empty state for a week without replies', async () => {
    await render(vi.fn().mockResolvedValue(week({ replies: 0, okCount: 0, failedCount: 0, okPercent: 0, bots: [] })));
    expect(host.textContent).toContain('На этой неделе ответов не было');
    expect(host.querySelector('.rp-chart')).toBeNull();
  });

  it('falls back to labelled demo data on 404 only', async () => {
    const err = Object.assign(new Error('nf'), { status: 404 });
    await render(vi.fn().mockRejectedValue(err));
    expect(host.querySelector('.rp-demo')).not.toBeNull();
  });

  it('shows an error with retry on other failures', async () => {
    const err = Object.assign(new Error('boom'), { status: 500 });
    await render(vi.fn().mockRejectedValue(err));
    expect(host.querySelector('.rp-error').textContent).toContain('boom');
    expect(host.querySelector('.rp-demo')).toBeNull();
  });
});

describe('report helpers', () => {
  it('picks ru plural forms', () => {
    expect([1, 2, 5, 11, 21, 22].map(ruForm)).toEqual([0, 1, 2, 2, 0, 1]);
  });
  it('tones', () => {
    expect([toneOf(0, 0), toneOf(5, 95), toneOf(5, 80), toneOf(3, 67)]).toEqual(['empty', 'great', 'good', 'rough']);
  });
  it('week math', () => {
    expect(mondayOf(new Date(2026, 10, 14))).toBe('2026-11-09');
    expect(addDays('2026-11-09', -7)).toBe('2026-11-02');
  });
});
