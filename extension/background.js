// Deloist в Chrome человека: отладчик подключается только к вкладкам ботов.
//
// Расширение — проводник. Решения (какие сайты, какие методы CDP, кто может работать без
// человека) принимает приложение Deloist. Здесь только то, что нельзя доверить каналу:
// бот видит лишь вкладки своей группы, открывает лишь http(s), а «Остановить всё»
// отключает отладчик сразу, не дожидаясь приложения.

import { denyPatterns as _dp, denyLoadFailed as _dlf, loadDeny as _loadDeny, isDeniedUrl as _isDenied, hostMatchesPattern as _hostMatch } from './deny.js';
import { Conn } from './protocol.js';
const HOST = 'com.deloist.bots';
const RETRY_MS = 5000;
const RETRY_MAX_MS = 60000;
const SECRET = /^(Storage|DOMStorage|IndexedDB|CacheStorage|Autofill)\.|^Network\.(getCookies|getAllCookies|setCookies?|deleteCookies|clearBrowserCookies|getRequestPostData)$|^Page\.(getCookies|addScriptToEvaluateOnNewDocument)$/;
const COLORS = {
  grey: [122, 133, 149], blue: [66, 133, 244], red: [234, 67, 53], yellow: [251, 188, 4], green: [52, 168, 83],
  pink: [255, 99, 184], purple: [161, 66, 244], cyan: [36, 193, 224], orange: [250, 144, 62],
};

let port = null;
let retry = 0;
let retryDelay = RETRY_MS; // растёт вдвое после каждого протокольного обрыва, сбрасывается на ready
let pairingKeyHex = null;
// deny state lives in deny.js
let allowedOriginsCache = new Set();
const secretUnlock = new Map(); // tabId -> expiry ms
let conn = null; // состояние текущего подключения (protocol.js); null, пока связи нет
let connecting = false;
// Что ждёт решения человека: ключ — что именно, значение — до какого времени показывать «!».
const waiting = new Map();
const WAITING_MS = 120000;

async function loadPairingKey() {
  const { pairingKey } = await chrome.storage.local.get('pairingKey');
  pairingKeyHex = pairingKey || null;
}
async function loadAllowedOrigins() {
  const { allowedOrigins = [] } = await chrome.storage.local.get('allowedOrigins');
  allowedOriginsCache = new Set(allowedOrigins);
}
async function loadDeny() {
  const url = chrome.runtime.getURL('deny.json');
  await _loadDeny(fetch, url);
}
function hostMatchesPattern(host, pattern) { return _hostMatch(host, pattern); }
function isDeniedUrl(url) { return _isDenied(url); }
async function isOriginAllowed(url) {
  await loadAllowedOrigins();
  let origin;
  try { origin = new URL(url).origin; } catch { return false; }
  return allowedOriginsCache.has(origin);
}
function originOfUrl(url) {
  try { return new URL(url).origin; } catch { return null; }
}
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local') {
    if (changes.pairingKey) {
      pairingKeyHex = changes.pairingKey.newValue || null;
      // Другой код связи: начинаем подключение заново, с новым ключом.
      if (port) {
        closeLink(t('errDisconnected'));
        connect();
      }
    }
    if (changes.allowedOrigins) {
      allowedOriginsCache = new Set(changes.allowedOrigins.newValue || []);
      for (const key of [...waiting.keys()]) if (key.startsWith('site:')) waiting.delete(key);
      badge();
    }
  }
});
const link = { connected: false, paired: false, error: '', since: 0 };
let known = { bots: [] }; // что приложение рассказало о ботах и сайтах: для окна расширения
/** botId → { name, color, groupId, windowId, tabs: Map<tabId, { targetId, sessionId, url, title }> } */
const bots = new Map();

