import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const appJs = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');

describe('saving settings never changes the main model', () => {
  it('fillSettings puts the saved driver into the select before the agent list loads', () => {
    const start = appJs.indexOf('function fillSettings()');
    const body = appJs.slice(start, appJs.indexOf('fillProviderDrivers(f);', start));
    expect(body).toContain('ensureOption(f.driver, S.settings.driver);');
    expect(body).toContain('f.acpModel.value = S.settings.model');
  });

  it('submit falls back to the saved driver and model when the driver field is empty', () => {
    const start = appJs.indexOf("$('#form-settings').addEventListener('submit'");
    const body = appJs.slice(start, start + 900);
    expect(body).toContain("f.driver.value || S.settings.driver");
    expect(body).toContain('!f.driver.value ? S.settings.model');
  });
});
