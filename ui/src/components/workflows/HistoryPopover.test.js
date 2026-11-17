import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import HistoryPopover from './HistoryPopover.svelte';

let host;
let instance;
async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}
async function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(HistoryPopover, { target: host, props });
  await drain();
}
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const run = (id, status, source) => ({ id, status, source, started_at: 1700000000000, ended_at: 1700000012000, steps: [] });

describe('HistoryPopover', () => {
  it('lists runs with status, source and duration, and shows one on click', async () => {
    const api = vi.fn(async () => ({ runs: [run('r1', 'done', 'manual'), run('r2', 'failed', 'schedule')], next_before: null }));
    const onview = vi.fn();
    await render({ api, workflowId: 'w1', mode: 'runs', onview });
    expect(api.mock.calls[0][0]).toBe('/api/workflows/w1/runs?limit=20');
    const rows = host.querySelectorAll('.wf-hp__row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('Готово');
    expect(rows[0].textContent).toContain('Вручную');
    expect(rows[0].textContent).toContain('12 с');
    expect(rows[1].textContent).toContain('По расписанию');
    rows[1].querySelector('.k-lrow__t').click();
    expect(onview).toHaveBeenCalledWith(expect.objectContaining({ id: 'r2' }));
  });

  it('loads the next page with `before`', async () => {
    const api = vi.fn(async (path) => (path.includes('before=') ? { runs: [run('r3', 'done', 'manual')], next_before: null } : { runs: [run('r1', 'done', 'manual')], next_before: 1699999999000 }));
    await render({ api, workflowId: 'w1', mode: 'runs' });
    host.querySelector('.wf-hp__more').click();
    await drain();
    expect(api.mock.calls[1][0]).toContain('before=1699999999000');
    expect(host.querySelectorAll('.wf-hp__row')).toHaveLength(2);
  });

  it('offers a way back while an old run is shown', async () => {
    const onview = vi.fn();
    await render({ api: async () => ({ runs: [], next_before: null }), workflowId: 'w1', mode: 'runs', viewedId: 'r1', onview });
    host.querySelector('.wf-hp__back').click();
    expect(onview).toHaveBeenCalledWith(null);
  });

  it('lists versions newest first and opens one for preview', async () => {
    const api = vi.fn(async () => [
      { v: 1, kind: 'auto', at: 1700000000000, revision: 1, steps: 3 },
      { v: 2, kind: 'run', at: 1700000100000, revision: 2, steps: 4 },
    ]);
    const onpreview = vi.fn();
    await render({ api, workflowId: 'w1', mode: 'versions', onpreview });
    expect(api.mock.calls[0][0]).toBe('/api/workflows/w1/versions');
    const rows = host.querySelectorAll('.wf-hp__row');
    expect(rows[0].textContent).toContain('Перед запуском');
    expect(rows[1].textContent).toContain('Автосохранение');
    rows[0].querySelector('.k-lrow__t').click();
    expect(onpreview).toHaveBeenCalledWith(expect.objectContaining({ v: 2 }));
  });

  it('shows an error with retry when the request fails', async () => {
    await render({ api: async () => { throw new Error('нет связи'); }, workflowId: 'w1', mode: 'versions' });
    expect(host.querySelector('.is-err').textContent).toContain('нет связи');
  });
});