// Реестр ботов и вкладок переживает перезапуск service worker (MV3 усыпляет его): иначе
// после пробуждения webNavigation не знал бы, какие вкладки бота надо сторожить.
let registryLoaded = false;
function persist() {
  if (!registryLoaded) return; // пока не прочитали сохранённое, пустой реестр не пишем поверх него
  try {
    const data = [...bots].map(([id, bot]) => [id, { name: bot.name, color: bot.color, groupId: bot.groupId, windowId: bot.windowId, tabs: [...bot.tabs] }]);
    chrome.storage.session.set({ registry: data }).catch(() => {});
  } catch {
    // нет storage.session: работаем без сохранения
  }
}
async function restoreRegistry() {
  try {
    const { registry } = await chrome.storage.session.get('registry');
    if (!Array.isArray(registry)) return;
    for (const [id, bot] of registry) {
      if (bots.has(id)) continue;
      bots.set(id, { name: bot.name, color: bot.color, groupId: bot.groupId, windowId: bot.windowId, tabs: new Map(bot.tabs || []) });
    }
  } catch {
    // реестра нет: сторожим по группам вкладок (см. guardedTab)
  } finally {
    registryLoaded = true;
  }
}
const restored = restoreRegistry();

/**
 * Вкладка под охраной: известная вкладка бота или любая вкладка в группе «Deloist · …»,
 * даже если реестр её не знает (потерян или не успел записаться). Правила те же, что для ботов.
 */
async function guardedTab(tabId) {
  await restored;
  if (findTab(tabId)) return true;
  try {
    const tab = await chrome.tabs.get(tabId);
    if (!tab || tab.groupId === undefined || tab.groupId < 0) return false;
    const group = await chrome.tabGroups.get(tab.groupId);
    return typeof group.title === 'string' && group.title.startsWith(t('groupTitle', ['']));
  } catch {
    return false;
  }
}

/** Ждёт решения человека (карточка в окне расширения): значок «!» на время. */
function wait(key) {
  waiting.set(key, Date.now() + WAITING_MS);
  badge();
  setTimeout(badge, WAITING_MS + 100);
}

const t = (key, subs) => chrome.i18n.getMessage(key, subs) || key;

// ---------------------------------------------------------------------------
// Связь с приложением
// ---------------------------------------------------------------------------

async function connect() {
  if (port || connecting) return;
  connecting = true;
  try {
    await Promise.all([loadPairingKey(), loadAllowedOrigins(), loadDeny(), restored]);
  } finally {
    connecting = false;
  }
  if (port) return;
  clearTimeout(retry);
  const fresh = new Conn({ k: pairingKeyHex });
  let opened;
  try {
    opened = chrome.runtime.connectNative(HOST);
  } catch (e) {
    link.error = String(e && e.message || e);
    linkDown(link.error);
    return;
  }
  port = opened;
  conn = fresh;
  link.paired = !!pairingKeyHex;
  opened.onMessage.addListener((raw) => { if (port === opened) handleIncoming(fresh, raw); });
  opened.onDisconnect.addListener(() => {
    // Своё закрытие (closeLink) уже убрало port: второй раз не чистим.
    if (port !== opened) return;
    const error = chrome.runtime.lastError;
    port = null;
    conn = null;
    linkDown(error ? error.message : '');
  });
  // hello уходит без подписи; без пары следом только status
  postRaw(fresh.helloMessage({ version: chrome.runtime.getManifest().version, browser: browserName() }));
  if (!pairingKeyHex) postRaw({ type: 'status', paired: false });
  badge();
}

/** Связь пропала (сама или закрыта нами): сбросить всё и назначить переподключение. */
function linkDown(error) {
  if (link.connected || !link.error) link.error = error || t('errDisconnected');
  link.connected = false;
  detachAll('disconnected');
  badge();
  schedule();
}

/** Закрыть подключение самим. Chrome не зовёт onDisconnect порта при нашем disconnect(), поэтому чистим здесь. */
function closeLink(error) {
  // Протокольный обрыв: следующая попытка позже (5 с, 10 с … до 60 с), чтобы не молотить приложение.
  retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
  const closing = port;
  port = null;
  conn = null;
  if (closing) {
    try { closing.disconnect(); } catch {}
  }
  linkDown(error || t('errProtocol'));
}

function schedule() {
  clearTimeout(retry);
  retry = setTimeout(connect, retryDelay);
}

function postRaw(obj) {
  if (!port) return;
  try { port.postMessage(obj); } catch {}
}

/** Подписанный кадр: номер, подпись и отправка в одной цепочке (порядок n = порядок в канале). */
function send(message) {
  const c = conn;
  if (!c || !port) return;
  c.seal(message, (frame) => { if (conn === c) postRaw(frame); });
}

