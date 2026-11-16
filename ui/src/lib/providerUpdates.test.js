import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

const source = readFileSync('../web/updates.js', 'utf8');
const provider = { id: 'gemini', name: 'Gemini', installed: '1', latest: '2', update: true, status: 'connected' };

beforeEach(() => {
  localStorage.clear();
  document.body.innerHTML = '<div id="updates"></div>';
  window.tr = (s, vars = {}) => s.replace(/\{(\w+)\}/g, (_, key) => vars[key]);
  window.DeloistProviders = { update: vi.fn() };
  window.eval(source);
});

describe('provider update notifications', () => {
  it('deduplicates versions and updates only on click', () => {
    window.DeloistUpdates.providers([provider, provider]);
    expect(document.querySelectorAll('.update-card')).toHaveLength(1);
    expect(window.DeloistProviders.update).not.toHaveBeenCalled();
    document.querySelector('.update-card .primary').click();
    expect(window.DeloistProviders.update).toHaveBeenCalledWith('gemini');
    window.DeloistUpdates.providers([provider]);
    expect(document.querySelectorAll('.update-card')).toHaveLength(0);
    window.DeloistUpdates.providers([{ ...provider, latest: '3' }]);
    expect(document.querySelectorAll('.update-card')).toHaveLength(1);
  });

  it('does not announce an installation in progress', () => {
    window.DeloistUpdates.providers([{ ...provider, status: 'installing' }]);
    expect(document.querySelectorAll('.update-card')).toHaveLength(0);
    window.DeloistUpdates.providers([provider]);
    expect(document.querySelectorAll('.update-card')).toHaveLength(1);
  });
});
