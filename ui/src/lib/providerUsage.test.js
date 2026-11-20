import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { icon } from './icons.js';
import { resetLine, ageLine } from './usage.js';

const source = readFileSync('../web/providers.js', 'utf8');
const helpers = source.slice(source.indexOf('  function windows('), source.indexOf('  async function renderUsage('));
const details = new Function('esc', 'tr', 'iconColor', 'limitLabel', 'DeloistUI', `${helpers}\nreturn details;`)(
  (value) => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;'),
  (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => values[key] ?? ''),
  () => 'ffffff', (label) => label, { icon, resetLine, ageLine },
);
const provider = { id: 'antigravity', name: 'Gemini', limits: [] };

describe('provider quota explanation', () => {
  it('distinguishes expired authorization from an unsupported quota source', () => {
    const expired = details({ ...provider, limitsStatus: 'auth_expired' });
    expect(expired).toContain('токен недействителен или истёк');
    expect(expired).not.toContain('Этот провайдер не сообщает лимиты');
    expect(details({ ...provider, limitsStatus: 'unsupported' })).toContain('Для входа по API-ключу');
    expect(details({ ...provider, limitsStatus: 'unavailable' })).toContain('Повторим позже');
  });

  it('shows the reset as a time left and hides a window that has already reset', () => {
    const now = Date.now();
    expect(resetLine(now + 3 * 3_600_000, now)).toBe('сброс через 3 ч');
    expect(resetLine(now + 25 * 60_000, now)).toBe('сброс через 25 мин');
    expect(resetLine(now - 1, now)).toBe('');
    expect(resetLine(null, now)).toBe('');
    expect(ageLine(now - 5 * 60_000, now)).toBe('обновлено 5 мин назад');
    const html = details({ ...provider, limits: [{ label: '5h limit', left: 42, resetsAt: now + 3 * 3_600_000 }], limitsStatus: 'ok', at: now - 5 * 60_000 });
    expect(html).toContain('сброс через 3 ч');
    expect(html).toContain('обновлено 5 мин назад');
  });

  it('keeps the last quota visible with its date and stale-data explanation', () => {
    const limits = [{ label: '5h limit', left: 42, resetsAt: 1_800_000_000_000 }];
    const stale = details({ ...provider, limits, limitsStatus: 'auth_expired', at: 1_800_000_000_000 });
    expect(stale).toContain('осталось 42 %');
    expect(stale).toContain('Данные от');
    expect(stale).toContain('токен недействителен или истёк');
    const fresh = details({ ...provider, limits, limitsStatus: 'ok' });
    expect(fresh).toContain('осталось 42 %');
    expect(fresh).not.toContain('токен недействителен');
  });
});

it('renders one sidebar ring for each Gemini account', async () => {
  const box = document.createElement('div');
  const providers = [
    { id: 'antigravity', kind: 'acp', name: 'Gemini' },
    { id: 'antigravity@a2', kind: 'acp', name: 'Gemini · Second', pickerOnly: true },
  ];
  const quotas = { providers: [provider, { id: 'antigravity@a2', name: 'Gemini · Second', limits: [] }] };
  const renderSource = source.slice(source.indexOf('  function arc('), source.indexOf('  function startUsage('));
  const renderUsage = new Function('el', 'api', 'esc', 'tr', 'iconColor', `
    ${renderSource}
    return renderUsage;
  `)(
    () => box,
    async (path) => path.endsWith('/limits') ? quotas : { providers },
    String, (text, { name }) => text.replace('{name}', name), () => 'ffffff',
  );

  await renderUsage();

  const rings = [...box.querySelectorAll('.usage-item')];
  expect(rings.map((ring) => ring.dataset.provider)).toEqual(['antigravity', 'antigravity@a2']);
  expect(rings[1].querySelector('img').getAttribute('src')).toContain('/api/providers/antigravity/icon');
});