async function handleIncoming(c, raw) {
  const result = await c.onMessage(raw); // проверка по одному, в порядке прихода
  if (c !== conn) return; // за это время подключение закрыто или заменено
  switch (result.action) {
    case 'close':
      closeLink();
      break;
    case 'session_ready':
      send({ type: 'ready' });
      break;
    case 'deliver':
      if (result.msg && result.msg.type === 'ready') {
        retryDelay = RETRY_MS;
        link.connected = true;
        link.error = ''
        link.since = Date.now();
        badge();
      } else {
        onMessage(result.msg);
      }
      break;
    default:
      break;
  }
}

function browserName() {
  const brands = (navigator.userAgentData && navigator.userAgentData.brands) || [];
  if (brands.some((b) => /Edge/i.test(b.brand))) return 'edge';
  if (brands.some((b) => /Chrome/i.test(b.brand))) return 'chrome';
  return 'chromium';
}

function onMessage(message) {
  switch (message && message.type) {
    case 'state':
      known = { bots: Array.isArray(message.bots) ? message.bots : [] };
      break;
    case 'cdp':
      command(message).catch((e) => reply(message, null, String(e && e.message || e)));
      break;
    case 'detach':
      detachBot(message.bot, 'app');
      break;
    case 'stop_all':
      detachAll('app');
      break;
    default:
      break;
  }
}

function reply(message, result, error) {
  const msg = { id: message.msg.id };
  if (error) msg.error = { message: error };
  else msg.result = result || {};
  send({ type: 'cdp', bot: message.bot, msg });
}

function event(botId, method, params, sessionId) {
  const msg = { method, params };
  if (sessionId) msg.sessionId = sessionId;
  send({ type: 'cdp', bot: botId, msg });
}

// ---------------------------------------------------------------------------
// Вкладки ботов
// ---------------------------------------------------------------------------

function botOf(botId, name, color) {
  let bot = bots.get(botId);
  if (!bot) {
    bot = { name: name || 'бот', color: color || '', groupId: -1, windowId: -1, tabs: new Map() };
    bots.set(botId, bot);
  }
  if (name) bot.name = name;
  if (color) bot.color = color;
  return bot;
}

function findTab(tabId) {
  for (const [botId, bot] of bots) {
    const tab = bot.tabs.get(tabId);
    if (tab) return { botId, bot, tab };
  }
  return null;
}

function bySession(bot, sessionId) {
  for (const [tabId, tab] of bot.tabs) if (tab.sessionId === sessionId) return [tabId, tab];
  return [null, null];
}

function nearestColor(hex) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex || '');
  if (!m) return 'grey';
  const rgb = [1, 2, 3].map((i) => parseInt(m[i], 16));
  let best = 'grey';
  let bestD = Infinity;
  for (const [name, c] of Object.entries(COLORS)) {
    const d = c.reduce((sum, v, i) => sum + (v - rgb[i]) ** 2, 0);
    if (d < bestD) [best, bestD] = [name, d];
  }
  return best;
}

function webAddress(url) {
  return url === 'about:blank' || /^https?:\/\//i.test(url || '');
}

async function targetIdOf(tabId) {
  const targets = await chrome.debugger.getTargets();
  const found = targets.find((x) => x.tabId === tabId && x.type === 'page');
  return found ? found.id : `tab-${tabId}`;
}

async function windowFor(bot) {
  if (bot.windowId >= 0) {
    try {
      await chrome.windows.get(bot.windowId);
      return bot.windowId;
    } catch {
      bot.windowId = -1;
    }
  }
  try {
    const win = await chrome.windows.getLastFocused({ windowTypes: ['normal'] });
    if (win && win.id >= 0) return win.id;
  } catch {
    // окон нет: создадим своё
  }
  const created = await chrome.windows.create({ focused: false, url: 'about:blank' });
  return created.id;
}

