import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { JSDOM } from 'jsdom';
import { describe, expect, it } from 'vitest';

// Витест запускается из ui/, рядом лежит web/.
const html = readFileSync(resolve(process.cwd(), '../web/index.html'), 'utf8');
const appJs = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');
const uiIndex = readFileSync(resolve(process.cwd(), 'src/index.js'), 'utf8');
const doc = new JSDOM(html).window.document;
const form = doc.querySelector('#form-settings');
const SECTIONS = ['main', 'providers', 'screen', 'voice', 'agents', 'browser', 'updates'];

describe('settings window layout', () => {
  it('has all settings sections in nav order', () => {
    const ids = [...doc.querySelectorAll('#modal-settings [data-settings-section]')].map((section) => section.dataset.settingsSection);
    expect(ids).toEqual(SECTIONS);
  });

  it('lists the same six ids in SETTINGS_SECTIONS of app.js', () => {
    const start = appJs.indexOf('const SETTINGS_SECTIONS = [');
    const block = appJs.slice(start, appJs.indexOf('];', start));
    expect([...block.matchAll(/id: '([a-z]+)'/g)].map((match) => match[1])).toEqual(SECTIONS);
  });

  it('keeps old section ids working through the legacy map', () => {
    for (const id of ['model', 'look', 'skills', 'security', 'night', 'advanced']) {
      expect(appJs).toContain(`  ${id}: '`);
    }
  });

  it('builds every section from kit cards and rows', () => {
    for (const section of doc.querySelectorAll('#modal-settings [data-settings-section]')) {
      // Карточки голоса вставляют voice.js и stt.js в якорь.
      if (['voice', 'providers'].includes(section.dataset.settingsSection)) continue;
      expect(section.querySelector('.k-section'), section.dataset.settingsSection).not.toBeNull();
    }
    expect(doc.querySelectorAll('#modal-settings .k-row').length).toBeGreaterThan(20);
    expect(doc.querySelector('[data-settings-section="voice"] #set-voice-anchor')).not.toBeNull();
  });

  it('opens sixteen sub-screens inside the form, each with a parent section and a title', () => {
    const subs = [...doc.querySelectorAll('#form-settings .settings-sub')];
    expect(subs.map((sub) => sub.dataset.sub).sort()).toEqual(['chrome', 'cloud', 'devices', 'discord', 'dock', 'engines', 'images', 'mail', 'mcp', 'rules', 'scripts', 'skills', 'slack', 'stt-models', 'team', 'telegram', 'vault']);
    for (const sub of subs) {
      expect(sub.hidden, sub.dataset.sub).toBe(true);
      expect(SECTIONS, sub.dataset.sub).toContain(sub.dataset.parent);
      expect(sub.dataset.title, sub.dataset.sub).toBeTruthy();
    }
    expect(doc.querySelector('#settings-back')).not.toBeNull();
  });

  it('has a link row for every sub-screen opened from a section', () => {
    const links = [...doc.querySelectorAll('[data-s-link]')].map((node) => node.dataset.sLink);
    for (const id of ['dock', 'skills', 'mcp', 'images', 'team', 'telegram', 'discord', 'slack', 'mail', 'vault', 'devices', 'cloud', 'rules', 'engines', 'chrome']) expect(links).toContain(id);
  });

  it('wires the Discord sub-screen through the mount helper and the link map', () => {
    expect(appJs).toContain("if (subId === 'discord') mountDiscordSettings();");
    expect(appJs).toContain('window.DeloistUI.mountDiscordSettings');
    expect(appJs).toContain("discord: { label: tr('Discord')");
    expect(uiIndex).toContain('mountDiscordSettings: (target, props) => mountComponent(DiscordSettings, target, props)');
  });

  it('wires the Slack sub-screen through the mount helper and the link map', () => {
    expect(appJs).toContain("if (subId === 'slack') mountSlackSettings();");
    expect(appJs).toContain('window.DeloistUI.mountSlackSettings');
    expect(appJs).toContain("slack: { label: tr('Slack')");
    expect(uiIndex).toContain('mountSlackSettings: (target, props) => mountComponent(SlackSettings, target, props)');
  });

  it('wires the mail sub-screen through the mount helper and the link map', () => {
    expect(appJs).toContain("if (subId === 'mail') mountMailSettings();");
    expect(appJs).toContain('window.DeloistUI.mountMailSettings');
    expect(appJs).toContain("mail: { label: tr('Почта')");
    expect(uiIndex).toContain('mountMailSettings: (target, props) => mountComponent(MailSettings, target, props)');
  });

  it('wires the vault sub-screen through the mount helper and the link map', () => {
    expect(appJs).toContain("if (subId === 'vault') mountVaultSettings();");
    expect(appJs).toContain('window.DeloistUI.mountVaultSettings');
    expect(appJs).toContain("vault: { label: tr('Менеджер паролей')");
    expect(uiIndex).toContain('mountVaultSettings: (target, props) => mountComponent(VaultSettings, target, props)');
  });

  it('keeps the ids and field names the handlers rely on', () => {
    for (const id of ['skills-list', 'skill-name', 'skill-desc', 'skill-body', 'skill-add', 'mcp-list', 'mcp-name', 'mcp-transport', 'mcp-command', 'mcp-url', 'mcp-args', 'mcp-auto', 'mcp-add', 'team-export', 'team-import', 'team-file', 'team-note', 'rules-list', 'user-chrome-body', 'dock-enabled', 'dock-layer', 'dock-display', 'dock-edge', 'dock-autohide', 'dock-activity', 'dock-results', 'dock-bots', 'pets-mode', 'pet-picks', 'obscura-action', 'obscura-line', 'browser-engine', 'upd-check', 'upd-whatsnew', 'upd-tools', 'upd-app-line', 'upd-summary-version', 'upd-settings-foot', 'selfdev-checkout', 'selfdev-save', 'selfdev-clone', 'btn-know-me', 'settings-path', 'set-model', 'stt-models-host', 'sub-telegram', 'sub-discord', 'sub-slack', 'sub-mail', 'sub-vault', 'sub-cloud']) {
      expect(doc.getElementById(id), id).not.toBeNull();
    }
    for (const name of ['driver', 'model', 'acpModel', 'planModel', 'baseUrl', 'apiKey', 'anthropicApiKey', 'geminiApiKey', 'apiKeyEnv', 'cliCommand', 'acpCommand', 'fallbackDriver', 'fallbackModel', 'fallbackBaseUrl', 'fallbackApiKey', 'fallbackCliCommand', 'temperature', 'historyLimit', 'vision', 'browserPath', 'browserEngine', 'imageBaseUrl', 'imageModel', 'imageDaily', 'imageApiKey', 'dailyTokenLimit', 'autostart', 'keepAwake', 'checkUpdates', 'autoUpdate', 'petsMode', 'skillLearning']) {
      expect(form.elements[name], name).toBeTruthy();
    }
  });

  it('keeps key and address fields out of view, so saving never sends them', () => {
    for (const name of ['baseUrl', 'apiKey', 'anthropicApiKey', 'geminiApiKey']) {
      expect(form.elements[name].closest('[hidden]'), name).not.toBeNull();
    }
    expect(appJs).not.toContain('botDrivers');
    expect(appJs).not.toContain('data-key');
  });

  it('offers to move bots with their own model back to the main one', () => {
    expect(doc.querySelector('#set-model-card #own-models #own-models-fix')).not.toBeNull();
    expect(doc.querySelector('#own-models').hidden).toBe(true);
    for (const name of ['botsWithOwnModel', 'moveBotsToMainModel', 'offerOwnModels']) expect(appJs).toContain('function ' + name);
    expect(appJs).toContain("body: { driver: '', model: '' }");
  });

  it('turns the yes/no selects into switches', () => {
    for (const name of ['autostart', 'keepAwake', 'checkUpdates', 'autoUpdate', 'vision']) {
      expect(form.elements[name].type, name).toBe('checkbox');
    }
  });

  it('hides developer settings in a collapsed block at the bottom of the last section', () => {
    const dev = doc.querySelector('#set-dev');
    expect(dev.tagName).toBe('DETAILS');
    expect(dev.open).toBe(false);
    expect(dev.closest('[data-settings-section]').dataset.settingsSection).toBe('updates');
    expect(dev.contains(form.elements.browserPath)).toBe(true);
    expect(dev.querySelector('#set-selfdev')).not.toBeNull();
  });

  it('keeps every model setting on the Models and connections screen', () => {
    const models = doc.querySelector('[data-settings-section="providers"]');
    expect(models.querySelector('#set-model-card #set-model')).not.toBeNull();
    expect(models.querySelector('#modal-providers #prov-list')).not.toBeNull();
    for (const name of ['driver', 'model', 'acpModel', 'planModel', 'baseUrl', 'apiKey', 'anthropicApiKey', 'geminiApiKey', 'fallbackDriver', 'fallbackModel', 'fallbackBaseUrl', 'fallbackApiKey', 'fallbackCliCommand']) {
      expect(models.contains(form.elements[name]), name).toBe(true);
    }
    // Тонкие настройки модели лежат в свёрнутом блоке того же экрана, а не в «Для разработчика».
    const adv = models.querySelector('#set-model-adv');
    expect(adv.tagName).toBe('DETAILS');
    expect(adv.open).toBe(false);
    for (const name of ['apiKeyEnv', 'temperature', 'historyLimit', 'vision', 'cliCommand', 'acpCommand']) {
      expect(adv.contains(form.elements[name]), name).toBe(true);
    }
    // В «Основных» моделей не осталось.
    const main = doc.querySelector('[data-settings-section="main"]');
    expect(main.querySelector('#set-model-card')).toBeNull();
    for (const name of ['driver', 'model', 'fallbackDriver', 'temperature']) expect(main.contains(form.elements[name]), name).toBe(false);
  });

  it('sends the old model links to the Models and connections section', () => {
    expect(appJs).toContain("  model: 'providers',");
    expect(appJs).toContain("DeloistProviders.showList();");
  });

  it('has no night-shift section and only one always-open group left', () => {
    expect(doc.querySelector('[data-settings-section="night"]')).toBeNull();
    // Остался только мой Chrome: ему нужен toggle для загрузки данных. Сочетания и голос вставляют скрипты.
    expect([...doc.querySelectorAll('#modal-settings details.group')].map((group) => group.id)).toEqual(['set-user-chrome']);
  });

  it('keeps the form, the section title and the save/close controls', () => {
    expect(doc.querySelector('#form-settings')).not.toBeNull();
    expect(doc.querySelector('#settings-nav')).not.toBeNull();
    expect(doc.querySelector('#settings-title')).not.toBeNull();
    expect(doc.querySelector('#modal-settings footer .k-btn--primary[type="submit"]')).not.toBeNull();
    expect(doc.querySelector('#modal-settings footer [data-close]')).not.toBeNull();
    expect(doc.querySelector('#modal-settings .settings-close[data-close]')).not.toBeNull();
  });
});
