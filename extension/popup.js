// Окно расширения: связь с программой, кто работает, какие сайты разрешены, «Остановить всё».
import { deriveK } from './protocol.js';
const t = (key, subs) => chrome.i18n.getMessage(key, subs) || key;
const $ = (id) => document.getElementById(id);

function item(text, extra, color) {
  const li = document.createElement('li');
  if (color) {
    const swatch = document.createElement('span');
    swatch.className = 'swatch';
    swatch.style.background = color;
    li.append(swatch);
  }
  const main = document.createElement('span');
  main.textContent = text;
  li.append(main);
  if (extra) {
    const small = document.createElement('small');
    small.textContent = extra;
    li.append(small);
  }
  return li;
}

function render(state) {
  $('dot').classList.toggle('on', !!state.connected);
  $('status').textContent = state.connected ? t('popupConnected') : t('popupDisconnected');
  const error = state.peerUnpaired ? t('popupPeerUnpaired') : state.error;
  $('error').hidden = state.connected || !error;
  $('error').textContent = error || '';
  $('reconnect').hidden = !!state.connected;
  const active = $('active');
  active.replaceChildren();
  if (!state.active.length) active.append(item(t('popupNobody')));
  for (const bot of state.active) active.append(item(bot.name, t('popupTabs', [String(bot.tabs)]), bot.color));
  const sites = $('sites');
  sites.replaceChildren();
  let any = false;
  for (const bot of state.known) {
    for (const site of bot.sites || []) {
      any = true;
      sites.append(item(site, bot.name + (bot.unattended ? ' · ' + t('popupUnattended') : ''), bot.color));
    }
  }
  if (!any) sites.append(item(t('popupNoSites')));
}

async function loadAllowed() {
  const { allowedOrigins = [] } = await chrome.storage.local.get('allowedOrigins');
  return allowedOrigins;
}

async function saveAllowed(list) {
  await chrome.storage.local.set({ allowedOrigins: list });
}

async function renderPairing() {
  const { pairingKey } = await chrome.storage.local.get('pairingKey');
  const status = pairingKey ? t('popupPaired') : t('popupNotPaired');
  $('pair-status').textContent = status;
}

async function renderAllowed() {
  const list = $('allowed-list');
  list.replaceChildren();
  const origins = await loadAllowed();
  if (!origins.length) list.append(item(t('popupNoAllowed')));
  for (const origin of origins) {
    const li = item(origin);
    const btn = document.createElement('button');
    btn.textContent = t('popupRemove');
    btn.className = 'ghost';
    btn.addEventListener('click', async () => {
      await saveAllowed(origins.filter((x) => x !== origin));
      await renderAllowed();
    });
    li.append(btn);
    list.append(li);
  }
}

async function refresh() {
  render(await chrome.runtime.sendMessage({ type: 'popup_state' }));
  await renderPairing();
  await renderAllowed();
}

$('active-title').textContent = t('popupActive');
$('sites-title').textContent = t('popupSites');
$('stop').textContent = t('popupStop');
$('reconnect').textContent = t('popupReconnect');
$('note').textContent = t('popupNote');
$('pair-title').textContent = t('popupPairTitle');
$('pair-btn').textContent = t('popupPair');
$('unpair-btn').textContent = t('popupUnpair');
$('pair-btn').addEventListener('click', async () => {
  const code = $('pair-code').value.trim();
  if (!code) return;
  const key = await deriveK(code);
  await chrome.storage.local.set({ pairingKey: key });
  await renderPairing();
});
$('unpair-btn').addEventListener('click', async () => {
  await chrome.storage.local.remove(['pairingKey']);
  await renderPairing();
  await renderAllowed();
});

$('allowed-title').textContent = t('popupAllowedTitle');
$('allow-site').textContent = t('popupAllowSite');
$('allow-secret').textContent = t('popupAllowSecret');
$('allow-site').addEventListener('click', async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !tab.url) return;
  let origin;
  try { origin = new URL(tab.url).origin; } catch { return; }
  const origins = await loadAllowed();
  if (!origins.includes(origin)) origins.push(origin);
  await saveAllowed(origins);
  await renderAllowed();
});
$('allow-secret').addEventListener('click', async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab) return;
  await chrome.runtime.sendMessage({ type: 'allow_secrets_once', tabId: tab.id });
});

$('stop').addEventListener('click', async () => {
  render(await chrome.runtime.sendMessage({ type: 'stop_all' }));
  $('note').textContent = t('popupStopped');
});
$('reconnect').addEventListener('click', async () => render(await chrome.runtime.sendMessage({ type: 'reconnect' })));
refresh();
setInterval(refresh, 1500);