async function putInGroup(bot, tabId) {
  try {
    if (bot.groupId >= 0) {
      try {
        await chrome.tabs.group({ tabIds: [tabId], groupId: bot.groupId });
        return;
      } catch {
        bot.groupId = -1; // группу закрыли: заведём новую
      }
    }
    bot.groupId = await chrome.tabs.group({ tabIds: [tabId] });
    await chrome.tabGroups.update(bot.groupId, { title: t('groupTitle', [bot.name]), color: nearestColor(bot.color), collapsed: false });
  } catch {
    // в браузере без групп вкладок (или в окне-приложении) работаем без группы
  }
}

/** Вкладка становится вкладкой бота: отладчик, группа, событие для приложения. */
async function adopt(botId, bot, tabId, url) {
  await chrome.debugger.attach({ tabId }, '1.3');
  const targetId = await targetIdOf(tabId);
  const sessionId = `deloist-${tabId}`;
  const info = { targetId, sessionId, url: url || 'about:blank', title: '' };
  bot.tabs.set(tabId, info);
  await putInGroup(bot, tabId);
  event(botId, 'Target.attachedToTarget', {
    sessionId,
    targetInfo: { targetId, type: 'page', url: info.url, title: '', attached: true },
    waitingForDebugger: false,
  });
  persist();
  badge();
  return info;
}

async function release(botId, tabId, why) {
  const found = bots.get(botId);
  const tab = found && found.tabs.get(tabId);
  if (!tab) return;
  found.tabs.delete(tabId);
  try {
    await chrome.debugger.detach({ tabId });
  } catch {
    // уже отключён
  }
  event(botId, 'Target.detachedFromTarget', { sessionId: tab.sessionId, targetId: tab.targetId, reason: why });
  persist();
  badge();
}

async function detachBot(botId, why) {
  const bot = bots.get(botId);
  if (!bot) return;
  await Promise.all([...bot.tabs.keys()].map((tabId) => release(botId, tabId, why)));
}

async function detachAll(why) {
  await Promise.all([...bots.keys()].map((botId) => detachBot(botId, why)));
}

// ---------------------------------------------------------------------------
// Команды CDP
// ---------------------------------------------------------------------------

async function command(message) {
  const { bot: botId, msg } = message;
  if (!botId || !msg || typeof msg.method !== 'string') throw new Error('bad message');
  const bot = botOf(botId, message.name, message.color);
  const params = msg.params || {};
  const method = msg.method;

  if (method.startsWith('Fetch.')) throw new Error(t('errDenied'));

  if (!msg.sessionId) {
    if (SECRET.test(method)) throw new Error(t('errSecret'));
    switch (method) {
      case 'Target.setDiscoverTargets':
      case 'Target.setAutoAttach':
      case 'Browser.setDownloadBehavior':
        return reply(message, {});
      case 'Target.createTarget': {
        const url = params.url || 'about:blank';
        if (!webAddress(url)) throw new Error(t('errScheme'));
        if (url !== 'about:blank') {
          if (isDeniedUrl(url)) throw new Error(t('errDenied'));
          if (!(await isOriginAllowed(url))) {
            wait(`site:${originOfUrl(url)}`);
            throw new Error(t('errNotAllowed'));
          }
        }
        const windowId = await windowFor(bot);
        bot.windowId = windowId;
        const tab = await chrome.tabs.create({ url: 'about:blank', active: params.background === false, windowId });
        const info = await adopt(botId, bot, tab.id, 'about:blank');
        if (url !== 'about:blank') await chrome.debugger.sendCommand({ tabId: tab.id }, 'Page.navigate', { url });
        return reply(message, { targetId: info.targetId });
      }
      case 'Target.closeTarget': {
        for (const [tabId, tab] of bot.tabs) {
          if (tab.targetId === params.targetId) {
            await release(botId, tabId, 'closed');
            await chrome.tabs.remove(tabId);
            event(botId, 'Target.targetDestroyed', { targetId: tab.targetId });
            return reply(message, { success: true });
          }
        }
        throw new Error(t('errNoTab'));
      }
      case 'Target.detachFromTarget': {
        const [tabId] = bySession(bot, params.sessionId);
        if (tabId !== null) await release(botId, tabId, 'app');
        return reply(message, {});
      }
      case 'Browser.close':
        // Браузер человека не закрываем: только отпускаем вкладки бота.
        await detachBot(botId, 'app');
        return reply(message, {});
      default:
        throw new Error(t('errMethod', [method]));
    }
  }

  const [tabId, tab] = bySession(bot, msg.sessionId);
  if (tabId === null) throw new Error(t('errNoTab'));
  if (SECRET.test(method)) {
    // Разрешение одноразовое: первый же вызов его расходует, дальше нужна новая отмашка.
    const ok = secretUnlock.has(tabId) && secretUnlock.get(tabId) > Date.now();
    secretUnlock.delete(tabId);
    if (!ok) {
      wait(`secret:${tabId}`);
      throw new Error(t('errSecret'));
    }
  }
  switch (method) {
    case 'Browser.getWindowForTarget': {
      const info = await chrome.tabs.get(tabId);
      return reply(message, { windowId: info.windowId, bounds: {} });
    }
    case 'Browser.setWindowBounds':
      return reply(message, {}); // окно человека не двигаем
    case 'Page.bringToFront': {
      const info = await chrome.tabs.update(tabId, { active: true });
      await chrome.windows.update(info.windowId, { focused: true });
      return reply(message, {});
    }
    case 'Page.navigate':
      if (!webAddress(params.url)) throw new Error(t('errScheme'));
      if (isDeniedUrl(params.url)) throw new Error(t('errDenied'));
      if (!(await isOriginAllowed(params.url))) {
        wait(`site:${originOfUrl(params.url)}`);
        throw new Error(t('errNotAllowed'));
      }
      break;
    default:
      break;
  }
  const result = await chrome.debugger.sendCommand({ tabId }, method, params);
  void tab;
  return reply(message, result);
}

