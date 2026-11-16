import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

const source = readFileSync('../web/app.js', 'utf8');
const helper = source.slice(source.indexOf('async function providersChanged('), source.indexOf('function providerModelOptions('));

describe('provider defaults inside settings', () => {
  it('synchronizes the model form after the provider changes the saved default', async () => {
    const state = { settings: { driver: 'old', model: 'old-model' } };
    const apply = vi.fn(), update = vi.fn(), reload = vi.fn();
    const refresh = async () => { state.settings = { driver: 'new', model: 'new-model' }; };
    const changed = new Function('S', '$', 'refreshState', 'applySettingsModel', 'settingsPicker', 'reloadModelPickers', `${helper}\nreturn providersChanged;`)(
      state, () => ({ open: true }), refresh, apply, { picker: { update } }, reload,
    );
    await changed({ defaultChanged: true });
    expect(apply).toHaveBeenCalledWith({ driver: 'new', model: 'new-model' });
    expect(update).toHaveBeenCalledWith({ value: { driver: 'new', model: 'new-model' }, settings: state.settings });
    expect(reload).toHaveBeenCalledOnce();
    apply.mockClear();
    await changed();
    expect(apply).not.toHaveBeenCalled();
  });
});