// ---------------------------------------------------------------------------
// События браузера
// ---------------------------------------------------------------------------

async function refreshInfo(botId, tabId) {
  const found = findTab(tabId);
  if (!found) return;
  try {
    const targets = await chrome.debugger.getTargets();
    const target = targets.find((x) => x.tabId === tabId && x.type === 'page');
    if (target) {
      found.tab.url = target.url;
      found.tab.title = target.title;
    }
  } catch {
    // вкладка уже закрыта
  }
  event(botId, 'Target.targetInfoChanged', {
    targetInfo: { targetId: found.tab.targetId, type: 'page', url: found.tab.url, title: found.tab.title, attached: true },
  });
}

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (source.sessionId) return; // дочерние сессии (фреймы) приложению не нужны
  const found = findTab(source.tabId);
  if (!found) return;
  if (method === 'Page.frameNavigated' && params && params.frame && !params.frame.parentId) {
    // Адрес — сразу: по нему приложение решает, можно ли боту читать эту вкладку.
    found.tab.url = params.frame.url;
    event(found.botId, 'Target.targetInfoChanged', {
      targetInfo: { targetId: found.tab.targetId, type: 'page', url: found.tab.url, title: found.tab.title, attached: true },
    });
  }
  event(found.botId, method, params, found.tab.sessionId);
  if (method === 'Page.loadEventFired' || method === 'Page.domContentEventFired' || method === 'Page.navigatedWithinDocument') {
    refreshInfo(found.botId, source.tabId);
  }
});

chrome.debugger.onDetach.addListener((source, reason) => {
  const found = findTab(source.tabId);
  if (!found) return;
  found.bot.tabs.delete(source.tabId);
  event(found.botId, 'Target.detachedFromTarget', { sessionId: found.tab.sessionId, targetId: found.tab.targetId, reason });
  badge();
  if (reason === 'canceled_by_user') {
    // Человек нажал «Отмена» на полосе отладки: это то же, что «Остановить всё».
    stopAll('canceled_by_user');
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  const found = findTab(tabId);
  if (!found) return;
  found.bot.tabs.delete(tabId);
  event(found.botId, 'Target.targetDestroyed', { targetId: found.tab.targetId });
  badge();
});

chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (change.groupId === undefined) return;
  const found = findTab(tabId);
  // Человек вынес вкладку из группы бота: она снова его, бот её больше не трогает.
  if (found && found.bot.groupId >= 0 && change.groupId !== found.bot.groupId) {
    release(found.botId, tabId, 'ungrouped');
    event(found.botId, 'Target.targetDestroyed', { targetId: found.tab.targetId });
  }
});

async function enforceNavigation(details) {
  if (!(await guardedTab(details.tabId))) return;
  const url = details.url;
  if (!url || url === 'about:blank') return;
  if (isDeniedUrl(url) || !(await isOriginAllowed(url))) {
    if (!isDeniedUrl(url)) wait(`site:${originOfUrl(url)}`);
    if (details.frameId === 0) {
      try { await chrome.tabs.update(details.tabId, { url: 'about:blank' }); } catch {}
    } else if (isDeniedUrl(url)) {
      try { await chrome.tabs.remove(details.tabId); } catch {}
    }
  }
}
chrome.webNavigation.onBeforeNavigate.addListener(enforceNavigation);
chrome.webNavigation.onCommitted.addListener(enforceNavigation);
chrome.tabs.onCreated.addListener(async (tab) => {
  // Только вкладка, открытая страницей вкладки бота, и только в группе этого бота.
  // Вкладки, которые человек открывает руками, никогда не усыновляются.
  if (tab.openerTabId === undefined) return;
  const opener = findTab(tab.openerTabId);
  if (!opener || findTab(tab.id)) return;
  if (opener.bot.groupId < 0 || tab.groupId !== opener.bot.groupId) return;
  const url = tab.pendingUrl || 'about:blank';
  if (url !== 'about:blank' && (isDeniedUrl(url) || !(await isOriginAllowed(url)))) return;
  try {
    await adopt(opener.botId, opener.bot, tab.id, url);
  } catch {
    // вкладка закрылась раньше, чем мы успели
  }
});

// ---------------------------------------------------------------------------
// Остановка, значок, окно расширения
// ---------------------------------------------------------------------------

async function stopAll(reason) {
  await detachAll(reason);
  send({ type: 'stopped', reason });
}

function activeTabs() {
  let count = 0;
  for (const bot of bots.values()) count += bot.tabs.size;
  return count;
}

function badge() {
  persist(); // badge() зовут после каждой смены вкладок бота
  const now = Date.now();
  for (const [key, until] of waiting) if (until <= now) waiting.delete(key);
  const count = activeTabs();
  let text = '';
  let color = '#2f7d4f';
  if (count) {
    text = String(count);
  } else if (waiting.size) {
    text = '!'; // секрет или сайт ждут решения человека
    color = '#b26a00';
  } else if (!pairingKeyHex) {
    text = '?'; // нет связи с приложением по коду
    color = '#5f6368';
  } else if (!link.connected) {
    text = '×';
    color = '#b3261e';
  }
  chrome.action.setBadgeText({ text });
  chrome.action.setBadgeBackgroundColor({ color });
}

function popupState() {
  return {
    connected: link.connected,
    paired: !!pairingKeyHex,
    peerUnpaired: !!(conn && conn.peerStatus && conn.peerStatus.paired === false),
    waiting: waiting.size,
    error: link.error,
    browser: browserName(),
    active: [...bots.entries()]
      .filter(([, bot]) => bot.tabs.size > 0)
      .map(([id, bot]) => ({ id, name: bot.name, color: bot.color, tabs: bot.tabs.size })),
    known: known.bots,
  };
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  // Только popup: страницы сайтов сюда писать не могут.
  if (sender.id !== chrome.runtime.id || sender.tab || sender.url !== chrome.runtime.getURL('popup.html')) return false;
  if (message && message.type === 'popup_state') {
    respond(popupState());
    return false;
  }
  if (message && message.type === 'allow_secrets_once') {
    const tabId = message.tabId;
    if (typeof tabId === 'number') {
      secretUnlock.set(tabId, Date.now() + 60000);
      waiting.delete(`secret:${tabId}`);
      badge();
    }
    respond({ ok: true });
    return false;
  }
  if (message && message.type === 'stop_all') {
    stopAll('popup').then(() => respond(popupState()));
    return true;
  }
  if (message && message.type === 'reconnect') {
    connect();
    setTimeout(() => respond(popupState()), 600);
    return true;
  }
  return false;
});

chrome.alarms.create('deloist-reconnect', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === 'deloist-reconnect' && !port) connect();
});
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
connect();
badge();
