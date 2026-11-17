// Клиент Deloist: один поток событий, один рендер, никакой сборки.

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

// Вторая панель разделённого окна: эта же страница во фрейме (theme.js ставит флаг по адресу ?pane=1).
const PANE = window.DELOIST_PANE || null;

const S = {
  bots: [],
  rooms: [],
  skills: [],
  mcp: [],
  routines: [],
  settings: {},
  levels: {}, // провайдер → { strong, standard, fast } из /api/state
  dataDir: '',
  currentId: null,
  view: { kind: 'bot', id: '' },
  // Кто отвечает в каждой занятой группе: roomId → { botId, name }.
  roomActive: new Map(),
  roomLive: null,
  threads: new Map(),
  roomThreads: new Map(),
  live: null, // {turnId, step, row, bubble, text}
  openFile: '', // файл воркспейса, который «Файлы» покажет при открытии (ссылка из ответа)
  turnMs: null, // id ответа -> длительность хода, считается при показе треда
  busy: new Set(),
  roomsBusy: new Set(),
  unread: new Set(),
  roomsUnread: new Set(),
  filter: '',
  tab: 'profile',
  cards: new Map(),
  waiting: new Map(), // бот → id его висящих подтверждений и вопросов
  queues: new Map(), // бот → {paused, items}
  onSpare: new Map(), // бот → запасная модель, пока на ней идёт ход
  avatars: new Map(), // живые аватары: `side:<бот>` в списке, `head` в шапке
  picker: [], // аватары панели выбора внешности
  journal: { entries: [], seenAt: 0, usage: null, limit: 0 },
  sessionStats: new Map(), // счётчики сеанса бота для строки под полем ввода
  journalQuietOpen: false,
  waitTitle: new Map(), // бот → о чём он спрашивает
  teamAway: '',
  learnedSkills: new Map(), // имя навыка (строчными) → id, из события skill_learned
  intros: new Map(), // бот → приветствие и три задачи (или обещание их)
  memoryRunning: false,
};

const inRoom = () => S.view.kind === 'room';
const roomById = (id) => S.rooms.find((r) => r.id === id) || null;
const currentRoom = () => (inRoom() ? roomById(S.view.id) : null);

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

async function api(path, { method = 'GET', body } = {}) {
  const res = await fetch(path, {
    method,
    headers: body ? { 'content-type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const error = new Error(data.error || `HTTP ${res.status}`);
    error.status = res.status;
    error.data = data;
    if (data.code) error.code = data.code;
    throw error;
  }
  return data;
}

// ---------------------------------------------------------------------------
// Утилиты
// ---------------------------------------------------------------------------

const escapeHtml = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const safeColor = (c) => (/^#[0-9a-f]{6}$/i.test(c) ? c : '#7a8595');

// Цвета аватаров. Только для них, не для интерфейса.
const BOT_COLORS = window.CrewAvatar ? CrewAvatar.COLORS : ['#4f9ce6', '#8e6ac9', '#f2a93b', '#3fb8bd', '#d06ac0', '#4fbf8a'];

// Значки: рисунки лежат в ui/src/lib/icons.js, сюда приходят готовой строкой через DeloistUI.icon (kit.js грузится раньше app.js).
const ic = (name, opts = {}) => DeloistUI.icon(name, { className: 'i', ...opts });
const ICONS = {
  tool: ic('tool'),
  send: ic('send'),
  stop: ic('stop'),
  close: ic('close'),
  group: ic('group'),
  chev: ic('chevron-right', { className: 'i t-chev' }),
};
// Значки статичной разметки index.html (svg с data-icon) заполняются сразу, до первой отрисовки.
DeloistUI.paintIcons();

const avatarInitial = (bot) => (bot.name || '?').trim().charAt(0).toUpperCase() || '?';

// Что бот делает сейчас: ждёт решения важнее, чем работает.
function botState(id) {
  if (S.waiting.get(id)?.size) return 'wait';
  return S.busy.has(id) ? 'work' : 'idle';
}

// Живой аватар — один из двенадцати стеклянных шаров. Без WebGL2 остаётся кружок с инициалом, как в Deloist.
const lookOf = (bot) => ({
  color: safeColor(bot.color),
  skin: bot.skin ?? 0,
  name: bot.name || '',
});

function paintAvatar(el, bot, slot = `side:${bot.id}`, options = {}) {
  const look = lookOf(bot);
  let avatar = S.avatars.get(slot);
  if (!avatar && window.CrewAvatar) {
    el.textContent = '';
    avatar = CrewAvatar.mount(el, { size: 40, ...look, ...options });
    if (avatar) S.avatars.set(slot, avatar);
  }
  if (avatar) {
    if (avatar.canvas.parentElement !== el) {
      el.textContent = '';
      avatar.attach(el);
    }
    avatar.set({ ...look, state: botState(bot.id), quiet: true });
    avatarStates?.sync(bot.id);
    el.classList.add('live');
    el.style.background = '';
    paintBadges(el, bot);
    return;
  }
  el.textContent = avatarInitial(bot);
  el.style.background = safeColor(bot.color);
  paintBadges(el, bot);
}

// Значки на аватаре: сколько сообщений ждёт в очереди и не сорвался ли последний ход.
// Ошибку помнит тред, поэтому утром она видна и после перезапуска; пока идёт новый ход, её нет.
function paintBadges(el, bot) {
  el.querySelector('.avatar-badges')?.remove();
  const count = queuedCount(bot.id);
  const failed = Boolean(bot.preview?.error) && !S.busy.has(bot.id);
  if (!count && !failed) return;

  const box = document.createElement('div');
  box.className = 'avatar-badges';
  if (count) {
    const badge = document.createElement('div');
    badge.className = 'avatar-badge';
    badge.textContent = count;
    badge.title = tr('В очереди: {count}', { count });
    box.append(badge);
  }
  if (failed) {
    const badge = document.createElement('div');
    badge.className = 'avatar-badge is-error';
    badge.title = bot.preview.text;
    badge.setAttribute('role', 'img');
    badge.setAttribute('aria-label', tr('Последний ход не удался'));
    badge.innerHTML = ic('close');
    box.append(badge);
  }
  el.append(box);
}

// Настроение после события: списка не перерисовываем, аватар сам знает, как перейти.
// Все живые аватары бота в окне: в списке, в команде и в шапке открытого чата.
// Строка бота внутри команды живёт в слоте «side:<бот>:<команда>»: у одного бота их может быть несколько, и все они его.
const slotBotId = (slot) => (/^(?:side|team):([^:]+)/.exec(slot) || [])[1] || '';
const avatarsOf = (id) => {
  const found = [];
  for (const [slot, avatar] of S.avatars) if (slotBotId(slot) === id || (slot === 'head' && id === S.currentId)) found.push(avatar);
  return found;
};
const feelBot = (id, mood, seconds) => avatarsOf(id).forEach((avatar) => avatar.feel(mood, seconds));
// Знак занятия и эмоции от событий хода: правила в avatar-states.js, общие для программы, дока и питомцев.
const avatarStates = window.CrewAvatarStates?.createTracker(avatarsOf);
// Ход остановил человек: у такого конца хода своя реакция, звёзд за выполненную работу не будет.
const stoppedByHuman = new Set();

function syncAvatar(id, extra = {}) {
  const state = botState(id);
  avatarsOf(id).forEach((avatar) => avatar.set({ state, ...extra }));
}

// Пока человек пишет боту, его аватары «слушают» поле ввода; замолчал на пару секунд — снова за мышью.
let listeners = [];
let listenTimer = 0;

const botAvatars = avatarsOf;

function listenToInput(on) {
  const next = on && !inRoom() && S.currentId ? botAvatars(S.currentId) : [];
  for (const avatar of listeners) if (!next.includes(avatar)) avatar.listen?.(null);
  for (const avatar of next) avatar.listen?.($('#input'));
  listeners = next;
  clearTimeout(listenTimer);
  if (next.length) listenTimer = setTimeout(() => listenToInput(false), 2500);
}

function trackMood(ev) {
  avatarStates?.feed(ev);
  const set = S.waiting.get(ev.botId);
  const wait = window.CrewAvatarStates?.waitOf(ev);
  if (wait?.open) {
    if (set) set.add(wait.open);
    else S.waiting.set(ev.botId, new Set([wait.open]));
    S.waitTitle.set(ev.botId, ev.title || ev.question || ev.reason || ev.offer?.specialistName || '');
    syncAvatar(ev.botId);
  } else if (wait?.close) {
    set?.delete(wait.close);
    syncAvatar(ev.botId);
  } else if (set && window.CrewAvatarStates?.clears(ev) && ev.type !== 'turn_start' && ev.type !== 'turn_end') {
    // Просьбу посмотреть браузер и предложение специалиста никто не закрывает событием: бот пошёл дальше сам.
    const before = set.size;
    for (const id of [...set]) if (id === 'human' || id.startsWith('offer:')) set.delete(id);
    if (set.size !== before) syncAvatar(ev.botId);
  } else if (ev.type === 'turn_start') {
    S.busy.add(ev.botId);
    syncAvatar(ev.botId);
  } else if (ev.type === 'turn_end') {
    S.busy.delete(ev.botId);
    S.waiting.delete(ev.botId);
    S.waitTitle.delete(ev.botId);
    const stopped = stoppedByHuman.delete(ev.botId);
    syncAvatar(ev.botId, ev.quiet || stopped ? { quiet: true } : {});
    if (ev.failed && !stopped && avatarStates) {
      const { mood, seconds } = avatarStates.endMood(ev.botId);
      feelBot(ev.botId, mood, seconds);
    } else if (ev.failed && !stopped) {
      feelBot(ev.botId, 'error', 5);
    }
  }
}

function markSwitch(id, value) {
  $$(`#${id} [data-value]`).forEach((btn) => btn.setAttribute('aria-checked', String(btn.dataset.value === value)));
}

function markLook() {
  markSwitch('theme-switch', document.documentElement.dataset.theme);
  markSwitch('font-switch', S.settings.uiFont || 'system');
  markSwitch('lang-switch', window.LANG);
}

// Шрифт ставит theme.js до первой отрисовки, поэтому выбор держим и в браузере.
function applyFont(font) {
  if (font && font !== 'system') document.documentElement.dataset.font = font;
  else delete document.documentElement.dataset.font;
  try {
    localStorage.setItem('deloist-font', font || 'system');
  } catch {}
}

async function saveLook(body) {
  try {
    const { settings } = await api('/api/settings', { method: 'PUT', body });
    if (settings) S.settings = settings;
  } catch (err) {
    toast(tr('Не удалось запомнить: {error}', { error: err.message }));
  }
}

// Sun/moon buttons use the same path as the Settings switch; the page crossfades for a moment.
function toggleTheme() {
  const root = document.documentElement;
  root.classList.add('theme-fade');
  chooseTheme(root.dataset.theme === 'dark' ? 'light' : 'dark');
  setTimeout(() => root.classList.remove('theme-fade'), 450);
}

// Sidebar rail: state in localStorage, Ctrl+B, width animated by CSS.
const SIDE_KEY = 'deloist-sidebar';

function sidebarCollapsed() {
  return document.body.classList.contains('side-collapsed');
}

function setSidebarCollapsed(collapsed, animate = true) {
  const body = document.body;
  if (animate) {
    body.classList.add('side-anim');
    setTimeout(() => body.classList.remove('side-anim'), 650);
  }
  body.classList.toggle('side-collapsed', collapsed);
  const btn = $('#btn-side-collapse');
  if (btn) {
    const label = collapsed ? tr('Развернуть боковую панель') : tr('Свернуть боковую панель');
    btn.setAttribute('aria-expanded', String(!collapsed));
    btn.setAttribute('aria-label', label);
    btn.title = `${label} (Ctrl+B)`;
  }
  try {
    localStorage.setItem(SIDE_KEY, collapsed ? 'collapsed' : 'open');
  } catch {}
}

function initSidebarCollapse() {
  let saved = '';
  try {
    saved = localStorage.getItem(SIDE_KEY) || '';
  } catch {}
  setSidebarCollapsed(saved === 'collapsed', false);
  $('#btn-side-collapse')?.addEventListener('click', () => setSidebarCollapsed(!sidebarCollapsed()));
}

function setSearchOpen(open) {
  const field = $('#sidebar-search');
  const toggle = $('#btn-search-toggle');
  field.hidden = !open;
  toggle.setAttribute('aria-expanded', String(open));
  if (open) {
    if (sidebarCollapsed()) setSidebarCollapsed(false);
    $('#sidebar').classList.add('open');
    $('#search').focus();
  }
}

// Ctrl+B folds the sidebar, Ctrl+K opens search.
function arcShortcuts(e) {
  if (PANE) return;
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
  const key = e.key.toLowerCase();
  if (key === 'b') {
    e.preventDefault();
    setSidebarCollapsed(!sidebarCollapsed());
  } else if (key === 'k') {
    const search = $('#search');
    if (!search) return;
    e.preventDefault();
    setSearchOpen(true);
    search.select();
  }
}

// New rows (a message, an approval card, a hand-off line) pop in; a thread that is being opened does not.
function watchNewRows() {
  const host = messagesEl();
  new MutationObserver((records) => {
    if (host.dataset.history === '1') return;
    for (const rec of records) {
      for (const node of rec.addedNodes) {
        if (node.nodeType !== 1 || node.matches('.empty, .intro')) continue;
        node.classList.add('arc-pop');
        node.addEventListener('animationend', () => node.classList.remove('arc-pop'), { once: true });
      }
    }
  }).observe(host, { childList: true });
}

// Содержание разговора: каждое сообщение человека открывает раздел; список строится из уже нарисованных строк.
const Toc = (() => {
  let btn, pop, list, io, rows = [], current = -1, isOpen = false, queued = 0;

  const titleOf = (row) => {
    const who = $('.msg-author', row)?.textContent.trim();
    const bubble = $('.bubble', row);
    const first = (bubble?.textContent || '').split('\n').map((s) => s.trim()).find(Boolean) || '';
    let text = first.length > 60 ? `${first.slice(0, 59).trimEnd()}…` : first;
    if (!text && row.dataset.files) text = `${tr('Файлы')}: ${row.dataset.files}`;
    if (!text) text = '…';
    return who ? `${who}: ${text}` : text;
  };

  function place() {
    const host = messagesEl();
    btn.style.top = `${host.offsetTop + 8}px`;
    pop.style.top = `${host.offsetTop + 42}px`;
  }

  function pick() {
    if (!rows.length) return;
    const host = messagesEl();
    const line = host.getBoundingClientRect().top + host.clientHeight * 0.3;
    let at = 0;
    rows.forEach((row, i) => { if (row.getBoundingClientRect().top <= line) at = i; });
    if (at === current) return;
    current = at;
    [...list.children].forEach((li, i) => li.classList.toggle('is-current', i === at));
    if (isOpen) list.children[at]?.scrollIntoView({ block: 'nearest' });
  }

  function rebuild() {
    queued = 0;
    const host = messagesEl();
    rows = $$('.msg.user', host);
    io?.disconnect();
    btn.hidden = rows.length < 3;
    if (btn.hidden) { close(); return; }
    place();
    list.replaceChildren(...rows.map((row, i) => {
      const li = document.createElement('li');
      li.setAttribute('role', 'option');
      li.tabIndex = -1;
      const dot = document.createElement('i');
      const name = document.createElement('span');
      name.className = 'toc-name';
      name.textContent = titleOf(row);
      const time = document.createElement('time');
      time.textContent = row.dataset.at ? whenText(row.dataset.at) : '';
      li.append(dot, name, time);
      li.addEventListener('click', () => jump(i));
      return li;
    }));
    current = -1;
    io = new IntersectionObserver(pick, { root: host, rootMargin: '0px 0px -70% 0px', threshold: [0, 1] });
    rows.forEach((row) => io.observe(row));
    pick();
    if (isOpen) list.children[current]?.scrollIntoView({ block: 'nearest' });
  }

  function jump(i) {
    const host = messagesEl();
    const delta = rows[i].getBoundingClientRect().top - host.getBoundingClientRect().top - 16;
    close();
    // Как в scrollBottom: далеко — сразу, рядом — плавно.
    if (Math.abs(delta) > host.clientHeight * 2) {
      host.style.scrollBehavior = 'auto';
      host.scrollTop += delta;
      requestAnimationFrame(() => { host.style.scrollBehavior = ''; });
    } else {
      host.scrollTop += delta;
    }
    current = i;
    [...list.children].forEach((li, k) => li.classList.toggle('is-current', k === i));
  }

  function close(refocus = false) {
    if (!isOpen) return;
    isOpen = false;
    pop.hidden = true;
    btn.setAttribute('aria-expanded', 'false');
    if (refocus) btn.focus();
  }

  function open() {
    isOpen = true;
    pop.hidden = false;
    btn.setAttribute('aria-expanded', 'true');
    place();
    list.children[current]?.scrollIntoView({ block: 'nearest' });
  }

  function init() {
    btn = $('#toc-btn');
    pop = $('#toc-pop');
    list = $('#toc-list');
    const schedule = () => { if (!queued) queued = requestAnimationFrame(rebuild); };
    new MutationObserver(schedule).observe(messagesEl(), { childList: true });
    new ResizeObserver(() => btn.hidden || place()).observe(messagesEl());
    btn.addEventListener('click', () => (isOpen ? close() : open()));
    document.addEventListener('mousedown', (e) => {
      if (isOpen && !pop.contains(e.target) && !btn.contains(e.target)) close();
    });
    document.addEventListener('keydown', (e) => {
      if (isOpen && e.key === 'Escape') { e.stopPropagation(); close(true); }
    }, true);
    schedule();
  }
  return { init };
})();

function chooseTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem('crew-theme', theme);
  } catch {}
  markLook();
  // Док и питомцы живут в своих окнах и узнают тему только из настроек.
  saveLook({ uiTheme: theme });
}

async function chooseFont(font) {
  S.settings.uiFont = font;
  applyFont(font);
  markLook();
  await saveLook({ uiFont: font });
}

async function chooseLang(lang) {
  if (lang === window.LANG) return;
  await saveLook({ uiLang: lang });
  if (rememberLang(lang)) location.reload();
}

// После перезагрузки окна карточки живут только на сервере: занятым ботам спрашиваем их.
async function syncWaiting() {
  for (const id of S.busy) {
    try {
      const { pending } = await api(`/api/bots/${id}/pending`);
      if (pending.length) {
        S.waiting.set(id, new Set(pending.map((item) => item.id)));
        S.waitTitle.set(id, pending[0].title || pending[0].question || '');
      }
      syncAvatar(id);
    } catch {
      // бот мог исчезнуть
    }
  }
}

function pruneAvatars() {
  for (const [slot, avatar] of S.avatars) {
    if (slotBotId(slot) && !botById(slotBotId(slot))) {
      avatar.destroy();
      S.avatars.delete(slot);
    }
  }
}

// Разметка ответа: абзацы, заголовки, списки, таблицы, код, ссылки со значком сервиса. Живёт в ui/src/lib/markdown.js.
function md(text) {
  return window.DeloistUI.markdown(text);
}

const timeOf = (ts) => {
  const d = new Date(ts || Date.now());
  return d.toLocaleTimeString(window.LOCALE, { hour: '2-digit', minute: '2-digit' });
};

const fmtCount = (n) => {
  const v = Number(n || 0);
  const short = (x, digits) => x.toLocaleString(window.LOCALE, { maximumFractionDigits: digits });
  if (v >= 1000000) return tr('{n} млн', { n: short(v / 1000000, 1) });
  if (v >= 1000) return tr('{n} тыс', { n: short(v / 1000, v >= 10000 ? 0 : 1) });
  return String(v);
};

// Что показать под ответом: модель, расход и доля кэша. Кэш здесь не украшение:
// по нему видно, держится ли стабильный префикс промпта (и не сломали ли его).
function usageMeta(usage, model) {
  if (!usage) return '';
  const parts = [];
  if (model) parts.push(model);
  const prompt = Number(usage.prompt || 0);
  const completion = Number(usage.completion || 0);
  const cached = Number(usage.cached || 0);
  if (prompt) parts.push(tr('вход {n}', { n: fmtCount(prompt) }));
  if (completion) parts.push(tr('выход {n}', { n: fmtCount(completion) }));
  if (prompt > 0 && cached > 0) parts.push(tr('кэш {n}%', { n: Math.round((cached / prompt) * 100) }));
  return parts.join(' · ');
}

function metaNode(usage, model) {
  const text = usageMeta(usage, model);
  if (!text) return null;
  const el = document.createElement('div');
  el.className = 'msg-meta';
  el.textContent = text;
  return el;
}

function toast(message, ms = 3200) {
  const el = $('#toast');
  el.textContent = trServer(message);
  el.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => { el.hidden = true; }, ms);
}

// Файл ключей не открылся (чужой пользователь, потерянный ключ шифрования): запись ключей закрыта,
// файл цел. Выйти отсюда может только человек, кнопкой: файл уходит в копию рядом.
function paintSecretsBanner(unavailable) {
  let el = $('#secrets-banner');
  if (!unavailable) {
    if (el) el.hidden = true;
    return;
  }
  if (!el) {
    el = document.createElement('div');
    el.id = 'secrets-banner';
    el.className = 'secrets-banner';
    el.setAttribute('role', 'alert');
    const text = document.createElement('span');
    text.textContent = tr('Ключи недоступны: сохранённые ключи не прочитались, поэтому новые не сохраняются. Файл цел. Если вернуть ключ не получится, можно начать с пустых.');
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'ghost';
    button.textContent = tr('Начать с пустыми ключами');
    button.addEventListener('click', async () => {
      if (!confirm(tr('Прежний файл ключей будет отложен в сторону (не удалён), а ключи начнутся заново. Продолжить?'))) return;
      try {
        await api('/api/secrets/reset-unavailable', { method: 'POST' });
        toast(tr('Ключи начаты заново. Прежний файл отложен рядом с данными.'));
        refreshState();
      } catch (err) {
        toast(tr('Не удалось: {error}', { error: err.message }));
      }
    });
    el.append(text, button);
    document.body.prepend(el);
  }
  el.hidden = false;
}

const statusLabel = (status) =>
  ({
    running: tr('выполняется…'),
    waiting: tr('ждёт твоего решения'),
    ok: tr('готово'),
    error: tr('ошибка'),
    denied: tr('отклонено'),
    timeout: tr('истекло'),
  }[status] || status || '');

// ---------------------------------------------------------------------------
// Сайдбар
// ---------------------------------------------------------------------------

function botById(id) {
  return S.bots.find((b) => b.id === id) || null;
}

// Число сообщений в очереди: загруженная очередь точнее счётчика из состояния.
function queuedCount(id) {
  const queue = S.queues.get(id);
  if (queue) return queue.items.length;
  return Number(botById(id)?.queued) || 0;
}

function renderSidebar() {
  renderNeeds();
  const current = botById(S.currentId);
  if (current && !inRoom()) paintBadges($('#head-avatar'), current);
  paintGuests();
  $('#btn-team').classList.toggle('on', teamOpen());
  scheduleTeam();

  const list = $('#bot-list');
  const q = S.filter.trim().toLowerCase();
  const bots = S.bots
    .filter((b) => !q || `${b.name} ${b.role}`.toLowerCase().includes(q))
    .sort((a, b) => Number(b.pinned) - Number(a.pinned) || (b.preview?.at || b.createdAt) - (a.preview?.at || a.createdAt));
  const shown = new Set(bots.map((b) => b.id));
  const rooms = S.rooms
    .filter((r) => !q || `${r.name} ${r.instructions || ''}`.toLowerCase().includes(q) || (r.botIds || []).some((id) => shown.has(id)))
    .sort((a, b) => (b.preview?.at || b.createdAt) - (a.preview?.at || a.createdAt));

  const props = {
    filter: S.filter,
    currentId: inRoom() ? '' : S.currentId,
    roomIcon: ICONS.group,
    bots: bots.map((bot) => ({
      id: bot.id,
      name: bot.name,
      rank: bot.rank || 'lead',
      pinned: Boolean(bot.pinned),
      busy: S.busy.has(bot.id),
      unread: S.unread.has(bot.id),
      needs: Boolean(S.waiting.get(bot.id)?.size),
      error: Boolean(bot.preview?.error && !S.busy.has(bot.id)),
      time: bot.preview ? timeOf(bot.preview.at) : '',
      preview: bot.preview
        ? bot.preview.error
          ? explainError(bot.preview.text).title
          : `${bot.preview.role === 'user' ? tr('ты: ') : ''}${bot.preview.text}`
        : bot.role || '',
    })),
    rooms: rooms.map((room) => {
      const who = room.preview?.name ? `${room.preview.name}: ` : room.preview?.role === 'user' ? tr('ты: ') : '';
      return {
        id: room.id,
        name: room.name,
        emoji: room.emoji || '',
        color: room.color || '',
        memberIds: room.botIds || [],
        lead: room.lead || '',
        archived: Boolean(room.archived),
        active: inRoom() && room.id === S.view.id,
        unread: S.roomsUnread.has(room.id),
        time: room.preview ? timeOf(room.preview.at) : '',
        // Пустое превью оставляет подпись списку: он покажет участников команды или «пока пусто».
        preview: room.preview ? `${who}${room.preview.text}` : '',
      };
    }),
  };
  if (botListView) botListView.update(props);
  else botListView = window.DeloistUI.mountBotList(list, { ...props, paint: paintSideAvatar, onselect: pickBot, onselectroom: pickRoom, onpin: pinBot, onmenu: openBotMenu });
  // Вид бота мог поменяться: строки Svelte живут дольше одной отрисовки, перерисовываем аватары на месте.
  setTimeout(() => {
    for (const el of $$('#bot-list .sb-row[data-id] .sb-avatar:not(.room-icon)')) {
      const bot = botById(el.closest('[data-id]').dataset.id);
      if (bot) paintAvatar(el, bot, el.dataset.slot || undefined);
    }
    // Names as tooltips: in the collapsed rail only the avatar is left.
    for (const row of $$('#bot-list .sb-row[data-id]')) {
      const name = row.querySelector('.k-list-item__title')?.textContent || '';
      row.title = row.classList.contains('is-repeat') ? `${name} · ${tr('есть и в другой команде')}` : name;
    }
    Split.decorate();
  }, 0);
  Split.sync();
}

let botListView = null;

// Гости треда: специалисты, которым этот бот передал задачу. Имя открывает специалиста,
// крестик отпускает его в запас раньше срока.
function paintGuests() {
  const box = $('#head-guests');
  const bot = inRoom() ? null : botById(S.currentId);
  const guests = (bot?.guests || []).map((g) => botById(g.botId)).filter(Boolean);
  box.hidden = !guests.length;
  const key = guests.map((g) => `${g.id}:${g.name}`).join('|');
  if (box.dataset.key === key) return;
  box.dataset.key = key;
  box.innerHTML = '';
  if (!guests.length) return;
  const label = document.createElement('span');
  label.className = 'head-guests-label';
  label.textContent = tr('Гости');
  box.append(label);
  for (const guest of guests) {
    const chip = document.createElement('span');
    chip.className = 'guest-chip';
    chip.dataset.id = guest.id;
    chip.innerHTML = `<button type="button" class="guest-open"></button><button type="button" class="guest-drop" aria-label="">${DeloistUI.icon('close', { size: 12, stroke: 2.4 })}</button>`;
    const open = chip.querySelector('.guest-open');
    open.textContent = guest.name;
    open.title = tr('Открыть чат специалиста');
    open.addEventListener('click', () => selectBot(guest.id));
    const drop = chip.querySelector('.guest-drop');
    drop.title = tr('Отпустить в запас');
    drop.setAttribute('aria-label', tr('Отпустить в запас: {name}', { name: guest.name }));
    drop.addEventListener('click', async () => {
      try {
        await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: { removeGuest: guest.id } });
        await refreshState();
      } catch (err) {
        toast(err.message);
      }
    });
    box.append(chip);
  }
}

// Аватар строки списка. Когда строка уходит (раздел свернули, бота убрали), живой аватар гасим,
// чтобы скрытый холст не крутил анимацию.
function paintSideAvatar(el, id, slot = `side:${id}`) {
  const bot = botById(id);
  if (bot) paintAvatar(el, bot, slot);
  return () => {
    const avatar = S.avatars.get(slot);
    if (avatar && avatar.canvas.parentElement === el) {
      avatar.destroy();
      S.avatars.delete(slot);
    }
  };
}

function pickBot(id) {
  Split.beforeSelect('bot', id);
  selectBot(id);
  $('#sidebar').classList.remove('open');
}

function pickRoom(id) {
  Split.beforeSelect('room', id);
  selectRoom(id);
  $('#sidebar').classList.remove('open');
}

async function pinBot(id, pinned) {
  try {
    await api(`/api/bots/${id}`, { method: 'PATCH', body: { pinned } });
    await refreshState();
  } catch (err) {
    toast(err.message);
  }
}

// Меню бота в списке: кнопка «Настройки» в строке и правый щелчок. Меню кита, у курсора или под кнопкой.
let botMenu = null;

function closeBotMenu(refocus = false) {
  if (!botMenu) return;
  const { view, pop, trigger } = botMenu;
  botMenu = null;
  view.destroy();
  pop.remove();
  trigger?.setAttribute('aria-expanded', 'false');
  if (refocus) trigger?.focus({ preventScroll: true });
}

function botMenuItems(bot) {
  return [
    { id: 'profile', label: tr('Настройки бота'), icon: 'gear-simple' },
    { id: 'memory', label: tr('Память'), icon: 'brain-halves' },
    { id: 'files', label: tr('Файлы'), icon: 'file-lines' },
    { id: 'routines', label: tr('Рутины'), icon: 'repeat' },
    { kind: 'sep' },
    { id: 'move', label: tr('Перенести в команду'), icon: 'users', hint: '›' },
    { id: 'pin', label: bot.pinned ? tr('Открепить') : tr('Закрепить'), icon: 'pin' },
    { id: 'unread', label: S.unread.has(bot.id) ? tr('Отметить прочитанным') : tr('Отметить непрочитанным'), icon: 'eye' },
    { id: 'dock', label: bot.dock !== false ? tr('Убрать из дока') : tr('Показать в доке'), icon: 'dock-hide' },
    { id: 'compact', label: tr('Сжать историю'), icon: 'compact' },
    { kind: 'sep' },
    { id: 'delete', label: tr('Удалить бота'), icon: 'trash', danger: true },
  ];
}

// Второй экран меню: команды. Уже состоящий в команде бот видит её отмеченной; «Без команды» — только из ветки команды.
function botMoveItems(bot, from) {
  const live = S.rooms.filter((room) => !room.archived);
  return [
    { id: 'back', label: tr('Назад'), icon: 'chevron-left' },
    { kind: 'sep' },
    ...live.map((room) => {
      const inside = room.botIds.includes(bot.id);
      return { id: `room:${room.id}`, label: room.name, icon: inside ? 'check' : 'users', disabled: inside, tip: tr('Уже в этой команде') };
    }),
    ...(live.length ? [{ kind: 'sep' }] : []),
    ...(from ? [{ id: 'loose', label: tr('Без команды'), icon: 'close' }] : []),
    { id: 'new-team', label: tr('Новая команда…'), icon: 'plus' },
  ];
}

function openBotMenu(id, { x, y, trigger, from = '' }, page = 'main') {
  const reopen = page === 'main' && botMenu?.id === id && trigger && botMenu.trigger === trigger;
  closeBotMenu();
  if (reopen) return;
  const bot = botById(id);
  if (!bot) return;
  const pop = document.createElement('div');
  pop.className = 'bot-menu-pop';
  document.body.append(pop);
  trigger?.setAttribute('aria-expanded', 'true');
  const at = { x, y, trigger, from };
  const view = window.DeloistUI.mountMenu(pop, {
    items: page === 'move' ? botMoveItems(bot, from) : botMenuItems(bot),
    label: page === 'move' ? tr('Перенести в команду') : tr('Бот «{name}»', { name: bot.name }),
    onpick: (pick) => {
      if (pick === 'move' || pick === 'back') return openBotMenu(id, at, pick === 'move' ? 'move' : 'main');
      closeBotMenu();
      botMenuPick(bot, pick, from);
    },
    onclose: (focus) => closeBotMenu(focus),
  });
  botMenu = { id, view, pop, trigger };
  // Меню не уходит за край окна: сначала ставим, потом сдвигаем по своему размеру.
  const box = pop.getBoundingClientRect();
  const left = trigger ? x - box.width : x;
  pop.style.left = `${Math.max(8, Math.min(left, innerWidth - box.width - 8))}px`;
  pop.style.top = `${Math.max(8, Math.min(y + 4, innerHeight - box.height - 8))}px`;
}

async function botMenuPick(bot, pick, from = '') {
  const id = bot.id;
  // Перенос как перетаскивание: из команды строки — переезд, из «Без команды» — добавление.
  if (pick.startsWith('room:')) return moveBotToTeam(id, pick.slice(5), from);
  if (pick === 'loose') return moveBotToTeam(id, '', from);
  if (pick === 'new-team') return openRoomModal(null, { members: [id] });
  if (['profile', 'memory', 'files', 'routines'].includes(pick)) {
    if (S.currentId !== id || inRoom()) pickBot(id);
    openDrawer(pick);
    return;
  }
  if (pick === 'pin') return pinBot(id, !bot.pinned);
  if (pick === 'unread') {
    if (S.unread.has(id)) S.unread.delete(id);
    else S.unread.add(id);
    renderSidebar();
    return;
  }
  if (pick === 'dock') return setDockBot(id, bot.dock === false);
  if (pick === 'compact') return compactBot(id);
  if (pick === 'delete') {
    if (!confirm(tr('Удалить «{name}» вместе с чатом и памятью?', { name: bot.name }))) return;
    try {
      await api(`/api/bots/${id}`, { method: 'DELETE' });
      const wasOpen = !inRoom() && S.currentId === id;
      await refreshState();
      if (wasOpen) showTeam();
      toast(tr('Бот удалён'));
    } catch (err) {
      toast(err.message);
    }
  }
}

// ---------------------------------------------------------------------------
// Чат
// ---------------------------------------------------------------------------

function messagesEl() { return $('#messages'); }

function scrollBottom(force = false) {
  const el = messagesEl();
  const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
  const near = gap < 180;
  if (!force && !near) return;
  // Открыли чат или прыжок длиннее двух экранов: сразу вниз, без плавной прокрутки через всю историю.
  // Короткий сдвиг (пришло сообщение) остаётся плавным.
  if (gap > el.clientHeight * 2) {
    el.style.scrollBehavior = 'auto';
    el.scrollTop = el.scrollHeight;
    requestAnimationFrame(() => { el.style.scrollBehavior = ''; });
  } else {
    el.scrollTop = el.scrollHeight;
  }
}

// Приветствие с большим аватаром живёт, только пока в чате пусто.
function dropEmpty() {
  S.avatars.get('empty')?.destroy();
  S.avatars.delete('empty');
  $('.empty', messagesEl())?.remove();
  $('.intro', messagesEl())?.remove();
}

let workGroup = null;

function ensureWorkGroup() {
  if (workGroup?.isConnected) return workGroup;
  const group = document.createElement('details');
  group.className = 'work-group';
  const summary = document.createElement('summary');
  // Первый span — подпись (её меняют по `summary span`); следом девять пикселей и текущий шаг, пока ход идёт.
  summary.innerHTML = `${ICONS.chev}<span></span><span class="k-pix k-pix--paper k-pix--think work-pix" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span><span class="work-now" aria-live="polite"></span>`;
  summary.querySelector('span').textContent = tr('Ход работы');
  const body = document.createElement('div');
  body.className = 'work-body';
  group.append(summary, body);
  messagesEl().append(group);
  workGroup = group;
  return group;
}

// «Ход работы» пока бот работает: те же девять пикселей, что в плашке дока (StepPill), и текст текущего шага.
// kind: think | read | act | write | wait (docs/design/kit.md). Когда ход кончился, строка остаётся тихой.
function paintWorkNow(kind, text) {
  turnCard.status(text);
  const group = workGroup;
  if (!group?.isConnected) return;
  group.classList.add('is-live');
  const pix = group.querySelector('.work-pix');
  const cls = `k-pix k-pix--paper k-pix--${kind} work-pix`;
  if (pix.className !== cls) pix.className = cls;
  const now = group.querySelector('.work-now');
  if (now.textContent !== text) now.textContent = text;
}

function calmWork() {
  for (const group of $$('.work-group.is-live', messagesEl())) group.classList.remove('is-live');
}

function workKind(name) {
  return /^(read_file|list_files|web_fetch|recall|read_skill|search|memory_)/.test(String(name)) ? 'read' : 'act';
}

// Карточка или служебная строка посреди хода: лента идёт по времени. Прежняя группа остаётся над ней
// свёрнутой и тихой, а следующие шаги и живая строка «работает» идут в новой группе ниже.
function appendInFeed(node) {
  const open = workGroup?.isConnected && workGroup.classList.contains('is-live') ? workGroup : null;
  messagesEl().append(node);
  if (!open) return;
  workGroup = null;
  open.classList.remove('is-live');
  if (!open.querySelector('.work-body').children.length) open.remove();
  ensureWorkGroup();
  splitLive(open);
  paintWorkNow('think', tr('Думает'));
}

function finishWorkGroup() {
  calmWork();
  if (workGroup && !workGroup.querySelector('.work-body').children.length) workGroup.remove();
  workGroup = null;
}

function markWorkError(group) {
  if (group) group.querySelector('summary span').textContent = tr('Ход работы · есть ошибка');
}

function clearMessages() {
  workGroup = null;
  turnCard.detach();
  dropEmpty();
  const host = messagesEl();
  host.innerHTML = '';
  // Rows drawn from history appear at once; only rows that arrive afterwards pop in (watchNewRows).
  host.dataset.history = '1';
  requestAnimationFrame(() => requestAnimationFrame(() => delete host.dataset.history));
}

const TOOL_LABELS = {
  list_files: tr('Смотрит файлы'),
  read_file: tr('Читает файл'),
  write_file: tr('Записывает файл'),
  run_command: tr('Запускает команду'),
  web_fetch: tr('Открывает адрес'),
  web_search: tr('Ищет в интернете'),
  remember: tr('Запоминает'),
  recall: tr('Ищет в памяти'),
  read_skill: tr('Читает навык'),
  save_skill: tr('Сохраняет навык'),
  save_script: tr('Сохраняет скрипт'),
  run_script: tr('Запускает скрипт'),
  delegate: tr('Передаёт задачу'),
  assign_task: tr('Даёт задачу участнику'),
  task_status: tr('Смотрит задачи'),
  team_status: tr('Смотрит команду'),
  share_file: tr('Передаёт файл'),
  list_bots: tr('Смотрит команду'),
  create_bot: tr('Создаёт бота'),
  update_bot: tr('Настраивает бота'),
  browser_nav: tr('Открывает страницу'),
  browser_page: tr('Читает страницу'),
  browser_mouse: tr('Двигает мышью'),
  browser_keyboard: tr('Печатает'),
  browser_devtools: tr('Смотрит в отладчик'),
  browser_human: tr('Зовёт тебя в браузер'),
  browser_show_to_human: tr('Просит посмотреть на экран'),
};

// Агенты ACP зовут наш браузер через MCP: «mcp.deloist_browser.browser_page» → «browser_page».
const plainName = (name) => String(name || "").replace(/^mcp(?:\.|__)[a-z0-9_-]+?(?:\.|__)(?=(?:browser_|web_|delegate$|list_bots$|remember$|recall$|assign_task$|task_status$|team_status$|share_file$))/i, "");
// Внешний агент (ACP) называет шаг своим заголовком: «Execute `<весь скрипт>`», «Write `путь` (unreadable, no preview)»,
// а иногда кладёт туда обрывок JSON. В строку шага и в карточку хода такое не годится: берём короткую подпись по виду шага (kind).
const STEP_KIND_LABELS = {
  execute: tr('Запускает команду'),
  edit: tr('Записывает файл'),
  read: tr('Читает файл'),
  search: tr('Ищет'),
  fetch: tr('Открывает адрес'),
  think: tr('Думает'),
  delete: tr('Удаляет файл'),
  move: tr('Переносит файл'),
};
const MESSY_TITLE = /^[\s{["]|[{}`\r\n]|":/;
function agentLabel(call) {
  const name = String(call.name || '').trim();
  const kind = STEP_KIND_LABELS[call.kind];
  if (kind && (MESSY_TITLE.test(name) || /^(execute|write|edit|read|delete|move|fetch)\b/i.test(name))) return kind;
  if (!name || name.length > 60 || MESSY_TITLE.test(name)) return kind || tr('Выполняет шаг');
  return name;
}
const toolLabel = (call) => TOOL_LABELS[plainName(call.name)] || TOOL_LABELS[call.name] || agentLabel(call);
// Семья шага: все браузерные шаги идут одной строкой, остальные собираются по имени инструмента.
const toolFamily = (call) => {
  const plain = plainName(call.name);
  return plain.startsWith('browser_') ? 'browser' : plain;
};

const hostOf = (url) => {
  try {
    return new URL(/^[a-z][a-z0-9+.-]*:/i.test(url) ? url : 'https://' + url).hostname.replace(/^www\./, '');
  } catch {
    return String(url || '').slice(0, 40);
  }
};

const clip = (value, n = 60) => {
  const s = String(value ?? '').replace(/\s+/g, ' ').trim();
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
};

// Что шаг делает «над чем», из аргументов вызова (у агентов ACP они приходят из rawInput). Пусто, если сказать нечего.
function toolTarget(call) {
  const args = call.args || {};
  const plain = plainName(call.name);
  const str = (v) => (typeof v === 'string' ? v : typeof v === 'number' ? String(v) : '');
  if (plain === 'browser_nav') return args.url ? hostOf(str(args.url)) : '';
  if (plain === 'web_search') return clip(str(args.query) || str(args.q));
  if (plain === 'web_fetch') return args.url ? hostOf(str(args.url)) : '';
  if (plain === 'run_script' || plain === 'save_script') return scriptTarget(str(args.name));
  if (plain === 'browser_mouse') return clip(str(args.label) || str(args.selector) || (args.ref ? '#' + str(args.ref) : ''));
  if (plain === 'browser_keyboard') {
    const typed = str(args.text);
    if (typed) {
      // Пароли и ключи в строку шага не попадают.
      const secret = /passw|парол|secret|token|cvv|card|карт/i.test(str(args.label) + ' ' + str(args.selector));
      return secret ? '••••••' : '«' + clip(typed, 40) + '»';
    }
    return clip(str(args.keys));
  }
  const value = args.command || args.path || args.url || args.query || args.bot || args.name || args.bot_id || '';
  return typeof value === 'string' ? clip(value, 200) : '';
}

// Текст текущего шага для карточки хода и строки «Ход работы»: короткая подпись и, если есть, над чем. Код команды не показываем.
function liveStepText(call, label, target) {
  const isFile = /^(read_file|write_file|list_files)$/.test(plainName(call.name)) || call.kind === 'edit' || call.kind === 'read';
  if (!target || call.kind === 'execute' || plainName(call.name) === 'run_command') return label;
  const shown = (isFile ? target.split(/[\\/]/).pop() : target).slice(0, 48);
  return !shown || label.includes(shown) ? label : label + ' ' + shown;
}

// Скрипты ботов: название и то, что скрипт делает, показываем в строке шага вместо голого «run_script».
const scriptsInfo = new Map();
const scriptAbout = (name) => scriptsInfo.get(String(name || '').trim().toLowerCase())?.description || '';
function scriptTarget(name) {
  if (!name) return '';
  const about = scriptAbout(name);
  return clip(about ? name + ' — ' + about : name, 160);
}

async function loadScripts() {
  let view;
  try {
    view = await api('/api/optimizer');
  } catch {
    return null;
  }
  S.scriptsList = view.scripts || [];
  S.optimizerOn = view.enabled !== false;
  scriptsInfo.clear();
  for (const s of S.scriptsList) scriptsInfo.set(String(s.name).toLowerCase(), s);
  updateSettingsLinks();
  return view;
}

// Сколько шаг шёл: сервер времени шага не присылает, поэтому считаем на экране от появления шага до его конца.
// У шагов из истории (их не видели живыми) времени нет, и мы его не выдумываем.
const stepClock = new Map();
const stepSpent = new Map();
function stepTrack(id, status) {
  if (!id) return null;
  if (!['ok', 'error', 'denied', 'timeout'].includes(status)) {
    if (!stepClock.has(id)) stepClock.set(id, Date.now());
    return null;
  }
  if (!stepSpent.has(id) && stepClock.has(id)) {
    stepSpent.set(id, Date.now() - stepClock.get(id));
    turnCard.note({ failed: status === 'error' || status === 'timeout' });
  }
  const spent = stepSpent.get(id);
  return spent != null && spent >= 100 ? spent : null;
}

function stepDurNode(ms) {
  const el = document.createElement('span');
  el.className = 't-dur';
  el.textContent = tr('{n} с', { n: (ms / 1000).toLocaleString(window.LOCALE, { maximumFractionDigits: ms < 10_000 ? 1 : 0 }) });
  return el;
}

// Рутина или задача, которую завёл бот: под строкой шага показываем карточку, как в окне «Задачи и автоматизация».
const madeKind = (call) => ({ create_routine: 'routine', add_todo: 'todo' })[plainName(call.name)] || '';

function madeRoutine(args, result) {
  const id = /\brt_[0-9a-f]+/i.exec(result || '')?.[0] || '';
  const known = id ? S.routines.find((x) => x.id === id) : null;
  if (known) return known;
  return {
    id,
    title: String(args.title || ''),
    prompt: String(args.prompt || ''),
    botId: String(args.bot_id || S.currentId || ''),
    at: String(args.at || ''),
    everyMinutes: Number(args.every_minutes) || 0,
    days: args.days || [],
    windowFrom: String(args.window_from || ''),
    windowTo: String(args.window_to || ''),
    enabled: args.enabled !== false,
    quiet: args.quiet !== false,
    hook: false,
  };
}

function madeCard(kind, args, result) {
  const card = document.createElement('div');
  card.className = `made-card made-${kind}`;
  const icon = kind === 'routine'
    ? ic('repeat')
    : ic('todo');
  let title;
  let gist = '';
  let chips = '';
  if (kind === 'routine') {
    const r = madeRoutine(args, result);
    title = r.title;
    ({ gist, chips } = routineParts(r));
  } else {
    title = /«([^»]+)»/.exec(result || '')?.[1] || String(args.title || args.text || '');
    const day = /срок (\d{4}-\d{2}-\d{2})/.exec(result || '')?.[1] || String(args.due_date || '');
    const time = String(args.due_time || '');
    if (day) {
      const d = new Date(`${day}T00:00:00`);
      const label = Number.isNaN(d.getTime()) ? day : d.toLocaleDateString(window.LOCALE, { day: 'numeric', month: 'short' });
      chips = `<span class="rt-chip" title="${tr('Срок')}">${ic('calendar')}${escapeHtml(label)}${time ? ` ${escapeHtml(time)}` : ''}</span>`;
    }
  }
  card.innerHTML = `
    <div class="made-head">
      <span class="made-icon">${icon}</span>
      <span class="made-title" title="${escapeHtml(title)}">${escapeHtml(title)}</span>
      <button type="button" class="ghost made-open">${tr('Открыть')}</button>
    </div>
    ${gist ? `<div class="rt-gist">${escapeHtml(gist)}</div>` : ''}
    ${chips ? `<div class="rt-chips">${chips}</div>` : ''}`;
  $('.made-open', card).addEventListener('click', () => openRoutines(kind === 'routine' ? 'routines' : 'todos'));
  return card;
}

// Карточка встаёт в ленту чата, а не прячется в свёрнутый «Ход работы». Один шаг — одна карточка.
function feedMade(call, result) {
  const kind = call ? madeKind(call) : '';
  if (!kind) return;
  const id = String(call.id || '');
  if (id && messagesEl().querySelector(`.made-card[data-call-id="${CSS.escape(id)}"]`)) return;
  const card = madeCard(kind, call.args || {}, result || call.result);
  card.dataset.callId = id;
  appendInFeed(card);
}

function toolNode(call) {
  const d = document.createElement('details');
  d.className = 'tool';
  d.dataset.status = call.status || 'running';
  d.dataset.callId = call.id || '';
  d.dataset.family = toolFamily(call);
  d._call = call;
  const summary = document.createElement('summary');
  const target = toolTarget(call);
  summary.innerHTML = ICONS.chev + '<span class="t-label" title="' + escapeHtml(clip(call.name, 160)) + '">' + escapeHtml(toolLabel(call)) + '</span>' + (target ? '<span class="t-target" title="' + escapeHtml(target) + '">' + escapeHtml(target) + '</span>' : '') + '<span class="t-status">' + statusLabel(call.status) + '</span>';
  const spent = stepTrack(call.id, call.status);
  if (spent != null) summary.querySelector('.t-status').before(stepDurNode(spent));
  const body = document.createElement('div');
  body.className = 'tool-body';
  // Пустое «{}» не показываем: подробности есть, только если есть аргументы или результат.
  if (call.args && Object.keys(call.args).length) {
    const args = document.createElement('pre');
    args.textContent = JSON.stringify(call.args, null, 2);
    body.append(args);
  }
  if (call.result) {
    const res = document.createElement('pre');
    res.textContent = call.result;
    body.append(res);
  }
  d.append(summary, body);
  // Скрипт, которого нет в списке (его только что построили), подтягиваем и дописываем описание в готовую строку.
  const plain = plainName(call.name);
  if ((plain === 'run_script' || plain === 'save_script') && call.args?.name && !scriptAbout(call.args.name)) {
    loadScripts().then(() => {
      const el = d.querySelector('.t-target');
      const text = scriptTarget(String(call.args.name));
      if (el && text) {
        el.textContent = text;
        el.title = text;
      }
    });
  }
  return d;
}

// Подряд идущие шаги одной семьи сворачиваются в одну строку: «Работает в браузере · 12 шагов · example.com».
function groupNode(family) {
  const g = document.createElement('details');
  g.className = 'tool-grp';
  g.dataset.family = family;
  const summary = document.createElement('summary');
  summary.innerHTML = ICONS.chev + '<span class="t-label"></span><span class="t-target"></span><span class="t-status"></span>';
  const body = document.createElement('div');
  body.className = 'tool-grp-body';
  g.append(summary, body);
  return g;
}

function refreshGroup(g) {
  if (!g) return;
  const leaves = [...g.querySelectorAll('.tool')];
  const n = leaves.length;
  const first = leaves[0];
  const label = g.dataset.family === 'browser' ? tr('Работает в браузере') : first?.querySelector('.t-label')?.textContent || '';
  const hosts = [...g.querySelectorAll('.tool[data-family="browser"] .t-target')].map((el) => el.textContent).filter((s) => /^[\w-]+(\.[\w-]+)+$/.test(s));
  const host = hosts[hosts.length - 1] || '';
  const meta = n + ' ' + plural(n, 'шаг', 'шага', 'шагов') + (host ? ' · ' + host : '');
  const statuses = leaves.map((el) => el.dataset.status);
  const status = ['running', 'waiting', 'error', 'denied', 'timeout'].find((s) => statuses.includes(s)) || 'ok';
  g.dataset.status = status;
  g.querySelector('summary .t-label').textContent = label;
  g.querySelector('summary .t-target').textContent = meta;
  g.querySelector('summary .t-status').textContent = statusLabel(status);
}

// Добавляет шаг в список; если предыдущий шаг той же семьи, они становятся одной группой.
function addTool(tools, node) {
  const last = tools.lastElementChild;
  const family = node.dataset.family;
  if (last && family && last.dataset.family === family) {
    let group = last;
    if (!last.classList.contains('tool-grp')) {
      group = groupNode(family);
      last.replaceWith(group);
      group.querySelector('.tool-grp-body').append(last);
    }
    group.querySelector('.tool-grp-body').append(node);
    refreshGroup(group);
    return;
  }
  tools.append(node);
}

// Ошибка говорит по-человечески, что случилось и что делать. Ответ сервера остаётся ниже как есть.
const ERROR_KINDS = [
  [/\b40[13]\b|unauthori[sz]ed|forbidden|invalid.{0,20}key|api key/i, 'Ключ не подошёл', 'Проверь ключ в настройках модели.'],
  [/\b402\b|balance|insufficient|billing|payment|баланс/i, 'Закончился баланс', 'Пополни счёт у сервиса модели или подключи запасную модель.'],
  [/\b429\b|rate.?limit|too many|usage limit|quota|лимит/i, 'Модель упёрлась в лимит', 'Подожди немного или подключи запасную модель.'],
  [/\b404\b|model.{0,20}not.{0,10}found|no such model/i, 'Такой модели нет', 'Проверь название модели и адрес API.'],
  [/\b5\d\d\b|overloaded|bad gateway|unavailable/i, 'Сервис модели сейчас не отвечает', 'Обычно это ненадолго. Попробуй ещё раз через пару минут.'],
  [/connect|sending request|timed? ?out|dns|network|соедин|сеть/i, 'Нет связи с моделью', 'Проверь интернет и адрес API.'],
];

function explainError(text) {
  const raw = String(text ?? '').replace(/^(Не смог ответить|Couldn't answer):\s*/, '');
  const kind = ERROR_KINDS.find(([pattern]) => pattern.test(raw));
  const shown = trServer(raw);
  return kind ? { title: tr(kind[1]), hint: tr(kind[2]), raw: shown } : { title: tr('Не смог ответить'), hint: '', raw: shown };
}

function fillBubble(bubble, msg) {
  if (!msg.error) {
    bubble.innerHTML = md(msg.text);
    return;
  }
  const error = explainError(msg.text);
  const title = document.createElement('div');
  title.className = 'err-title';
  title.textContent = error.title;
  const hint = document.createElement('div');
  hint.className = 'err-hint';
  hint.textContent = error.hint;
  hint.hidden = !error.hint;
  const detail = document.createElement('div');
  detail.className = 'err-detail';
  detail.textContent = error.raw;
  const fix = document.createElement('button');
  fix.type = 'button';
  fix.className = 'k-btn k-btn--sm';
  fix.dataset.openSettings = '';
  fix.textContent = tr('Настройки модели');
  bubble.replaceChildren(title, hint, detail, fix);
}

// Запуск рутины приходит сообщением «[рутина] Название\nзадание». В чате это маленькая карточка:
// название и первая строка задания; всё задание — по щелчку.
function routineRun(msg) {
  if (msg.role !== 'user') return null;
  const m = /^\[рутина\] ([^\n]*)\n?([\s\S]*)$/.exec(String(msg.text || ''));
  return m ? { title: m[1].trim(), prompt: m[2].trim() } : null;
}

function routineRunCard(msg, run) {
  const card = document.createElement('details');
  card.className = 'rt-runcard';
  const summary = document.createElement('summary');
  const gist = run.prompt.replace(/\s+/g, ' ').slice(0, 160);
  summary.innerHTML = `
    <span class="rt-run-icon">${ic('repeat')}</span>
    <span class="rt-run-text">
      <span class="rt-run-title"><span class="rt-run-kind">${tr('Рутина')}</span>${escapeHtml(run.title)}</span>
      ${gist ? `<span class="rt-run-gist">${escapeHtml(gist)}</span>` : ''}
    </span>
    <span class="rt-run-chev">${ic('chevron-down')}</span>`;
  summary.title = tr('Показать задание целиком');
  card.append(summary);
  if (run.prompt) {
    const bubble = document.createElement('div');
    bubble.className = 'rt-run-body';
    fillBubble(bubble, { ...msg, text: run.prompt });
    card.append(bubble);
  }
  return card;
}

function messageRow(msg) {
  if (msg.role === 'compacted') return compactedRow(msg);
  if (msg.role === 'task') return taskRow(msg);
  const row = document.createElement('div');
  row.className = `msg ${msg.role === 'user' ? 'user' : 'bot'}`;
  row.dataset.id = msg.id || '';
  // Для содержания разговора: время и имена файлов человеческого сообщения.
  if (msg.role === 'user') {
    if (msg.at) row.dataset.at = String(msg.at);
    if (Array.isArray(msg.attachments) && msg.attachments.length) row.dataset.files = msg.attachments.map((f) => f.name).join(', ');
  }

  const col = document.createElement('div');
  col.className = 'msg-col';

  // Задачу от другого бота видно в треде получателя: «от Скаут 2» (след передачи, карточки на неё нет).
  const delegatedFrom = msg.role === 'user' && String(msg.source || '').startsWith('delegation:') ? String(msg.source).slice('delegation:'.length) : '';
  const author = msg.name || (delegatedFrom ? tr('от {name}', { name: delegatedFrom }) : '');
  if (author) {
    const who = document.createElement('div');
    who.className = 'msg-author';
    who.textContent = author;
    col.append(who);
  }

  // Файлы, которые бот сделал и назвал в ответе, показываем в самом чате, а не ссылкой file:///.
  const results = msg.role !== 'user' && !msg.error && msg.text ? Attach.extractResults(msg.text, attachOwner()) : null;
  const shownText = results ? results.text : msg.text;
  const run = routineRun(msg);
  if (run) {
    col.append(routineRunCard(msg, run));
  } else if (shownText && shownText.trim()) {
    const bubble = document.createElement('div');
    bubble.className = `bubble${msg.error ? ' error' : ''}`;
    fillBubble(bubble, { ...msg, text: shownText });
    col.append(bubble);
  }
  if (results?.files.length) col.append(Attach.resultCards(results.files, resultContext()));

  if (Array.isArray(msg.attachments) && msg.attachments.length) col.append(Attach.sent(msg.attachments, attachOwner()));

  const calls = messageCalls(msg);
  if (calls.length) col.append(toolsNode(calls));

  const meta = metaNode(msg.usage, msg.model);
  if (meta) col.append(meta);

  row.append(col);
  return row;
}

// Кнопки карточки результата: кому можно переслать файл и как.
function resultContext() {
  return {
    notify: (text) => toast(text),
    bots: (ownerId) => S.bots.filter((b) => b.id !== ownerId).map((b) => ({ id: b.id, name: b.name })),
    // Файл уезжает в папку выбранного бота, потом бот получает сообщение с путём.
    async send(botId, ref) {
      const res = await fetch(`/api/bots/${encodeURIComponent(ref.owner)}/file/raw?path=${encodeURIComponent(ref.path)}`);
      if (!res.ok) throw new Error(tr('Не удалось прочитать файл'));
      const blob = await res.blob();
      const data = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1] || '');
        reader.onerror = () => reject(new Error(tr('Не удалось прочитать файл')));
        reader.readAsDataURL(blob);
      });
      const saved = await api(`/api/bots/${botId}/attach`, { method: 'POST', body: { name: ref.name, data } });
      await api(`/api/bots/${botId}/messages`, {
        method: 'POST',
        body: { text: tr('Вот файл от другого бота: {name}', { name: ref.name }), attachments: [saved.path] },
      });
    },
  };
}

// Наши инструменты и ход внешнего агента (ACP) показываются одинаково.
function messageCalls(msg) {
  const own = Array.isArray(msg.toolCalls) ? msg.toolCalls : [];
  const agent = Array.isArray(msg.activity) ? msg.activity : [];
  return [...own, ...agent];
}

function toolsNode(calls) {
  const tools = document.createElement('div');
  tools.className = 'tools';
  for (const call of calls) addTool(tools, toolNode(call));
  return tools;
}

// Строка действий под готовым ответом бота: копировать, озвучить, когда написан.
const ACTION_ICONS = { copy: 'copy', check: 'check', speak: 'speak', stop: 'stop-small', branch: 'branch' };

function actionButton(kind, label) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = `msg-act msg-act-${kind}`;
  btn.title = label;
  btn.setAttribute('aria-label', label);
  setActionIcon(btn, kind);
  return btn;
}

function setActionIcon(btn, kind) {
  btn.innerHTML = ic(ACTION_ICONS[kind]);
}

// «1 час назад»; старше недели — дата и время, как везде.
function agoText(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return '';
  const sec = Math.round((Date.now() - n) / 1000);
  if (sec < 45) return tr('только что');
  if (sec >= 7 * 86400) return whenText(n);
  const fmt = new Intl.RelativeTimeFormat(window.LOCALE, { numeric: 'always' });
  if (sec < 3600) return fmt.format(-Math.max(1, Math.round(sec / 60)), 'minute');
  if (sec < 86400) return fmt.format(-Math.round(sec / 3600), 'hour');
  return fmt.format(-Math.round(sec / 86400), 'day');
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement('textarea');
    area.value = text;
    area.style.cssText = 'position:fixed;opacity:0;pointer-events:none';
    document.body.append(area);
    area.select();
    let ok = false;
    try { ok = document.execCommand('copy'); } catch {}
    area.remove();
    return ok;
  }
}

// Озвучка одного ответа за раз: тот же движок, что у голосового разговора; второй щелчок останавливает.
let answerSpeech = null;
function stopAnswerSpeech() {
  if (!answerSpeech) return;
  const { engine, btn } = answerSpeech;
  answerSpeech = null;
  try { engine.cancel(); } catch {}
  btn.classList.remove('on');
  btn.title = tr('Озвучить');
  btn.setAttribute('aria-label', tr('Озвучить'));
  setActionIcon(btn, 'speak');
}

function speakAnswer(btn, text) {
  const wasThis = answerSpeech?.btn === btn;
  stopAnswerSpeech();
  if (wasThis) return;
  const lib = window.DeloistVoiceChat;
  if (!lib) return;
  const engine = lib.pickEngine();
  const clean = (lib.stripForSpeech ? lib.stripForSpeech(text) : text).trim();
  if (!clean) return;
  const mine = { engine, btn };
  answerSpeech = mine;
  btn.classList.add('on');
  btn.title = tr('Остановить');
  btn.setAttribute('aria-label', tr('Остановить'));
  setActionIcon(btn, 'stop');
  engine.speak(clean).then(() => { if (answerSpeech === mine) stopAnswerSpeech(); });
}

// «Ветка отсюда»: копия бота с перепиской до сообщения. Только в личном чате.
function branchAction(msg) {
  const branch = actionButton('branch', tr('Ветка отсюда'));
  branch.addEventListener('click', async () => {
    const botId = S.currentId;
    if (!botId) return;
    branch.disabled = true;
    try {
      const { bot } = await api(`/api/bots/${encodeURIComponent(botId)}/branch`, {
        method: 'POST',
        body: { messageId: msg.id },
      });
      await refreshState();
      selectBot(bot.id);
      toast(tr('Ветка создана'));
    } catch (err) {
      toast(err.message);
    } finally {
      branch.disabled = false;
    }
  });
  return branch;
}

function answerActions(msg, bubble) {
  const bar = document.createElement('div');
  bar.className = 'msg-actions';
  const copy = actionButton('copy', tr('Копировать'));
  copy.addEventListener('click', async () => {
    const ok = await copyText(bubble.innerText.trim());
    const label = ok ? tr('Скопировано') : tr('Не получилось скопировать');
    copy.title = label;
    copy.setAttribute('aria-label', label);
    if (ok) setActionIcon(copy, 'check');
    copy.classList.add('done');
    toast(label, 1400);
    clearTimeout(copy._t);
    copy._t = setTimeout(() => {
      copy.classList.remove('done');
      copy.title = tr('Копировать');
      copy.setAttribute('aria-label', tr('Копировать'));
      setActionIcon(copy, 'copy');
    }, 1600);
  });
  bar.append(copy);
  // Ветка — только в личном чате бота: у группы своя лента и свои участники.
  if (!inRoom() && msg.id) bar.append(branchAction(msg));
  if (window.DeloistVoiceChat && 'speechSynthesis' in window) {
    const speak = actionButton('speak', tr('Озвучить'));
    speak.addEventListener('click', () => speakAnswer(speak, bubble.innerText));
    bar.append(speak);
  }
  const when = agoText(msg.at);
  if (when) {
    const time = document.createElement('span');
    time.className = 'msg-when';
    time.textContent = when;
    time.title = whenText(msg.at);
    // Сколько заняла задача: считается по метке времени вопроса и ответа, у старых тредов без пары — ничего.
    const took = window.DeloistUI.durationText(S.turnMs?.get(msg.id));
    if (took) {
      time.textContent = `${when} · ${took}`;
      time.title = `${whenText(msg.at)} · ${tr('Заняло')} ${took}`;
    }
    bar.append(time);
  }
  return bar;
}

// Готовый ответ бота (не ошибка и не служебная строка) получает строку действий; у последнего она видна всегда.
function answerRow(msg) {
  const row = messageRow(msg);
  const bubble = row.querySelector('.bubble');
  // У вопроса человека нет «Копировать», но ветку от него начать можно.
  if (row.classList.contains('user')) {
    if (!inRoom() && msg.id) {
      const bar = document.createElement('div');
      bar.className = 'msg-actions';
      bar.append(branchAction(msg));
      row.querySelector('.msg-col').append(bar);
    }
    return row;
  }
  if (msg.role !== 'assistant' || msg.error || !msg.text || !bubble) return row;
  row.querySelector('.msg-col').append(answerActions(msg, bubble));
  messagesEl().querySelector('.msg.last-answer')?.classList.remove('last-answer');
  row.classList.add('last-answer');
  return row;
}

// Промежуточный текст с вызовами — часть работы, финальный ответ остаётся снаружи.
function appendThreadMessage(msg) {
  const own = Array.isArray(msg.toolCalls) ? msg.toolCalls : [];
  const activity = Array.isArray(msg.activity) ? msg.activity : [];
  if (msg.role === 'assistant' && own.length) {
    const group = ensureWorkGroup();
    group.querySelector('.work-body').append(messageRow(msg));
    // Действия с возможностью отмены должны оставаться на виду.
    for (const call of own) {
      const learned = skillCall(call);
      if (learned) appendInFeed(skillCard(learned));
      if (call.status === 'ok') feedMade(call);
    }
    if (own.some((call) => ['error', 'denied', 'timeout'].includes(call.status))) markWorkError(group);
    return;
  }
  // ACP может вернуть действия вместе с финальным текстом: разделяем представление, не данные.
  if (msg.role === 'assistant' && activity.length) {
    const group = ensureWorkGroup();
    group.querySelector('.work-body').append(messageRow({ ...msg, id: '', text: '', usage: null }));
    if (activity.some((call) => ['error', 'denied', 'timeout'].includes(call.status))) markWorkError(group);
    msg = { ...msg, activity: [] };
  }
  finishWorkGroup();
  messagesEl().append(answerRow(msg));
}

// ---------------------------------------------------------------------------
// Сжатая история и навыки, которые бот выучил сам
// ---------------------------------------------------------------------------

// Строка о закрытой задаче участника: ведущий узнаёт об итоге, не просыпаясь. Само сообщение хранится в ленте бота.
// Шары подчинённых в строках задач: неподвижные, а ушедшие из ленты освобождаем, чтобы не копились.
const taskOrbs = new Set();
function pruneTaskOrbs() {
  const now = Date.now();
  for (const orb of taskOrbs) {
    if (now - orb.born > 2000 && !orb.avatar.canvas.isConnected) {
      orb.avatar.destroy();
      taskOrbs.delete(orb);
    }
  }
}

function taskAvatar(bot, name) {
  const face = document.createElement('div');
  face.className = 'task-line-avatar';
  face.setAttribute('aria-hidden', 'true');
  pruneTaskOrbs();
  const avatar = bot && window.CrewAvatar ? CrewAvatar.mount(face, { size: 28, still: true, ...lookOf(bot) }) : null;
  if (avatar) {
    taskOrbs.add({ avatar, born: Date.now() });
  } else {
    face.textContent = avatarInitial(bot || { name });
    face.style.background = safeColor(bot?.color);
  }
  return face;
}

// Длинный отчёт свёрнут до нескольких строк: целиком — по кнопке.
const TASK_TEXT_FOLD = 320;

function taskRow(msg) {
  const row = document.createElement('div');
  row.className = `task-line task-line--${msg.state === 'done' ? 'done' : 'failed'}`;
  row.dataset.id = msg.id || '';
  const bot = msg.to ? botById(msg.to) : null;
  const name = msg.toName || bot?.name || '';
  row.append(taskAvatar(bot, name));

  const body = document.createElement('div');
  body.className = 'task-line-body';
  const head = document.createElement('div');
  head.className = 'task-line-head';
  head.textContent = msg.state === 'done'
    ? tr('{name}: задача выполнена', { name })
    : msg.state === 'interrupted'
      ? tr('{name}: задача прервана', { name })
      : tr('{name}: задача не выполнена', { name });
  body.append(head);

  const actions = document.createElement('div');
  actions.className = 'task-line-actions';
  if (msg.text) {
    const text = document.createElement('div');
    text.className = 'task-line-text';
    // Как обычный ответ бота: ссылки на файлы в его папке открываются в ящике «Файлы».
    text.innerHTML = md(msg.text);
    body.append(text);
    if (String(msg.text).length > TASK_TEXT_FOLD) {
      text.classList.add('is-folded');
      const more = document.createElement('button');
      more.type = 'button';
      more.className = 'k-btn k-btn--sm k-btn--ghost';
      more.textContent = tr('Показать полностью');
      more.addEventListener('click', () => {
        const folded = text.classList.toggle('is-folded');
        more.textContent = folded ? tr('Показать полностью') : tr('Свернуть');
      });
      actions.append(more);
    }
  }
  // Файлы результата — ссылками: щелчок открывает файл у подчинённого.
  const files = (Array.isArray(msg.files) ? msg.files : []).filter((path) => window.DeloistUI.workspaceRef(path));
  if (files.length) {
    const list = document.createElement('div');
    list.className = 'task-line-files';
    for (const path of files) {
      const link = document.createElement('a');
      link.href = window.DeloistUI.fileHref(path);
      link.className = 'md-file task-line-file';
      link.dataset.file = path;
      link.title = path;
      link.innerHTML = ic('file-text');
      link.append(String(path).split(/[\\/]/).pop());
      list.append(link);
    }
    body.append(list);
  }
  if (bot) {
    const open = document.createElement('button');
    open.type = 'button';
    open.className = 'k-btn k-btn--sm';
    open.innerHTML = ic('chat-bubble');
    open.append(tr('Открыть чат'));
    open.title = tr('Посмотреть, как {name} сделал задачу', { name });
    open.addEventListener('click', () => selectBot(bot.id));
    actions.append(open);
  }
  if (actions.childElementCount) body.append(actions);
  row.append(body);
  return row;
}

function compactedRow(msg) {
  const row = document.createElement('div');
  row.className = 'compacted';
  row.dataset.id = msg.id || '';
  const line = document.createElement('div');
  line.className = 'compacted-line';
  const label = document.createElement('span');
  label.textContent = tr('Раньше: {n} {word} свёрнуты в сводку', {
    n: msg.dropped || 0,
    word: plural(Number(msg.dropped) || 0, 'сообщение', 'сообщения', 'сообщений'),
  });
  const toggle = document.createElement('button');
  toggle.type = 'button';
  toggle.className = 'compacted-toggle';
  toggle.textContent = tr('Показать сводку');
  const summary = document.createElement('div');
  summary.className = 'compacted-text';
  summary.textContent = msg.text || '';
  summary.hidden = true;
  toggle.addEventListener('click', () => {
    summary.hidden = !summary.hidden;
    toggle.textContent = summary.hidden ? tr('Показать сводку') : tr('Скрыть сводку');
  });
  line.append(label, toggle);
  row.append(line, summary);
  return row;
}

function compactLine(show) {
  $('#compact-line')?.remove();
  if (!show) return;
  const line = document.createElement('div');
  line.id = 'compact-line';
  line.className = 'model-switch';
  line.textContent = tr('Сжимаю историю…');
  messagesEl().append(line);
  scrollBottom();
}

async function reloadThread() {
  try {
    if (inRoom()) {
      const id = S.view.id;
      const { messages } = await api(`/api/rooms/${id}/thread?limit=300`);
      if (S.view.id !== id) return;
      S.roomThreads.set(id, messages);
      renderRoomThread(messages);
    } else if (S.currentId) {
      const id = S.currentId;
      const { messages } = await api(`/api/bots/${id}/thread?limit=300`);
      if (S.currentId !== id) return;
      S.threads.set(id, messages);
      renderThread(messages);
      if (S.busy.has(id)) appendTyping();
    }
  } catch (err) {
    toast(err.message);
  }
}

async function compactCurrent() {
  const room = inRoom();
  const id = room ? S.view.id : S.currentId;
  if (!id) return;
  const btn = $('#btn-compact');
  btn.disabled = true;
  compactLine(true);
  try {
    const { dropped } = await api(`/api/${room ? 'rooms' : 'bots'}/${id}/compact`, { method: 'POST' });
    toast(tr('История сжата: {n} сообщений в сводке', { n: dropped }));
    await reloadThread();
  } catch (err) {
    compactLine(false);
    toast(err.message);
  } finally {
    btn.disabled = false;
  }
}

// После смены модели: сколько истории новая модель прочитает с первого же хода (кэша у неё нет).
// Больше 30% её окна — предлагаем сжать; больше порога сжатия — программа сожмёт сама перед ходом
// (compact::bot_thread), говорим только об этом. Агенты ACP начинают новую сессию со сводкой сами.
const SWITCH_OFFER_SHARE = 0.3;

function modelSwitchAdvice(ctx, acp) {
  if (acp || !ctx) return { kind: 'none' };
  const tokens = Number(ctx.tokens || 0);
  if (Number(ctx.percent || 0) >= 100) return { kind: 'auto', tokens };
  const window = Number(ctx.window || 0);
  if (ctx.windowKnown && window > 0 && tokens / window >= SWITCH_OFFER_SHARE) return { kind: 'offer', tokens, share: Math.round((tokens / window) * 100) };
  return { kind: 'none' };
}

function toastAction(message, label, onClick, ms = 12000) {
  const el = $('#toast');
  el.textContent = message;
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'ghost';
  button.textContent = label;
  button.addEventListener('click', () => {
    el.hidden = true;
    onClick();
  });
  el.append(' ', button);
  el.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => { el.hidden = true; }, ms);
}

// Сжать историю личного чата бота. Пока бот работает, сжатие ждёт конца хода (turn_end).
const compactAfterTurn = new Set();

async function compactBot(id) {
  if (S.busy.has(id)) {
    compactAfterTurn.add(id);
    toast(tr('Сожму историю, как только бот закончит ход.'));
    return;
  }
  try {
    const { dropped } = await api(`/api/bots/${id}/compact`, { method: 'POST' });
    toast(tr('История сжата: {n} сообщений в сводке', { n: dropped }));
    if (!inRoom() && S.currentId === id) await reloadThread();
  } catch (err) {
    toast(err.message);
  }
}

async function afterModelSwitch(id) {
  const busy = S.busy.has(id);
  const later = busy ? tr('Бот сейчас работает. Новая модель включится со следующего сообщения в чате.') + ' ' : '';
  let advice = { kind: 'none' };
  try {
    const stats = await api(`/api/bots/${id}/stats`);
    S.sessionStats.set(id, stats);
    if (S.currentId === id) renderSessionStats();
    const bot = botById(id) || {};
    const driver = bot.driver || S.settings.driver || '';
    advice = modelSwitchAdvice(stats.context, driver === 'acp' || driver.startsWith('acp:'));
  } catch {
    // Без цифр — только про то, когда включится модель.
  }
  if (advice.kind === 'offer') {
    toastAction(later + tr('Новая модель прочитает историю ≈ {n} ток ({p}% её окна).', { n: fmtCount(advice.tokens), p: advice.share }),
      tr('Сжать историю'), () => compactBot(id));
  } else if (advice.kind === 'auto') {
    toast(later + tr('История больше окна новой модели: перед следующим ответом она сожмётся в сводку.'), 7000);
  } else if (busy) {
    toast(later.trim(), 5000);
  }
}

// Навык, который бот сохранил в этом вызове: имя и создан он или исправлен.
function skillCall(call) {
  if (call.name !== 'save_skill' || call.status !== 'ok') return null;
  const name = String(call.args?.name || '').trim();
  const result = String(call.result || '');
  if (!name || !/^Навык «.*»\s+(создан|исправлен)/.test(result)) return null;
  return { name, created: /^Навык «.*»\s+создан/.test(result) };
}

function skillIdByName(name) {
  const key = name.trim().toLowerCase();
  return S.learnedSkills.get(key) || S.skills.find((s) => String(s.name).trim().toLowerCase() === key)?.id || '';
}

// Откат правки навыка: пользователь решает, оставлять ли то, что бот выучил.
async function undoSkill(id) {
  const { result } = await api(`/api/skills/${encodeURIComponent(id)}/undo`, { method: 'POST' });
  toast(result === 'removed' ? tr('Навык удалён') : tr('Вернул прошлую версию навыка'));
  await refreshState();
  renderSkills();
  return result;
}

// Тихая строка после задачи: что программа запомнила, и отмена всей записи журнала.
function memoryCard({ entryId, titles, fixed, profile }) {
  const card = document.createElement('div');
  card.className = 'card skill-card memory-card';
  const title = document.createElement('div');
  title.className = 'card-title';
  const fixedTitles = fixed || [];
  const what = [...(titles || []).filter((t) => !fixedTitles.includes(t)), ...(profile ? [tr('профиль')] : [])];
  const parts = [];
  if (what.length) parts.push(tr('Запомнил: {what}', { what: what.join('; ') }));
  if (fixedTitles.length) parts.push(tr('Исправил: {what}', { what: fixedTitles.join('; ') }));
  title.textContent = parts.join(' · ');
  const actions = document.createElement('div');
  actions.className = 'card-actions';
  const undo = document.createElement('button');
  undo.type = 'button';
  undo.className = 'ghost';
  undo.textContent = tr('Отменить');
  actions.append(undo);
  card.append(title, actions);
  undo.addEventListener('click', async () => {
    undo.disabled = true;
    try {
      await api(`/api/memory/undo/${encodeURIComponent(entryId)}`, { method: 'POST' });
      const done = document.createElement('span');
      done.className = 'resolved';
      done.textContent = tr('Отменено');
      undo.replaceWith(done);
      if (sharedMemoryOpen()) loadSharedMemory();
    } catch (err) {
      undo.disabled = false;
      toast(err.message);
    }
  });
  return card;
}

function skillCard({ name, created }) {
  const card = document.createElement('div');
  card.className = 'card skill-card';
  const title = document.createElement('div');
  title.className = 'card-title';
  title.textContent = created ? tr('Создан навык «{name}»', { name }) : tr('Навык «{name}» исправлен', { name });
  const actions = document.createElement('div');
  actions.className = 'card-actions';
  const undo = document.createElement('button');
  undo.type = 'button';
  undo.className = 'ghost';
  undo.textContent = tr('Отменить');
  actions.append(undo);
  card.append(title, actions);
  undo.addEventListener('click', async () => {
    undo.disabled = true;
    try {
      let id = skillIdByName(name);
      if (!id) {
        await refreshState();
        id = skillIdByName(name);
      }
      if (!id) throw new Error(tr('Не нашёл этот навык: возможно, его уже убрали'));
      await undoSkill(id);
      const done = document.createElement('span');
      done.className = 'resolved';
      done.textContent = tr('Отменено');
      undo.replaceWith(done);
    } catch (err) {
      undo.disabled = false;
      toast(err.message);
    }
  });
  return card;
}

function renderThread(messages) {
  clearMessages();
  if (!messages.length) {
    const bot = botById(S.currentId);
    const demo = (bot?.driver || S.settings.driver) === 'demo';
    messagesEl().innerHTML = `
      <div class="empty">
        <div class="empty-avatar" id="empty-avatar"></div>
        <h2>${escapeHtml(bot?.name || tr('Бот'))}</h2>
        <p>${escapeHtml(bot?.role || '')}</p>
        ${demo ? `<p class="hint">${tr('Сейчас демо-режим: ответы идут по сценарию.')}</p><button type="button" class="ghost" data-open-settings>${tr('Подключить модель')}</button>` : `<p class="hint">${tr('Напиши задачу. Перед опасным действием бот сначала спросит тебя.')}</p>`}
      </div>`;
    const avatar = bot && window.CrewAvatar ? CrewAvatar.mount($('#empty-avatar'), { size: 96, ...lookOf(bot) }) : null;
    if (avatar) S.avatars.set('empty', avatar);
    else $('#empty-avatar').remove();
    if (bot) showIntro(bot.id);
    window.DeloistTeam?.restore();
    return;
  }
  S.turnMs = window.DeloistUI.turnDurations(messages);
  for (const m of messages) appendThreadMessage(m);
  window.DeloistTeam?.restore();
  scrollBottom(true);
}

// Первое приветствие бота: что он умеет прямо сейчас и с чего начать. Запрос один на бота за сессию.
function showIntro(botId) {
  let entry = S.intros.get(botId);
  if (!entry) {
    entry = api(`/api/bots/${encodeURIComponent(botId)}/intro`)
      .then(({ intro }) => {
        S.intros.set(botId, intro || null);
        return intro || null;
      })
      .catch(() => {
        S.intros.set(botId, null);
        return null;
      });
    S.intros.set(botId, entry);
  }
  Promise.resolve(entry).then((intro) => {
    if (!intro || inRoom() || S.currentId !== botId) return;
    // Если в чате уже что-то появилось, приветствие опоздало.
    if (!$('.empty', messagesEl()) || $('.intro', messagesEl())) return;
    const tasks = Array.isArray(intro.tasks) ? intro.tasks.filter((t) => typeof t === 'string' && t.trim()) : [];
    if (!intro.greeting && !tasks.length) return;
    const row = document.createElement('div');
    row.className = 'msg bot intro';
    const col = document.createElement('div');
    col.className = 'msg-col';
    if (intro.greeting) {
      const bubble = document.createElement('div');
      bubble.className = 'bubble';
      bubble.innerHTML = md(intro.greeting);
      col.append(bubble);
    }
    // Карточка входа в Google — выше задач, когда выполняются условия.
    const bot = (S.bots || []).find((b) => b.id === botId);
    const showSignIn = window.DeloistUI?.shouldShowSignIn ? window.DeloistUI.shouldShowSignIn(bot, S.bots || []) : false;
    let signInHost = null;
    let signInHandle = null;
    if (showSignIn && window.DeloistUI?.mountSignInCard) {
      signInHost = document.createElement('div');
      signInHost.className = 'sic-host';
      signInHandle = window.DeloistUI.mountSignInCard(signInHost, {
        onOpenBrowser: async () => {
          await api(`/api/bots/${encodeURIComponent(botId)}/browser/sign-in`, { method: 'POST', body: { site: 'google' } });
        },
        onDone: async () => {
          await api(`/api/bots/${encodeURIComponent(botId)}/sign-in-state`, { method: 'PUT', body: { state: 'done' } });
          if (bot) bot.googleSignIn = 'done';
          try { signInHandle?.destroy(); } catch {}
          signInHost?.remove();
        },
        onLater: async () => {
          await api(`/api/bots/${encodeURIComponent(botId)}/sign-in-state`, { method: 'PUT', body: { state: 'later' } });
          if (bot) bot.googleSignIn = 'later';
          try { signInHandle?.destroy(); } catch {}
          signInHost?.remove();
        },
      });
    }
    if (tasks.length) {
      const chips = document.createElement('div');
      chips.className = 'intro-chips';
      for (const task of tasks.slice(0, 3)) {
        const chip = document.createElement('button');
        chip.type = 'button';
        chip.className = 'chip intro-chip';
        chip.textContent = task;
        chip.addEventListener('click', () => {
          const input = $('#input');
          if (input.disabled) return;
          input.value = task;
          autosize();
          sendMessage();
        });
        chips.append(chip);
      }
      col.append(chips);
    }
    // Покажем карточку выше задач.
    if (signInHost) {
      col.insertBefore(signInHost, col.firstChild);
    }
    row.append(col);
    messagesEl().append(row);
    scrollBottom();
  });
}

// ---------------------------------------------------------------------------
// Группы
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Сцена оркестратора: пока у команды stage = interview, вместо чата — большой шар и разговор пузырями.
// Это тот же чат бота-оркестратора (ведущего команды): события и карточки те же, меняется только вид.
// ---------------------------------------------------------------------------

let orchStage = null; // { roomId, botId, view, api, watch }

const isOrchestrator = (bot) => Boolean(bot?.orchestrator);
// Оркестратор команды, которая ещё собирается; иначе null.
const interviewBot = (room) => {
  if (!room || room.archived || room.stage !== 'interview') return null;
  const lead = botById(room.lead);
  return isOrchestrator(lead) ? lead : null;
};

function closeOrchStage() {
  if (!orchStage) return;
  const { view, watch, botId } = orchStage;
  orchStage = null;
  watch?.disconnect();
  try { view?.destroy(); } catch {}
  S.avatars.delete(`side:${botId}:stage`);
  const host = $('#orch-stage');
  host.hidden = true;
  host.replaceChildren();
  $('#chat').classList.remove('orch-on');
}

// Кнопка «Перестроить команду» в шапке: только у готовой команды с оркестратором во главе.
function paintRebuild() {
  const room = currentRoom();
  const show = Boolean(room && !room.archived && room.stage === 'ready' && isOrchestrator(botById(room.lead)));
  const btn = $('#head-rebuild');
  btn.hidden = !show;
  if (show) {
    btn.textContent = tr('Перестроить команду');
    btn.title = tr('Оркестратор заново расспросит и предложит состав');
  }
}

async function rebuildTeam() {
  const room = currentRoom();
  if (!room) return;
  try {
    await api(`/api/rooms/${encodeURIComponent(room.id)}/rebuild`, { method: 'POST' });
    await refreshState();
  } catch (err) {
    toast(err.message);
  }
}

// «Новая команда»: оркестратор создаётся вместе с ней, ботов заранее иметь не нужно.
async function newOrchestratedTeam() {
  try {
    const { room } = await api('/api/rooms', { method: 'POST', body: { name: tr('Новая команда'), withOrchestrator: true } });
    await refreshState();
    selectRoom(room.id);
  } catch (err) {
    toast(err.message);
  }
}

// Реплика человека (текстом или голосом) уходит оркестратору, как в обычном чате бота.
async function stageSend(botId, text, opts = {}) {
  const body = { text };
  if (opts.voice) body.voice = true;
  await api(`/api/bots/${encodeURIComponent(botId)}/messages`, { method: 'POST', body });
}

async function openOrchStage(room, bot) {
  closeOrchStage();
  const host = $('#orch-stage');
  // Кто уже был в команде: всё, что добавится к моменту «готово», сцена покажет сборкой.
  const entry = { roomId: room.id, botId: bot.id, view: null, api: null, watch: null, known: new Set(room.botIds || []), readied: false };
  orchStage = entry;
  $('#chat').classList.add('orch-on');
  host.hidden = false;
  let history = [];
  try {
    ({ messages: history } = await api(`/api/bots/${encodeURIComponent(bot.id)}/thread?limit=300`));
  } catch (err) {
    toast(err.message);
  }
  if (orchStage !== entry) return;
  // Первая реплика: показываем её сразу, пока сервер ничего не сказал.
  if (!history.some((m) => m.role === 'assistant' && String(m.text || '').trim()) && !S.busy.has(bot.id)) {
    history = [{ role: 'assistant', text: tr('Привет! Расскажи, какую задачу будем решать. Я подберу команду специалистов, назначу им роли и правила.') }, ...history];
  }
  entry.view = window.DeloistUI.mountOrchestratorStage(host, {
    bot: { id: bot.id, name: bot.name, ...lookOf(bot) },
    stage: 'interview',
    history,
    mountOrb: (el, { size }) => {
      const avatar = window.CrewAvatar ? CrewAvatar.mount(el, { size, ...lookOf(bot), skin: 13 }) : null;
      // Слот «side:<бот>:…» — общие настроения от событий хода доходят и до этого шара.
      if (avatar) S.avatars.set(`side:${bot.id}:stage`, avatar);
      return avatar;
    },
    mountBotOrb: (el, { size, bot: fresh }) => (window.CrewAvatar ? CrewAvatar.mount(el, { size, ...lookOf(fresh), skin: fresh.skin ?? 0 }) : null),
    onsend: (text, opts) => stageSend(bot.id, text, opts),
    onready: (stageApi) => { entry.api = stageApi; },
    onleave: () => {
      if (orchStage !== entry) return;
      closeOrchStage();
      selectRoom(room.id);
    },
    onerror: (message) => toast(message),
  });
  // Карточки (вопрос, предложение команды, разрешение) рисует прежний код в скрытую ленту чата: забираем их на сцену.
  entry.watch = new MutationObserver((records) => {
    for (const rec of records) {
      for (const node of rec.addedNodes) {
        if (node.nodeType === 1 && node.matches?.('.card, .msg-card')) entry.api?.addCard(node);
      }
    }
  });
  entry.watch.observe(messagesEl(), { childList: true });
  if (S.busy.has(bot.id)) entry.api?.event({ type: 'turn_start' });
  await showPending(bot.id);
}

// Состояние команды изменилось: сцена появляется (перестроить) или уходит в угол (команда собрана).
function syncOrchStage() {
  if (inRoom()) {
    const room = currentRoom();
    const bot = interviewBot(room);
    if (orchStage && orchStage.roomId === room?.id) {
      if (!bot && !orchStage.readied) {
        orchStage.readied = true;
        const { known } = orchStage;
        const fresh = (room.botIds || [])
          .filter((id) => !known.has(id) && id !== room.lead)
          .map((id) => botById(id))
          .filter(Boolean)
          .map((b) => ({ id: b.id, name: b.name, role: b.role || '', persona: b.persona || '', skin: b.skin ?? 0, color: safeColor(b.color) }));
        orchStage.view?.update({ stage: 'ready', newBots: fresh, rules: room.instructions || '' });
      }
    } else if (bot && !orchStage) {
      selectRoom(room.id);
    }
  }
  paintRebuild();
}

async function selectRoom(id) {
  leaveTeam();
  if (!$('#drawer').hidden) closeDrawer();
  S.queueEdit = null;
  abortQueueDrag();
  S.view = { kind: 'room', id };
  attachFollowView();
  S.currentId = null;
  S.live = null;
  S.roomLive = null;
  S.roomsUnread.delete(id);
  const room = roomById(id);
  if (!room) return;

  // Живой аватар бота занимает ту же ячейку: снимаем его, иначе он вернётся поверх чужого значка.
  S.avatars.get('head')?.destroy();
  S.avatars.delete('head');
  const head = $('#head-avatar');
  head.classList.add('room-icon');
  head.innerHTML = ICONS.group;
  setPanelTitle(tr('Настройки команды: состав, папки, навыки'));
  $('#head-name').textContent = room.name;
  const members = room.botIds.map((b) => botById(b)?.name).filter(Boolean);
  $('#head-role').textContent = members.length ? members.join(', ') : tr('в команде нет ботов');
  $('#head-model').hidden = true;
  const reassign = $('#head-reassign');
  reassign.textContent = tr('Передать…');
  reassign.title = tr('Передать задачу');
  reassign.hidden = Boolean(room.archived) || !members.length;
  closeReassignMenu();
  $('#btn-browser').hidden = true;
  $('#mode-switch').hidden = true;
  $('#session-stats').hidden = true;
  $('#queue').hidden = true;

  const input = $('#input');
  input.disabled = !members.length || Boolean(room.archived);
  input.placeholder = room.archived
    ? tr('Команда в архиве. Верни её, чтобы писать.')
    : members.length
      ? tr('Написать в «{name}» — ответит ведущий или тот, кого позовёшь через @', { name: room.name })
      : tr('Добавь в команду хотя бы одного бота');

  renderSidebar();
  clearMessages();
  paintRebuild();
  // Команда ещё собирается: вместо чата — сцена оркестратора.
  const orchBot = interviewBot(room);
  if (orchBot) {
    await openOrchStage(room, orchBot);
    return;
  }

  const cached = S.roomThreads.get(id);
  if (cached?.length) renderRoomThread(cached);
  try {
    const { messages } = await api(`/api/rooms/${id}/thread?limit=300`);
    if (!inRoom() || S.view.id !== id) return;
    S.roomThreads.set(id, messages);
    renderRoomThread(messages);
  } catch (err) {
    if (!inRoom() || S.view.id !== id) return;
    if (!cached?.length) {
      const note = document.createElement('div');
      note.className = 'empty';
      note.textContent = tr('Не удалось загрузить чат. Закрой окно в трей и открой снова.');
      messagesEl().append(note);
    }
    toast(err.message);
    setSendState();
    return;
  }

  if (S.view.id !== id) return;
  const active = S.roomActive.get(id);
  if (active) await showPending(active.botId, id);
  // Вопросы, которые бот задал в группе и которые ждут после конца хода.
  for (const botId of new Set(needsItems.filter((n) => n.roomId === id).map((n) => n.botId))) {
    if (botId !== active?.botId) await showPending(botId, id);
  }
  if (S.view.id !== id) return;
  if (S.roomsBusy.has(id)) appendTyping();
  setSendState();
  refreshGrantsLine();
  input.focus();
}

function renderRoomThread(messages) {
  clearMessages();
  const room = roomById(S.view.id);
  if (!messages.length) {
    const members = (room?.botIds || []).map((b) => botById(b)?.name).filter(Boolean);
    messagesEl().innerHTML = `
      <div class="empty">
        <h2>${escapeHtml(room?.name || tr('Команда'))}</h2>
        <p>${members.length ? escapeHtml(members.join(', ')) : tr('Пока никого')}</p>
        <p class="hint">${tr('Напиши задачу — ответит ведущий. Позови другого через @имя.')}</p>
      </div>`;
    return;
  }
  S.turnMs = window.DeloistUI.turnDurations(messages);
  for (const m of messages) appendThreadMessage(m);
  window.DeloistTeam?.restore();
  scrollBottom(true);
}

function upsertRoomMessage(msg) {
  const old = messagesEl().querySelector(`[data-id="${CSS.escape(msg.id || '')}"]`);
  // Строка о закрытой задаче участника, выданной из чата команды: служебная строка, не ответ.
  if (msg.role === 'task') {
    if (!old) {
      dropEmpty();
      appendInFeed(taskRow(msg));
      scrollBottom();
    }
    return;
  }
  const row = answerRow(msg);
  if (old) old.replaceWith(row);
  else appendThreadMessage(msg);
  scrollBottom();
}

function roomDelta(ev) {
  removeTyping();
  if (!S.roomLive || S.roomLive.turnId !== ev.turnId) {
    const row = document.createElement('div');
    row.className = 'msg bot';
    const col = document.createElement('div');
    col.className = 'msg-col';
    const speaker = S.roomActive.get(ev.roomId)?.name;
    if (speaker) {
      const who = document.createElement('div');
      who.className = 'msg-author';
      who.textContent = speaker;
      col.append(who);
    }
    const bubble = document.createElement('div');
    bubble.className = 'bubble cursor';
    col.append(bubble);
    row.append(col);
    ensureWorkGroup().querySelector('.work-body').append(row);
    S.roomLive = { turnId: ev.turnId, row, bubble, text: '' };
  }
  S.roomLive.text += ev.text;
  S.roomLive.bubble.textContent = S.roomLive.text;
  scrollBottom();
}

let roomMembersView = null;

function setPanelTitle(text) {
  const panel = $('#btn-panel');
  panel.title = text;
  panel.setAttribute('aria-label', text);
}
let roomResourcesView = null;
let reassignView = null;
let newMenuView = null;

// «+» в шапке списка: меню из двух пунктов, нового бота или новую команду.
function closeNewMenu(refocus = false) {
  if (!newMenuView) return;
  newMenuView.destroy();
  newMenuView = null;
  $('#new-pop').hidden = true;
  $('#btn-new').setAttribute('aria-expanded', 'false');
  if (refocus) $('#btn-new').focus({ preventScroll: true });
}

function toggleNewMenu() {
  if (newMenuView) return closeNewMenu(true);
  $('#new-pop').hidden = false;
  $('#btn-new').setAttribute('aria-expanded', 'true');
  newMenuView = window.DeloistUI.mountMenu($('#new-pop'), {
    items: [
      { id: 'bot', label: tr('Новый бот') },
      { id: 'team', label: tr('Новая команда') },
      { id: 'manual', label: tr('Соберу сам') },
    ],
    label: tr('Создать'),
    onpick: (id) => {
      closeNewMenu();
      if (id === 'bot') return openModal('modal-new');
      if (id === 'manual') {
        if (!S.bots.length) return toast(tr('Сначала создай хотя бы одного бота'));
        return openRoomModal(null);
      }
      newOrchestratedTeam();
    },
    onclose: (focus) => closeNewMenu(focus),
  });
}

// «Передать…» в шапке группы: меню кита со списком участников, как «…» в доке.
function closeReassignMenu(refocus = false) {
  if (!reassignView) return;
  reassignView.destroy();
  reassignView = null;
  $('#head-reassign-pop').hidden = true;
  $('#head-reassign').setAttribute('aria-expanded', 'false');
  if (refocus) $('#head-reassign').focus({ preventScroll: true });
}

function openReassignMenu() {
  const room = currentRoom();
  if (!room || room.archived) return;
  if (reassignView) {
    closeReassignMenu(true);
    return;
  }
  const members = room.botIds.map((id) => botById(id)).filter(Boolean);
  if (!members.length) return;
  $('#head-reassign-pop').hidden = false;
  $('#head-reassign').setAttribute('aria-expanded', 'true');
  reassignView = window.DeloistUI.mountMenu($('#head-reassign-pop'), {
    items: members.map((bot) => ({ id: bot.id, label: bot.name })),
    label: tr('Передать задачу'),
    onpick: pickReassign,
    onclose: (focus) => closeReassignMenu(focus),
  });
}

async function pickReassign(botId) {
  const room = currentRoom();
  closeReassignMenu();
  if (!room) return;
  try {
    await api(`/api/rooms/${room.id}/reassign`, { method: 'POST', body: { botId } });
  } catch (err) {
    toast(err.message);
  }
}

/** Создание или правка группы: состав, ведущий, правила. */
function openRoomModal(room = null, { members: preset = [] } = {}) {
  const form = $('#form-room');
  $('#room-title').textContent = room ? tr('Команда') : tr('Новая команда');
  form.name.value = room?.name || '';
  form.mode.value = room?.mode || 'lead';
  form.instructions.value = room?.instructions || '';
  $('#room-delete').hidden = !room;
  const archive = $('#room-archive');
  archive.hidden = !room;
  archive.textContent = room ? (room.archived ? tr('Вернуть из архива') : tr('В архив')) : '';

  const members = $('#room-members');
  roomMembersView?.destroy();
  roomMembersView = DeloistUI.mountRoomMembers(members, {
    bots: S.bots,
    selected: room ? room.botIds : preset,
    empty: tr('Сначала создай хотя бы одного бота.'),
  });

  // Общие папки и навыки: меняют доступ ботов, сервер проверяет пути при сохранении.
  let resources = { folders: room?.folders || [], skills: room?.skills || [] };
  roomResourcesView?.destroy();
  roomResourcesView = DeloistUI.mountRoomResources($('#room-resources'), {
    folders: resources.folders,
    skills: resources.skills,
    allSkills: S.skills,
    onchange: (next) => (resources = next),
  });

  const lead = $('#room-lead');
  const fillLead = () => {
    const checked = $$('#room-members input:checked').map((el) => el.value);
    const keep = lead.value;
    lead.innerHTML = checked
      .map((id) => `<option value="${id}">${escapeHtml(botById(id)?.name || id)}</option>`)
      .join('');
    if (checked.includes(keep)) lead.value = keep;
    else if (room && checked.includes(room.lead)) lead.value = room.lead;
  };
  fillLead();
  members.onchange = fillLead;

  form.onsubmit = async (e) => {
    e.preventDefault();
    const botIds = $$('#room-members input:checked').map((el) => el.value);
    if (!botIds.length) {
      toast(tr('В команде нужен хотя бы один бот'));
      return;
    }
    const body = {
      name: form.name.value.trim(),
      mode: form.mode.value,
      instructions: form.instructions.value,
      botIds,
      lead: lead.value || botIds[0],
    };
    // Новую команду создаём без папок, потом одним запросом добавляем: так проверка одна и та же.
    const hasResources = resources.folders.length > 0 || resources.skills.length > 0 || Boolean(room);
    if (room) Object.assign(body, resources);
    try {
      let saved;
      try {
        ({ room: saved } = room
          ? await api(`/api/rooms/${room.id}`, { method: 'PATCH', body })
          : await api('/api/rooms', { method: 'POST', body }));
      } catch (err) {
        // В папке лежит сама программа: проект целиком открывается только явным подтверждением человека.
        if (!room || err.code !== 'confirm_program_folder') throw err;
        const sure = confirm(
          tr('В этой папке лежит сама программа. Боты команды смогут менять файлы проекта; данные программы, ключи и автозапуск остаются закрытыми. Разрешить?'),
        );
        if (!sure) throw err;
        ({ room: saved } = await api(`/api/rooms/${room.id}`, { method: 'PATCH', body: { ...body, programFolders: resources.folders } }));
      }
      if (!room && hasResources && (resources.folders.length || resources.skills.length)) {
        await api(`/api/rooms/${saved.id}`, { method: 'PATCH', body: resources });
      }
      $('#modal-room').close();
      await refreshState();
      selectRoom(saved.id);
      toast(room ? tr('Команда сохранена') : tr('Команда «{name}» создана', { name: saved.name }));
    } catch (err) {
      toast(err.message);
    }
  };

  archive.onclick = async () => {
    if (!room) return;
    try {
      const { room: saved } = await api(`/api/rooms/${room.id}`, { method: 'PATCH', body: { archived: !room.archived } });
      $('#modal-room').close();
      await refreshState();
      if (inRoom() && S.view.id === room.id) selectRoom(room.id);
      toast(saved.archived ? tr('Команда в архиве') : tr('Команда возвращена из архива'));
    } catch (err) {
      toast(err.message);
    }
  };

  $('#room-delete').onclick = async () => {
    if (!room) return;
    if (!confirm(tr('Удалить команду «{name}»? Личные чаты ботов останутся.', { name: room.name }))) return;
    try {
      await api(`/api/rooms/${room.id}`, { method: 'DELETE' });
      $('#modal-room').close();
      S.roomThreads.delete(room.id);
      await refreshState();
      showTeam();
      toast(tr('Команда удалена'));
    } catch (err) {
      toast(err.message);
    }
  };

  openModal('modal-room');
}

// Карточка хода (turn-card.js): аватар, имя, «Думает… N с» и «Стоп». Появляется при отправке, до первого
// события сервера; turn_start, дельты и шаги дополняют ту же карточку, а не рисуют вторую.
const turnCard = window.CrewTurnCard.create({
  root: messagesEl,
  mountAvatar: (host, bot) => (window.CrewAvatar ? CrewAvatar.mount(host, { size: 28, ...lookOf(bot) }) : null),
  onStop: () => stopFromCard(),
  labels: {
    thinking: tr('Думает'),
    choosing: tr('Выбираю, кто ответит'),
    stop: tr('Стоп'),
    stopHint: tr('Остановить'),
    failed: tr('Не отправилось'),
    sec: tr('{n} с'),
    min: tr('{m} мин {n} с'),
  },
});

const turnKey = () => (inRoom() ? 'room:' + S.view.id : 'bot:' + S.currentId);

function appendTyping() {
  const room = inRoom();
  const speaker = room ? S.roomActive.get(S.view.id) : null;
  const bot = room ? (speaker?.botId ? botById(speaker.botId) : null) : botById(S.currentId);
  const result = turnCard.begin({
    key: turnKey(),
    bot,
    name: room ? speaker?.name || '' : bot?.name || '',
    phase: room && !speaker?.botId ? 'choosing' : 'thinking',
  });
  scrollBottom();
  return result;
}

// Ход закончился: карточка уходит, а итог (шаги, ошибки, секунды) остаётся строкой под ответом.
function endTurnCard(key) {
  return turnCard.end(key);
}

function removeTyping() {
  // Раньше здесь убиралась строка «печатает»; теперь карточка живёт до конца хода (endTurnCard).
}

function startLive(turnId, step) {
  if (S.live && S.live.turnId === turnId && S.live.step === step) return S.live;
  removeTyping();
  const row = document.createElement('div');
  row.className = 'msg bot';
  const col = document.createElement('div');
  col.className = 'msg-col';
  const bubble = document.createElement('div');
  bubble.className = 'bubble cursor';
  col.append(bubble);
  row.append(col);
  ensureWorkGroup().querySelector('.work-body').append(row);
  S.live = { turnId, step, row, col, bubble, text: '' };
  paintWorkNow('think', tr('Думает'));
  scrollBottom();
  return S.live;
}

// Карточка встала посреди ответа (у внешнего агента весь ход — один шаг): сказанное до неё остаётся
// над карточкой, а дальнейший текст и шаги идут новой строкой в новой группе под ней. Иначе слова
// «человек подтвердил» дописывались в пузырь над вопросом, на который человек ещё не ответил.
function splitLive(oldGroup) {
  const live = S.live;
  if (!live || !oldGroup.contains(live.row) || !workGroup) return;
  live.bubble.classList.remove('cursor');
  const said = live.text.slice(live.offset || 0);
  if (said.trim()) live.bubble.innerHTML = md(said);
  else live.bubble.remove();
  live.col.querySelector('.tools.live')?.classList.remove('live');
  const row = document.createElement('div');
  row.className = 'msg bot';
  const col = document.createElement('div');
  col.className = 'msg-col';
  const bubble = document.createElement('div');
  bubble.className = 'bubble cursor';
  col.append(bubble);
  row.append(col);
  workGroup.querySelector('.work-body').append(row);
  Object.assign(live, { row, col, bubble, offset: live.text.length });
}

function appendDelta(ev) {
  const live = startLive(ev.turnId, ev.step);
  live.text += ev.text;
  live.bubble.textContent = live.text.slice(live.offset || 0);
  paintWorkNow('write', tr('Пишет ответ'));
  scrollBottom();
}

// Строка хода агента появляется сразу и обновляется по мере работы.
function showActivity(ev) {
  const live = startLive(ev.turnId, ev.step);
  let tools = live.col.querySelector('.tools.live');
  if (!tools) {
    tools = document.createElement('div');
    tools.className = 'tools live';
    live.col.insertBefore(tools, live.bubble);
  }
  const call = ev.call || {};
  const target = toolTarget(call);
  const label = toolLabel(call);
  paintWorkNow(workKind(plainName(call.name)), liveStepText(call, label, target));
  const fresh = toolNode(call);
  const old = call.id ? tools.querySelector('.tool[data-call-id="' + CSS.escape(String(call.id)) + '"]') : null;
  if (old) {
    const group = old.closest('.tool-grp');
    fresh.open = old.open;
    old.replaceWith(fresh);
    refreshGroup(group);
  } else {
    addTool(tools, fresh);
  }
  scrollBottom();
}

function finalizeLive(ev) {
  const msg = ev.message;
  const allCalls = messageCalls(msg);
  const live = S.live;

  if (!live || live.turnId !== ev.turnId || live.step !== ev.step) {
    appendThreadMessage(msg);
    scrollBottom();
    return;
  }

  live.bubble.classList.remove('cursor');
  live.col.querySelector('.tools.live')?.remove();
  // Шаги до карточки уже стоят в группе над ней: в последнюю строку — только новые.
  const calls = live.offset
    ? allCalls.filter((c) => !c.id || !messagesEl().querySelector(`.tool[data-call-id="${CSS.escape(String(c.id))}"]`))
    : allCalls;
  // После карточки в последней строке только хвост ответа: начало уже стоит над карточкой.
  const head = live.offset ? live.text.slice(0, live.offset) : '';
  const tail = !head || msg.error ? msg.text
    : String(msg.text || '').startsWith(head) ? String(msg.text).slice(head.length).replace(/^\s+/, '') : live.text.slice(live.offset).trim();
  if (tail) {
    fillBubble(live.bubble, { ...msg, text: tail });
    live.bubble.classList.toggle('error', Boolean(msg.error));
  } else {
    live.bubble.remove();
  }
  if (calls.length) live.col.append(toolsNode(calls));
  const meta = metaNode(msg.usage, msg.model);
  if (meta) live.col.append(meta);
  live.row.dataset.id = msg.id || '';
  const ownCalls = Array.isArray(msg.toolCalls) ? msg.toolCalls : [];
  if (!ownCalls.length) {
    // У внешнего агента действия и итог могут прийти одним сообщением.
    const activityTools = live.col.querySelector('.tools');
    if (activityTools) {
      const activityRow = document.createElement('div');
      activityRow.className = 'msg bot';
      activityRow.append(activityTools);
      ensureWorkGroup().querySelector('.work-body').append(activityRow);
    }
    live.row.remove();
    finishWorkGroup();
    messagesEl().append(live.row);
  }
  S.live = null;
  scrollBottom();
}

// Итог хода: сколько шагов, токенов и какая доля входа пришла из кэша.
function turnStats(ev, card) {
  const usage = ev.usage || {};
  const bits = [];
  const steps = Number(ev.steps || card?.steps || 0);
  if (steps > 0) bits.push(`${steps} ${plural(steps, 'шаг', 'шага', 'шагов')}`);
  const tokens = Number(usage.prompt || 0) + Number(usage.completion || 0);
  if (tokens > 0) bits.push(`${numberRu(tokens)} ${plural(tokens, 'токен', 'токена', 'токенов')}`);
  if (Number(usage.prompt) > 0 && Number(usage.cached) > 0) {
    bits.push(tr('из кэша {n}%', { n: Math.round((Number(usage.cached) / Number(usage.prompt)) * 100) }));
  }
  const ms = Number(ev.ms || card?.ms || 0);
  if (ms >= 1000) {
    bits.push(ms >= 60_000
      ? tr('{n} мин', { n: Math.round(ms / 60_000) })
      : tr('{n} с', { n: (ms / 1000).toLocaleString(window.LOCALE, { maximumFractionDigits: 1 }) }));
  }
  const failed = Number(card?.failed || 0);
  if (!bits.length) return;
  const el = document.createElement('div');
  el.className = 'turn-stats';
  el.textContent = bits.join(' · ');
  if (failed > 0) {
    const bad = document.createElement('span');
    bad.className = 'ts-fail';
    bad.textContent = ' · ' + tr('с ошибкой: {n}', { n: failed });
    el.append(bad);
  }
  messagesEl().append(el);
  scrollBottom();
}

function updateTool(ev) {
  const node = document.querySelector(`.tool[data-call-id="${CSS.escape(ev.callId)}"]`);
  if (!node) return;
  node.dataset.status = ev.status;
  if (['error', 'denied', 'timeout'].includes(ev.status)) markWorkError(node.closest('.work-group'));
  const status = node.querySelector('.t-status');
  if (status) status.textContent = statusLabel(ev.status);
  const spent = stepTrack(ev.callId, ev.status);
  if (spent != null && status && !node.querySelector('.t-dur')) status.before(stepDurNode(spent));
  refreshGroup(node.closest('.tool-grp'));
  const body = node.querySelector('.tool-body');
  if (body && ev.result) {
    const res = document.createElement('pre');
    res.textContent = ev.result;
    body.append(res);
    node.open = ev.status !== 'ok';
  }
  if (ev.status === 'ok') feedMade(node._call, ev.result);
  scrollBottom();
}

function cardShell(html, className = 'card') {
  dropEmpty();
  const card = document.createElement('div');
  card.className = className;
  card.innerHTML = html;
  appendInFeed(card);
  scrollBottom();
  return card;
}

// Показываем то, что будет сделано. Сокращаем, только когда значение одно: разрешение нельзя давать вслепую.
function approvalDetail(detail) {
  try {
    const value = JSON.parse(detail);
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      const keys = Object.keys(value);
      if (keys.length === 1 && typeof value[keys[0]] === 'string') return value[keys[0]];
    }
  } catch {
    // обычный текст показываем как есть
  }
  return detail || '';
}

function setToolStatus(node, status) {
  node.dataset.status = status;
  $('.t-status', node).textContent = statusLabel(status);
  refreshGroup(node.closest('.tool-grp'));
}

// Подтверждения от браузерных инструментов несут русские строки сервера: переводим по строкам, кроме просьбы к человеку.
function approvalBody(ev) {
  const text = approvalDetail(ev.detail);
  const tool = String(ev.tool || '');
  if (!tool.startsWith('browser_') || tool === 'browser_human') return text;
  return text.split('\n').map(trServer).join('\n');
}

// Вид необратимого действия: «всегда» для обычных действий его не открывает.
const KIND_LABELS = {
  pay: () => tr('оплата'),
  send: () => tr('отправка'),
  delete: () => tr('удаление'),
  publish: () => tr('публикация'),
  secure: () => tr('пароли и безопасность'),
};

function placeLabel(place) {
  return !place || place === '*' ? tr('везде') : place;
}

// Строка «Где: …» и вид необратимого действия: уходят в карточку как два текста.
function approvalScope(ev) {
  if (!ev.grantable || !ev.scope) return { scope: '', kind: '' };
  return {
    scope: tr('Где: {place}', { place: placeLabel(ev.scope.place) }),
    kind: KIND_LABELS[ev.scope.kind]?.() || '',
  };
}

// Кнопки решения по порядку: действие, затем «в разговоре» и «всегда», затем отказ.
function approvalActions(ev) {
  if (!ev.grantable) {
    return [
      { answer: 'once', label: ev.allowLabel ? tr(ev.allowLabel) : tr('Разрешить') },
      { answer: 'deny', label: ev.denyLabel ? tr(ev.denyLabel) : tr('Отклонить') },
    ];
  }
  return [
    { answer: 'once', label: tr('Разрешить один раз') },
    ...(ev.conversation ? [{ answer: 'conversation', label: tr('Разрешить в этом разговоре') }] : []),
    ...(ev.always !== false ? [{ answer: 'always', label: tr('Всегда разрешать здесь') }] : []),
    { answer: 'deny', label: tr('Запретить') },
  ];
}

// Карточка — Svelte-компонент ApprovalCard (ui/src/components/chat); оболочка и словарь S.cards остаются здесь.
function approvalCard(ev) {
  if (S.cards.has(ev.id)) return;
  const card = cardShell('', 'msg-card approval-host');
  card.dataset.id = ev.id;
  if (ev.allowLabel) card.dataset.allowLabel = ev.allowLabel;
  if (ev.tool) card.dataset.tool = ev.tool;
  S.cards.set(ev.id, card);
  paintWorkNow('wait', tr('Ждёт твоего решения'));
  // Пока человек решает, шаг не «выполняется», а ждёт его.
  const step = $$('.tool[data-status="running"]', messagesEl()).pop();
  if (step) {
    card.toolStep = step;
    setToolStatus(step, 'waiting');
  }

  // Серверу уходит только ответ: что именно разрешается, он берёт из своей карточки.
  const resolve = async (answer) => {
    card.ui.update({ busy: true });
    try {
      await api(`/api/approvals/${ev.id}`, { method: 'POST', body: { answer } });
      if (answer === 'always') feelBot(ev.botId, 'love', 1.5);
    } catch (err) {
      toast(err.message);
      card.ui.update({ busy: false });
    }
  };
  // Текст собирает программа (describe.rs); слова бота идут отдельной строкой, технические детали свёрнуты.
  const human = ev.human;
  const humanText = human ? (window.LANG === 'en' && human.textEn ? human.textEn : human.text) : '';
  const riskLabels = { low: tr('Риск: низкий'), medium: tr('Риск: средний'), high: tr('Риск: высокий') };
  card.ui = window.DeloistUI.mountApprovalCard(card, {
    title: humanText || `${ev.allowLabel ? '' : tr('Нужно разрешение: ')}${trServer(ev.title || ev.tool)}`,
    what: humanText ? '' : approvalBody(ev),
    ...(humanText
      ? {
          risk: human.risk || '',
          riskLabel: riskLabels[human.risk] || '',
          purpose: human.purpose || '',
          purposeLabel: tr('Бот объясняет:'),
          details: approvalDetail(ev.detail),
          detailsLabel: tr('Показать технические детали'),
        }
      : {}),
    ...approvalScope(ev),
    actions: approvalActions(ev),
    danger: Boolean(ev.danger),
    onanswer: resolve,
  });
}

function questionField(field) {
  const name = escapeHtml(field.name);
  const label = escapeHtml(field.label || field.name);
  if (field.type === 'choice') {
    const options = (field.options || []).map((o) => `<option value="${escapeHtml(o)}">${escapeHtml(o)}</option>`).join('');
    return `<label class="field"><span>${label}</span><select data-field="${name}"><option value=""></option>${options}</select></label>`;
  }
  const type = field.type === 'number' ? 'text" inputmode="decimal' : field.type === 'date' ? 'date' : 'text';
  return `<label class="field"><span>${label}</span><input data-field="${name}" type="${type}" /></label>`;
}

function questionBody(ev) {
  if (ev.form?.length) {
    return `<div class="q-form">${ev.form.map(questionField).join('')}</div>
      <div class="card-actions"><button class="primary" data-send>${tr('Ответить')}</button><button class="ghost" data-dismiss>${tr('Убрать вопрос')}</button></div>`;
  }
  const choices = (ev.choices || []).map((c) => `<button class="ghost" data-choice="${escapeHtml(c)}">${escapeHtml(c)}</button>`).join('');
  return `${choices ? `<div class="card-actions q-choices">${choices}</div>` : ''}
    <div class="row">
      <input class="q-input" placeholder="${choices ? tr('Или свой ответ…') : tr('Ответ…')}" />
      <button class="primary" data-send>${tr('Ответить')}</button>
      <button class="ghost" data-dismiss title="${tr('Убрать вопрос')}" aria-label="${tr('Убрать вопрос')}">×</button>
    </div>`;
}

function questionCard(ev) {
  if (S.cards.has(ev.id)) return;
  const card = cardShell(
    `
    <div class="card-title">${tr('Бот спрашивает')}</div>
    <div style="margin-bottom:8px">${md(ev.question)}</div>
    ${questionBody(ev)}`,
    'card question',
  );
  card.dataset.id = ev.id;
  S.cards.set(ev.id, card);

  const lock = (on) => $$('button, input, select', card).forEach((el) => { el.disabled = on; });
  const post = async (body) => {
    lock(true);
    try {
      await api(`/api/questions/${ev.id}`, { method: 'POST', body });
    } catch (err) {
      toast(err.message);
      lock(false);
    }
  };
  const send = () => {
    if (ev.form?.length) {
      const values = {};
      $$('[data-field]', card).forEach((el) => { values[el.dataset.field] = el.value; });
      post({ values });
      return;
    }
    const answer = $('.q-input', card).value.trim();
    if (answer) post({ answer });
  };
  $('[data-send]', card).addEventListener('click', send);
  $$('[data-choice]', card).forEach((btn) => btn.addEventListener('click', () => post({ answer: btn.dataset.choice })));
  $('[data-dismiss]', card)?.addEventListener('click', async () => {
    lock(true);
    try {
      await api(`/api/questions/${ev.id}`, { method: 'DELETE' });
    } catch (err) {
      toast(err.message);
      lock(false);
    }
  });
  $('.q-input', card)?.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); send(); }
  });
}

function specialistOfferCard(ev) {
  const offer = ev.offer || {};
  if (!offer.id || S.cards.has(offer.id)) return;
  const name = escapeHtml(offer.specialistName || '');
  const reason = String(offer.reason || '');
  const task = String(offer.task || '');
  const taskHtml = task.length > 160
    ? `<details><summary>${tr('Задача')}</summary><p class="note">${escapeHtml(task)}</p></details>`
    : task ? `<p class="note">${escapeHtml(task)}</p>` : '';
  const card = cardShell(
    `<div class="card-title">${tr('Подключу специалиста: {name}', { name })}</div>
    ${reason ? `<p class="note">${escapeHtml(reason)}</p>` : ''}
    ${taskHtml}
    <div class="card-actions">
      <button class="primary" data-accept>${tr('Передать')}</button>
      <button class="ghost" data-decline>${tr('Сам справлюсь')}</button>
    </div>`,
    'card offer',
  );
  card.dataset.id = offer.id;
  S.cards.set(offer.id, card);

  const finish = () => {
    card.remove();
    S.cards.delete(offer.id);
    S.waiting.get(ev.botId)?.delete(`offer:${offer.id}`);
    syncAvatar(ev.botId);
  };
  // Решение уходит на сервер; карточка гаснет только после ответа, чтобы отказ сети был виден.
  const resolve = async (action) => {
    $$('button', card).forEach((btn) => { btn.disabled = true; });
    try {
      await api(`/api/bots/${encodeURIComponent(ev.botId)}/offers/${encodeURIComponent(offer.id)}/${action}`, { method: 'POST' });
      finish();
    } catch (err) {
      // Предложение уже решили в другом окне: карточка больше не актуальна.
      if (err.status === 404) {
        finish();
        return;
      }
      toast(err.message);
      $$('button', card).forEach((btn) => { btn.disabled = false; });
    }
  };
  $('[data-accept]', card).addEventListener('click', () => resolve('accept'));
  $('[data-decline]', card).addEventListener('click', () => resolve('decline'));
}

// Карточка предложения: «Применить / Не надо» уходит на сервер, карточка гаснет после ответа.
function mountProposalCard(ev, proposal, html, cls) {
  const card = cardShell(html, cls);
  card.dataset.id = proposal.id;
  S.cards.set(proposal.id, card);

  const finish = () => {
    card.remove();
    S.cards.delete(proposal.id);
    S.waiting.get(ev.botId)?.delete(`offer:${proposal.id}`);
    syncAvatar(ev.botId);
  };
  const resolve = async (action) => {
    $$('button', card).forEach((btn) => { btn.disabled = true; });
    try {
      await api(`/api/bots/${encodeURIComponent(ev.botId)}/offers/${encodeURIComponent(proposal.id)}/${action}`, { method: 'POST' });
      finish();
    } catch (err) {
      if (err.status === 404) {
        finish();
        return;
      }
      toast(err.message);
      $$('button', card).forEach((btn) => { btn.disabled = false; });
    }
  };
  $('[data-accept]', card).addEventListener('click', () => resolve('accept'));
  $('[data-decline]', card).addEventListener('click', () => resolve('decline'));
}

// Было → стало: короткий простой текст без разметки, а весь текст раскрывается; всё экранируется.
function beforeAfterBlock(title, edit, long) {
  const flat = (text) => {
    const plain = window.DeloistUI.plainPreview(String(text || ''));
    return plain.length > 200 ? `${plain.slice(0, 200)}…` : plain;
  };
  // Полный текст — только если он не умещается в превью (иначе в нём нет ничего нового).
  const whole = (label, text) => (long && String(text || '').trim() && String(text).trim() !== flat(text)
    ? `<details class="team-full"><summary>${label}</summary><pre class="team-full-text">${escapeHtml(String(text))}</pre></details>`
    : '');
  const was = flat(edit.before) || tr('пусто');
  return `<div class="note team-change-title">${title}</div>
    <div class="team-change">
      <div class="note team-before"><span class="team-tag">${tr('Было')}</span> ${escapeHtml(was)}</div>
      ${whole(tr('Было целиком'), edit.before)}
      <div class="note team-after"><span class="team-tag">${tr('Станет')}</span> ${escapeHtml(flat(edit.after))}</div>
      ${whole(tr('Станет целиком'), edit.after)}
    </div>`;
}

// Бот предлагает поправить участника команды (propose_bot_change): было → стало, права не трогаются.
function botChangeCard(ev, proposal) {
  const target = escapeHtml(proposal.target?.name || '');
  const parts = [];
  if (proposal.role) parts.push(beforeAfterBlock(tr('Роль'), proposal.role, false));
  if (proposal.instructions) parts.push(beforeAfterBlock(tr('Инструкции'), proposal.instructions, true));
  if (proposal.removeFrom) {
    const lead = proposal.removeFrom.newLead
      ? ` ${tr('Ведущим станет {name}.', { name: escapeHtml(proposal.removeFrom.newLead) })}`
      : '';
    parts.push(`<p class="note team-warn">${tr('Уберу из команды «{room}»', { room: escapeHtml(proposal.removeFrom.name) })}.${lead}</p>`);
  }
  mountProposalCard(ev, proposal, `<div class="card-title">${tr('Предлагаю изменить: {name}', { name: target })}</div>
    ${parts.join('')}
    <p class="note">${tr('Режим подтверждений, сайты, модель, папки и лимиты не меняются.')}</p>
    <div class="card-actions">
      <button class="primary" data-accept>${tr('Применить')}</button>
      <button class="ghost" data-decline>${tr('Не надо')}</button>
    </div>`, 'card offer team-proposal team-change-card');
}

// Бот предлагает расписание участнику (propose_routine): кому, когда, что делать; «Свобода действий» — отдельной строкой.
function routineProposalCard(ev, proposal) {
  const target = escapeHtml(proposal.target?.name || '');
  const prompt = String(proposal.prompt || '');
  const flat = window.DeloistUI.plainPreview(prompt);
  const short = flat.length > 240 ? `${flat.slice(0, 240)}…` : flat;
  const whole = prompt.trim() && prompt !== short
    ? `<details class="team-full"><summary>${tr('Весь текст задачи')}</summary><pre class="team-full-text">${escapeHtml(prompt)}</pre></details>`
    : '';
  const free = proposal.freeMode
    ? `<p class="note team-warn team-free">⚠ ${tr('У «{name}» включена «Свобода действий»: рутина выполнит всё без вопросов.', { name: target })}</p>`
    : '';
  const quiet = proposal.quiet
    ? `<p class="note team-quiet">${tr('Тихая рутина: запуски не попадают в историю бота, человек видит только итог.')}</p>`
    : '';
  mountProposalCard(ev, proposal, `<div class="card-title">${tr('Предлагаю рутину для: {name}', { name: target })}</div>
    <p class="team-routine-title"><b>${escapeHtml(proposal.title || '')}</b></p>
    <p class="note">${tr('Когда')}: ${escapeHtml(proposal.schedule || '')}. ${escapeHtml(proposal.next || '')}</p>
    ${short ? `<div class="note">${escapeHtml(short)}</div>` : ''}
    ${whole}
    ${free}
    ${quiet}
    <div class="card-actions">
      <button class="primary" data-accept>${tr('Создать')}</button>
      <button class="ghost" data-decline>${tr('Не надо')}</button>
    </div>`, 'card offer team-proposal team-routine-card');
}

// Бот предлагает собрать команду (propose_team): показываем ровно то, что создадим, и доступ, который получат новые участники.
function teamProposalCard(ev) {
  const proposal = ev.proposal || {};
  if (!proposal.id || S.cards.has(proposal.id)) return;
  if (proposal.kind === 'bot_change') return botChangeCard(ev, proposal);
  if (proposal.kind === 'routine') return routineProposalCard(ev, proposal);
  const list = (items) => (Array.isArray(items) ? items : []);
  const bot = (name, role, extra = '') => `<li><b>${escapeHtml(name)}</b>${role ? ` — ${escapeHtml(role)}` : ''}${extra}</li>`;
  const fresh = list(proposal.newBots).map((b) => {
    const full = String(b.instructions || '').trim();
    // Превью — простой текст без разметки; весь текст инструкций на один щелчок ниже.
    const flat = window.DeloistUI.plainPreview(full);
    const short = flat.length > 240 ? `${flat.slice(0, 240)}…` : flat;
    const preview = short ? `<div class="note">${escapeHtml(short)}</div>` : '';
    const whole = full && full !== short
      ? `<details class="team-full"><summary>${tr('Все инструкции')}</summary><pre class="team-full-text">${escapeHtml(full)}</pre></details>`
      : '';
    return bot(b.name, b.role, preview + whole);
  });
  const fromCatalog = list(proposal.catalogBots).map((b) => {
    const needs = list(b.needs).map((n) => escapeHtml(n)).join('; ');
    const state = b.installed ? tr('уже установлен') : '';
    const extra = [needs ? `${tr('нужно')}: ${needs}` : '', state].filter(Boolean).join('. ');
    return bot(`${tr('из каталога')}: ${b.name}`, b.role, extra ? `<div class="note">${extra}</div>` : '');
  });
  const existing = list(proposal.addBots).map((b) => bot(b.name, ''));
  const room = proposal.room;
  const roomLine = room
    ? `<p class="note">${room.isNew ? tr('Новая команда: {name}', { name: escapeHtml(room.name) }) : tr('В команду: {name}', { name: escapeHtml(room.name) })}${proposal.lead ? ` · ${tr('ведёт: {name}', { name: escapeHtml(proposal.lead) })}` : ''}</p>`
    : '';
  const defaults = proposal.defaults || {};
  const makes = list(proposal.newBots).length + list(proposal.catalogBots).filter((b) => !b.installed).length;
  const defaultsLine = makes && defaults.approval
    ? `<p class="note team-defaults">${tr('Новые боты: режим «спрашивать», как у «{host}»: драйвер {driver}, модель {model}', {
      host: escapeHtml(defaults.hostName || ''),
      driver: escapeHtml(defaults.driver || tr('по умолчанию')),
      model: escapeHtml(defaults.model || tr('по умолчанию')),
    })}</p>`
    : '';
  const warn = list(proposal.accessWarn);
  const access = list(proposal.access).map((line, i) => (warn[i]
    ? `<p class="note team-access team-warn">⚠ ${escapeHtml(line)}</p>`
    : `<p class="note team-access">${escapeHtml(line)}</p>`)).join('');
  const section = (title, items) => (items.length ? `<div class="note">${title}</div><ul class="team-list">${items.join('')}</ul>` : '');
  const card = cardShell(
    `<div class="card-title">${tr('Предлагаю собрать команду')}</div>
    ${section(tr('Создам ботов'), fresh)}
    ${section(tr('Поставлю из каталога'), fromCatalog)}
    ${section(tr('Добавлю существующих'), existing)}
    ${roomLine}
    ${access}
    ${defaultsLine}
    <div class="card-actions">
      <button class="primary" data-accept>${tr('Создать')}</button>
      <button class="ghost" data-decline>${tr('Не надо')}</button>
    </div>`,
    'card offer team-proposal',
  );
  card.dataset.id = proposal.id;
  S.cards.set(proposal.id, card);

  const finish = () => {
    card.remove();
    S.cards.delete(proposal.id);
    S.waiting.get(ev.botId)?.delete(`offer:${proposal.id}`);
    syncAvatar(ev.botId);
  };
  const resolve = async (action) => {
    $$('button', card).forEach((btn) => { btn.disabled = true; });
    try {
      await api(`/api/bots/${encodeURIComponent(ev.botId)}/offers/${encodeURIComponent(proposal.id)}/${action}`, { method: 'POST' });
      finish();
    } catch (err) {
      if (err.status === 404) {
        finish();
        return;
      }
      toast(err.message);
      $$('button', card).forEach((btn) => { btn.disabled = false; });
    }
  };
  $('[data-accept]', card).addEventListener('click', () => resolve('accept'));
  $('[data-decline]', card).addEventListener('click', () => resolve('decline'));
}

// Бот-оптимизатор нашёл в журнале механику и спрашивает: нужен ли скрипт. Только вопрос: ничего не запускается.
function optimizerOfferCard(ev) {
  const offer = ev.offer || {};
  if (!offer.id || S.cards.has(offer.id)) return;
  const sig = escapeHtml(String(offer.sig || ''));
  const example = String(offer.example || '');
  const repeat = Number(offer.runs) > 1 ? `<p class="note">${tr('Такая работа повторяется уже в {runs} разговорах.', { runs: Number(offer.runs) })}</p>` : '';
  const card = cardShell(
    `<div class="card-title">${tr('Эту работу можно ускорить скриптом')}</div>
    <p class="note">${tr('Бот сделал {steps} однотипных шагов «{sig}» за один раз. Скрипт сделает это примерно в 10 раз быстрее: ИИ останется решать, что искать и что сказать.', { steps: Number(offer.steps) || 0, sig })}</p>
    ${repeat}
    ${example ? `<p class="note">${tr('Просьба')}: ${escapeHtml(example)}</p>` : ''}
    <div class="card-actions">
      <button class="primary" data-accept>${tr('Написать скрипт')}</button>
      <button class="ghost" data-decline>${tr('Разовая работа')}</button>
    </div>`,
    'card offer',
  );
  card.dataset.id = offer.id;
  S.cards.set(offer.id, card);
  const finish = () => {
    card.remove();
    S.cards.delete(offer.id);
  };
  const resolve = async (action) => {
    $$('button', card).forEach((btn) => { btn.disabled = true; });
    try {
      await api(`/api/optimizer/proposals/${encodeURIComponent(offer.id)}/${action}`, { method: 'POST' });
      finish();
    } catch (err) {
      if (err.status === 404) {
        finish();
        return;
      }
      toast(err.message);
      $$('button', card).forEach((btn) => { btn.disabled = false; });
    }
  };
  $('[data-accept]', card).addEventListener('click', () => resolve('accept'));
  $('[data-decline]', card).addEventListener('click', () => resolve('decline'));
}

// Предложения, которые пришли, пока окно было закрыто.
async function loadOptimizerOffers(botId) {
  try {
    const { offers } = await api(`/api/bots/${encodeURIComponent(botId)}/optimizer`);
    if (S.currentId !== botId || inRoom()) return;
    for (const offer of offers || []) optimizerOfferCard({ botId, offer });
  } catch { /* предложения не обязательны: чат работает и без них */ }
}

function closeCard(ev) {
  const card = S.cards.get(ev.id);
  if (!card) return;
  if (card.toolStep?.dataset.status === 'waiting') setToolStatus(card.toolStep, 'running');
  const handoff = card.dataset.tool === 'remember'
    ? { allowed: tr('запомнил'), denied: tr('не стал') }
    : card.dataset.allowLabel ? { allowed: tr('готово'), denied: tr('не стал') } : {};
  const label = handoff[ev.status] || { allowed: tr('разрешено'), denied: tr('отклонено'), timeout: tr('истекло время'), answered: tr('ответ отправлен'), turn_stopped: tr('ход остановлен'), dismissed: tr('вопрос убран') }[ev.status] || ev.status;
  if (card.ui) {
    card.ui.update({ resolved: label, busy: false });
    paintWorkNow('think', tr('Думает'));
  } else {
    $$('button, input, select', card).forEach((el) => { el.disabled = true; });
    $$('.card-actions, .row', card).forEach((el) => el.remove());
    card.insertAdjacentHTML('beforeend', `<div class="card-actions"><span class="resolved">${escapeHtml(label)}</span></div>`);
  }
  S.cards.delete(ev.id);
}

// В шапке та модель, что отвечает прямо сейчас: на время хода это может быть запасная.
function paintHeadModel(bot) {
  if (!bot) return;
  const head = $('#head-model');
  const spare = S.onSpare.get(bot.id);
  const driver = bot.driver || S.settings.driver;
  // Своя модель бота берётся только вместе со своим провайдером: модель из настроек принадлежит
  // другому провайдеру, и шапка с ней врала (бот на Anthropic показывал gpt-6.1-terra, 12.11.2026).
  const model = bot.driver ? (bot.model || bot.driver) : (S.settings.model || '');
  head.hidden = false;
  if (spare) head.textContent = tr('На запасной: {name}', { name: spare });
  else if (driver === 'demo') head.textContent = tr('Демо-режим');
  else if (driver === 'cli') head.textContent = model || tr('Программа на компьютере');
  else head.textContent = model || tr('Модель не выбрана');
  // Усилие бота: «модель · Низкое» (имя берём из списка провайдера, иначе из low/medium/high).
  if (!spare && bot.effort && driver !== 'demo') head.textContent += ` · ${effortName(driver, bot.effort)}`;
  if (bot.effort && !providerEffortsLoaded) {
    providerEffortsLoaded = true;
    DeloistProviders.choices().then((data) => rememberEfforts(data.providers)).catch(() => {});
  }
}

const EFFORT_NAMES = { low: () => tr('Низкое'), medium: () => tr('Среднее'), high: () => tr('Высокое') };
let providerEfforts = new Map();
let providerEffortsLoaded = false;

function effortName(driver, id) {
  const hit = providerEfforts.get(driver)?.find((e) => e.id === id);
  return hit?.name || (EFFORT_NAMES[id] ? EFFORT_NAMES[id]() : id);
}

// Плашка модели в шапке: один клик открывает выбор модели и усилия прямо тут, для этого бота.
// Сохраняет тем же PATCH бота, что и профиль; «Все настройки модели…» открывает прежнее окно.
let headModelPopover = null;

function rememberEfforts(list) {
  providerEfforts = new Map((list || []).filter((p) => p.driver && Array.isArray(p.efforts)).map((p) => [p.driver, p.efforts]));
  const bot = botById(S.currentId);
  if (bot) paintHeadModel(bot);
}

function closeHeadModelPopover(focusBack = false) {
  if (!headModelPopover) return;
  const { handle, host } = headModelPopover;
  headModelPopover = null;
  handle.destroy();
  host.remove();
  if (focusBack) $('#head-model')?.focus();
}

async function openHeadModelPopover() {
  const chip = $('#head-model');
  const bot = botById(S.currentId);
  if (!chip || !bot || inRoom()) return openSettings('model');
  if (headModelPopover) return closeHeadModelPopover(true);
  const host = document.createElement('div');
  document.body.append(host);
  const model = S.settings.model || S.settings.driver || '';
  const props = () => {
    const now = botById(bot.id) || bot;
    return {
      value: { driver: now.driver || '', model: now.model || '' },
      effort: now.effort || '',
      agentOptions: now.agentOptions || {},
      settings: S.settings,
      defaultLabel: tr('Как в настройках: {model}', { model: model || tr('Модель не выбрана') }),
    };
  };
  const save = async (patch) => {
    try {
      await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: patch });
      await refreshState();
      paintHeadModel(botById(bot.id));
      if (headModelPopover?.handle === handle && S.currentId === bot.id) handle.update(props());
      return true;
    } catch (err) {
      toast(err.message);
      return false;
    }
  };
  const handle = DeloistUI.mountChatModelPopover(host, {
    ...(DeloistProviders.snapshot() || { providers: [], plan: null }),
    loading: !DeloistProviders.snapshot(),
    ...props(),
    anchor: chip,
    onselect: async (next) => {
      // Конкретная модель главнее уровня: уровень возвращаем в «Авто», как в профиле.
      if (await save(next.model ? { driver: next.driver, model: next.model, modelLevel: '' } : { driver: next.driver, model: next.model })) {
        afterModelSwitch(bot.id);
      }
    },
    oneffort: async (effort) => {
      // Идущий ход доработает с прежним усилием: новое включится со следующего сообщения.
      if (await save({ effort }) && S.busy.has(bot.id)) toast(tr('Бот сейчас работает. Новое усилие включится со следующего сообщения в чате.'), 5000);
    },
    onoption: (id, valueId) => {
      const next = { ...(botById(bot.id)?.agentOptions || {}) };
      if (valueId) next[id] = valueId;
      else delete next[id];
      save({ agentOptions: next });
    },
    onconnect: () => DeloistProviders.open(),
    onmore: () => {
      closeHeadModelPopover();
      openSettings('model');
    },
    onclose: (focusBack) => queueMicrotask(() => {
      if (headModelPopover?.handle === handle) closeHeadModelPopover(focusBack);
    }),
  });
  headModelPopover = { handle, host };
  try {
    const data = await DeloistProviders.choices();
    rememberEfforts(data.providers);
    if (headModelPopover?.handle === handle && S.currentId === bot.id) handle.update({ providers: data.providers, plan: data.plan, loading: false, ...props() });
  } catch {
    if (headModelPopover?.handle === handle) handle.update({ loading: false });
  }
}

function modelLine(text, title = '') {
  dropEmpty();
  const line = document.createElement('div');
  line.className = 'model-switch';
  line.textContent = text;
  if (title) line.title = title;
  appendInFeed(line);
  scrollBottom();
}

function switchedLine(ev) {
  modelLine(tr('Основная модель недоступна, продолжаю на {name}', { name: ev.to }), ev.reason);
}

function delegateLine(ev, done = false) {
  if (done) {
    const last = messagesEl().lastElementChild;
    if (last?.dataset.delegate === String(ev.to)) {
      const tokens = Number(ev.tokens);
      last.textContent = tokens > 0
        ? tr('{name} ответил · {n} тыс. токенов', { name: ev.target, n: (tokens / 1000).toLocaleString(window.LOCALE, { maximumFractionDigits: 1 }) })
        : tr('{name} ответил', { name: ev.target });
      last.dataset.delegate = '';
      return;
    }
  }
  const card = cardShell(tr('Передал задачу: {name}', { name: `<b>${escapeHtml(ev.target)}</b>` }), 'card delegate');
  card.dataset.delegate = String(ev.to);
}

// Панель «Команда» (team.js): дерево ведущего и помощников вместо отдельных строк о передаче.
window.DeloistTeam?.init({
  bot: botById,
  isHere: (id) => (inRoom() ? S.roomActive.get(S.view.id)?.botId === id : S.currentId === id),
  place: (node) => {
    dropEmpty();
    appendInFeed(node);
    scrollBottom();
  },
  stop: (id) => api(`/api/bots/${encodeURIComponent(id)}/stop`, { method: 'POST' }),
  openBot: (id) => selectBot(id),
  refreshTeam: () => scheduleTeam(),
  toolLabel: (name) => TOOL_LABELS[plainName(name)] || TOOL_LABELS[name] || plainName(name),
});

// ---------------------------------------------------------------------------
// События
// ---------------------------------------------------------------------------

// Шаги идущего хода приходят только событиями, а в тред сервер пишет их в конце хода.
// Поэтому держим их по боту, чтобы, вернувшись в его чат, снова показать «Ход работы» целиком.
const turnReplay = new Map();

function keepForReplay(ev) {
  if (!ev.botId || ev.roomId) return;
  if (ev.type === 'turn_start') {
    turnReplay.set(ev.botId, []);
    return;
  }
  if (ev.type === 'turn_end') {
    turnReplay.delete(ev.botId);
    return;
  }
  const kept = turnReplay.get(ev.botId);
  if (!kept) return;
  // Готовое сообщение уже лежит в треде: его дельты повторять не нужно.
  if (ev.type === 'message' && ev.message?.role !== 'user') {
    turnReplay.set(ev.botId, kept.filter((e) => e.type !== 'text_delta'));
    return;
  }
  if (ev.type === 'tool_update' || ev.type === 'activity' || ev.type === 'text_delta') kept.push(ev);
}

function replayTurn(id) {
  for (const ev of turnReplay.get(id) || []) {
    if (ev.type === 'tool_update') updateTool(ev);
    else if (ev.type === 'activity') showActivity(ev);
    else appendDelta(ev);
  }
}

function handleEvent(ev) {
  keepForReplay(ev);
  // Экран бота (screen.js) слушает те же события.
  window.dispatchEvent(new CustomEvent('deloist:event', { detail: ev }));
  window.DeloistTeam?.onEvent(ev);
  if (ev.type === 'provider') {
    DeloistProviders.onEvent(ev);
    return;
  }
  // Идёт сцена оркестратора: его разговор и карточки показывает она.
  if (orchStage && ev.botId === orchStage.botId && !ev.roomId) {
    orchStage.api?.event(ev);
    if (ev.type === 'question') questionCard(ev);
    else if (ev.type === 'approval') approvalCard(ev);
    else if (ev.type === 'team_proposal') teamProposalCard(ev);
  }
  const roomMine = inRoom() && ev.roomId === S.view.id;
  const mine = !inRoom() && ev.botId === S.currentId;
  // Отвечающий в открытой группе бот спрашивает разрешение прямо там: иначе ход висел бы без ответа.
  const roomCard = inRoom() && Boolean(ev.botId) && S.roomActive.get(S.view.id)?.botId === ev.botId;
  trackMood(ev);
  onPermissionEvent(ev);
  if ((mine || roomMine) && (ev.type === 'message' || ev.type === 'text_delta' || ev.type === 'turn_start' || ev.type === 'room_message' || ev.type === 'room_delta')) dropEmpty();

  // События групп: свой тред, свой busy, свой останов.
  switch (ev.type) {
    case 'compacting':
    case 'compacted': {
      const here = ev.roomId ? roomMine : mine;
      if (here) {
        if (ev.type === 'compacting') compactLine(true);
        else reloadThread();
      }
      return;
    }

    case 'skill_learned':
      if (ev.name && ev.skillId) S.learnedSkills.set(String(ev.name).trim().toLowerCase(), ev.skillId);
      if ((mine || roomCard) && ev.name) appendInFeed(skillCard({ name: ev.name, created: Boolean(ev.created) }));
      refreshState().then(() => { if ($('#modal-settings').open) renderSkills(); });
      return;

    case 'memory_learned':
      if ((mine || roomMine) && ev.entryId) appendInFeed(memoryCard(ev));
      return;

    case 'memory':
      if (sharedMemoryOpen()) loadSharedMemory();
      // Вкладка показывает факты одного бота: перечитываем, только если правка задела его память.
      if (drawerShown() && S.tab === 'memory' && (ev.scopes || []).includes(S.currentId)) S.drawerMemoryLoad?.();
      return;

    case 'memory_doctor':
      if (ev.report?.note && !S.memoryRunning) toast(ev.report.note);
      if (sharedMemoryOpen()) loadSharedMemory();
      return;

    case 'room_turn_start':
      if (roomMine) finishWorkGroup();
      S.roomsBusy.add(ev.roomId);
      S.roomActive.set(ev.roomId, { botId: ev.botId || '', name: ev.name || '' });
      if (roomMine) {
        appendTyping();
        turnCard.phase('thinking');
        setSendState();
      } else {
        S.roomsUnread.add(ev.roomId);
      }
      renderSidebar();
      return;

    case 'room_delta':
      if (roomMine) roomDelta(ev);
      return;

    case 'room_message':
      if (roomMine) {
        // Готовое сообщение заменяет черновик из дельт, иначе ответ видно дважды.
        if (ev.message.role === 'assistant') {
          S.roomLive?.row.remove();
          S.roomLive = null;
        }
        upsertRoomMessage(ev.message);
        if (ev.message.role === 'user') turnCard.toEnd();
      } else {
        S.roomsUnread.add(ev.roomId);
      }
      renderSidebar();
      return;

    case 'room_message_update':
      if (roomMine) upsertRoomMessage(ev.message);
      return;

    case 'room_retry':
      if (roomMine) {
        S.roomLive?.row.remove();
        S.roomLive = null;
        ensureWorkGroup().querySelector('summary span').textContent = tr('Переподключаю модель…');
      }
      return;

    case 'room_turn_end':
      S.roomsBusy.delete(ev.roomId);
      S.roomActive.delete(ev.roomId);
      S.roomLive = null;
      turnCard.forget('room:' + ev.roomId);
      if (roomMine) {
        endTurnCard('room:' + ev.roomId);
        setSendState();
      }
      renderSidebar();
      refreshState();
      return;

    case 'workflow_approval':
      workflowApprovalEvent(ev);
      break;

    default:
      break;
  }

  switch (ev.type) {
    case 'turn_start':
      if (mine) finishWorkGroup();
      S.busy.add(ev.botId);
      // Ход в группе идёт в её треде: в личном чате печатать нечего.
      if (mine && !ev.roomId) {
        appendTyping();
        turnCard.phase('thinking');
        setSendState();
      }
      renderSidebar();
      break;

    case 'step_start':
      break;

    case 'text_delta':
      if (mine) appendDelta(ev);
      break;

    case 'message':
      if (!mine) {
        S.unread.add(ev.botId);
        renderSidebar();
        break;
      }
      if (ev.message.role === 'user') {
        appendThreadMessage(ev.message);
        turnCard.toEnd();
        scrollBottom();
      } else {
        removeTyping();
        finalizeLive(ev);
      }
      (S.threads.get(ev.botId) || S.threads.set(ev.botId, []).get(ev.botId)).push(ev.message);
      break;

    case 'tool_update':
      if (mine || roomCard) updateTool(ev);
      break;

    case 'activity':
      if (mine) showActivity(ev);
      break;

    case 'approval':
      // Карточка про группу идёт в группу, а не в личный чат бота.
      if ((mine && !ev.roomId) || roomCard || roomMine) approvalCard(ev);
      break;

    case 'approval_closed':
      closeCard(ev);
      break;

    case 'question':
      if (mine || roomCard) questionCard(ev);
      break;

    case 'question_closed':
      closeCard(ev);
      break;

    case 'task_done':
      // Строка о закрытой задаче: в личном чате ведущего, ход ведущего она не запускает.
      if (mine && !ev.roomId) {
        dropEmpty();
        appendInFeed(taskRow(ev.message));
        (S.threads.get(ev.botId) || S.threads.set(ev.botId, []).get(ev.botId)).push(ev.message);
        scrollBottom();
      } else if (!mine) {
        S.unread.add(ev.botId);
        renderSidebar();
      }
      break;

    case 'delegate_start':
      if (!window.DeloistTeam && (mine || roomCard)) delegateLine(ev);
      break;

    case 'delegate_end':
      if (!window.DeloistTeam && (mine || roomCard)) delegateLine(ev, true);
      break;

    case 'specialist_offer':
      // Карточку видит только открытый личный чат бота-хозяина.
      if (mine) specialistOfferCard(ev);
      break;

    case 'team_proposal':
      if (mine) teamProposalCard(ev);
      break;

    case 'team_import_request':
      teamImportCard(ev.request);
      break;

    case 'team_import_closed': {
      // Заявку закрыли в другом окне, отозвала командная строка или вышел срок.
      S.importCards?.get(ev.id)?.importFinish?.();
      break;
    }

    case 'optimizer_offer':
      if (mine) optimizerOfferCard(ev);
      break;

    case 'model_retry':
      if (mine) {
        S.live?.row.remove();
        S.live = null;
        ensureWorkGroup().querySelector('summary span').textContent = tr('Переподключаю модель…');
      }
      break;

    case 'model_switched':
      S.onSpare.set(ev.botId, ev.to);
      if (mine) {
        switchedLine(ev);
        paintHeadModel(botById(ev.botId));
      }
      break;

    // Две неудачи подряд: этот ход идёт на ступень выше, следующий успешный вернёт прежнюю.
    case 'level_escalated':
      if (mine) modelLine(tr('Подключаю модель сильнее: {model}', { model: ev.model || '' }));
      break;

    // Лимит подписки почти исчерпан: только предлагаем ступень ниже, уровень не меняем.
    case 'level_hint':
      if (mine) {
        modelLine(tr('Лимит провайдера почти исчерпан. Можно перейти на уровень «{level}».', {
          level: ev.level === 'strong' ? tr('Сильная') : ev.level === 'fast' ? tr('Быстрая') : tr('Обычная'),
        }));
      }
      break;

    // Ошибка уже стоит в чате и значком на аватаре: всплывающая подсказка её только дублировала.
    case 'error':
      break;

    case 'queue_updated': {
      const items = Array.isArray(ev.items) ? ev.items : [];
      S.queues.set(ev.botId, { paused: Boolean(ev.paused), pauseReason: ev.pauseReason, hold: S.queues.get(ev.botId)?.hold, items });
      const bot = botById(ev.botId);
      if (bot) bot.queued = items.length;
      if (mine) renderQueue();
      renderSidebar();
      break;
    }

    case 'journal':
      onJournalEvent();
      break;

    case 'updates':
      DeloistUpdates.apply(ev.updates);
      break;

    case 'session_stats':
      S.sessionStats.set(ev.botId, ev.stats);
      if (mine) renderSessionStats();
      break;

    case 'turn_end':
      S.busy.delete(ev.botId);
      if (compactAfterTurn.delete(ev.botId)) compactBot(ev.botId);
      S.unread.delete(ev.botId);
      S.onSpare.delete(ev.botId);
      turnCard.forget('bot:' + ev.botId);
      if (mine) {
        stepClock.clear();
        stepSpent.clear();
        S.live = null;
        finishWorkGroup();
        const card = endTurnCard('bot:' + ev.botId);
        setSendState();
        paintHeadModel(botById(ev.botId));
        if (!ev.quiet && !ev.failed) turnStats(ev, card);
      }
      renderSidebar();
      refreshPreview(ev.botId);
      // Тихий ход не оставляет следа: убираем уже показанные дельты и не метим непрочитанное.
      if (ev.quiet) {
        S.unread.delete(ev.botId);
        if (mine) selectBot(ev.botId);
        renderSidebar();
      }
      break;

    case 'open_bot':
      // Клик по питомцу на рабочем столе или по аватару в доке.
      if (ev.botId && botById(ev.botId)) selectBot(ev.botId);
      break;

    case 'open_new_bot':
      // «+» в доке.
      openModal('modal-new');
      break;

    case 'dev_reload':
      // Тестовая версия: интерфейс поправили на диске. Открытый чат вернётся после перечитывания.
      try { sessionStorage.setItem('deloist-dev-view', JSON.stringify(S.view || null)); } catch {}
      location.reload();
      break;
    case 'connector':
      // Свой MCP-сервер вошёл или потерял вход: строка списка должна показать это.
      refreshState().then(() => { if (!document.querySelector('.mcp-login-form')) renderMcp(); });
      break;
    case 'state_changed':
      // Питомца могли убрать крестиком на столе: выбор на экране не должен врать.
      refreshState().then(() => {
        markPets();
        markDock();
        // Бот мог поменять рутины из чата: открытый список и экран команды не должны показывать старое.
        if (routinesOpen()) renderRoutines();
        scheduleTeam();
        refreshCatalog();
        refreshWorkflowBots();
        if (drawerShown() && S.tab === 'goals') renderDrawer();
      });
      break;

    default:
      break;
  }
}

// ---------------------------------------------------------------------------
// «Нужен ты», разрешения разговора, правила и журнал решений
// ---------------------------------------------------------------------------

// Всё, что ждёт человека, по всем ботам и группам: карточки, передачи браузера, вопросы.
let needsItems = [];
let boardWorking = [];
let boardWaiting = [];
let needsTimer = null;

function refreshNeedsSoon() {
  clearTimeout(needsTimer);
  needsTimer = setTimeout(refreshNeeds, 150);
}

async function refreshNeeds() {
  try {
    const data = await api('/api/needs');
    needsItems = data.items || [];
    boardWorking = data.working || [];
    boardWaiting = data.waiting || [];
  } catch {
    return;
  }
  renderNeeds();
}

function needTitle(item) {
  if (item.kind === 'question') return item.question || '';
  if (item.kind === 'handoff') return item.detail || tr('Нужна твоя помощь в браузере');
  return trServer(item.title || item.tool || '');
}

function needKind(item) {
  return { question: tr('вопрос'), handoff: tr('браузер'), approval: tr('разрешение') }[item.kind] || '';
}

// Почему сообщение ждёт в очереди. Код приходит с сервера (`holdReason`), слова подбираем здесь.
function holdText(code) {
  return {
    paused_manual: tr('на паузе: остановлено вручную'),
    paused_restart: tr('на паузе после перезапуска'),
    limit: tr('ждёт: лимит токенов на месяц'),
    card: tr('ждёт твоего ответа на карточку'),
    busy: tr('ждёт конца хода'),
  }[code] || '';
}

// Причина для очереди открытого бота: пауза и лимит знает сервер, карточку и занятость видно по событиям.
function queueHold(id) {
  const data = S.queues.get(id);
  if (data?.paused) return data.pauseReason === 'manual' ? 'paused_manual' : 'paused_restart';
  if (data?.hold === 'limit') return 'limit';
  if (S.waiting.get(id)?.size) return 'card';
  return S.busy.has(id) ? 'busy' : '';
}

function boardSections() {
  const where = (item) => (item.roomName ? `${item.botName || ''} · ${item.roomName}` : item.botName || '');
  const needs = needsItems.map((item) => ({
    id: item.id, ref: item, where: where(item), meta: needKind(item), title: needTitle(item).slice(0, 140),
  }));
  const working = boardWorking.map((item) => ({
    id: `${item.kind}:${item.id || item.botId}`, ref: item, where: where(item),
    meta: window.DeloistUI.durationText(Date.now() - item.since) || tr('только что'),
    title: item.kind === 'task' ? tr('задача от {name}: {text}', { name: item.fromName || '', text: item.text || '' }) : '',
  }));
  const waiting = boardWaiting.map((item) => ({
    id: `${item.kind}:${item.id || item.botId}`, ref: item, where: where(item),
    meta: item.kind === 'queue' ? tr('В очереди: {count}', { count: item.count }) : tr('задача от {name}', { name: item.fromName || '' }),
    title: item.text || '',
    tag: holdText(item.reason),
  }));
  const unread = [
    ...S.bots.filter((b) => S.unread.has(b.id)).map((b) => ({ id: `bot:${b.id}`, ref: { botId: b.id }, where: b.name })),
    ...S.rooms.filter((r) => S.roomsUnread.has(r.id)).map((r) => ({ id: `room:${r.id}`, ref: { roomId: r.id }, where: r.name })),
  ];
  return [
    { key: 'needs', title: tr('Нужен ты'), items: needs },
    { key: 'working', title: tr('В работе'), items: working },
    { key: 'waiting', title: tr('Ждёт'), items: waiting },
    { key: 'unread', title: tr('Непрочитанные'), items: unread },
  ];
}

async function pickBoard(key, entry) {
  const item = entry.ref;
  if (key === 'needs') return openNeed(item);
  $('#needs-list').hidden = true;
  renderNeeds();
  if (item.roomId && roomById(item.roomId)) await selectRoom(item.roomId);
  else if (item.botId && botById(item.botId)) await selectBot(item.botId);
}

let needsBoardView = null;

function renderNeeds() {
  const btn = $('#btn-needs');
  const list = $('#needs-list');
  if (!btn || !list || !window.DeloistUI?.mountNeedsBoard) return;
  const count = needsItems.length;
  const sections = boardSections();
  const any = sections.some((s) => s.items.length);
  // Кнопка живёт, пока на доске что-то есть; значок считает только «Нужен ты».
  btn.hidden = !any;
  const badge = $('#needs-count');
  badge.textContent = String(count);
  badge.hidden = count === 0;
  if (!any) list.hidden = true;
  btn.setAttribute('aria-expanded', String(!list.hidden));
  if (list.hidden) return;
  if (needsBoardView) needsBoardView.update({ sections });
  else needsBoardView = window.DeloistUI.mountNeedsBoard(list, { sections, onpick: pickBoard });
}

// Переход к карточке: открыть разговор, дорисовать карточку, если её нет на странице, и показать её.
async function openNeed(item) {
  $('#needs-list').hidden = true;
  renderNeeds();
  if (item.roomId && roomById(item.roomId)) await selectRoom(item.roomId);
  else if (botById(item.botId)) await selectBot(item.botId);
  else return;
  let card = S.cards.get(item.id);
  if (!card?.isConnected) {
    S.cards.delete(item.id);
    if (item.kind === 'question') questionCard(item);
    else approvalCard(item);
    card = S.cards.get(item.id);
  }
  if (card) {
    card.scrollIntoView({ block: 'center', behavior: 'smooth' });
    card.classList.add('card-flash');
    setTimeout(() => card.classList.remove('card-flash'), 1600);
  }
}

// Строка над полем ввода: что разрешено в этом разговоре, и кнопка «Выключить».
function currentConv() {
  if (inRoom()) return S.view.id ? `room:${S.view.id}` : null;
  return S.currentId ? `bot:${S.currentId}` : null;
}

// Разрешения этого разговора. Мессенджерные (web.telegram.org и т.п.) показывает значок мессенджера
// в шапке (messengers.js); здесь остаётся щит со списком остальных.
let convGrants = [];
let grantsOpen = false;

function ownedByMessenger(grant) {
  return Boolean(window.DeloistMessengers?.ownsPlace(grant.place));
}

async function refreshGrantsLine() {
  const line = $('#grants-line');
  if (!line) return;
  const conv = currentConv();
  if (!conv) {
    convGrants = [];
    paintGrantsShield();
    window.dispatchEvent(new CustomEvent('deloist:grants'));
    return;
  }
  let grants = [];
  try {
    ({ grants } = await api(`/api/permissions/grants?conv=${encodeURIComponent(conv)}`));
  } catch {
    return;
  }
  if (conv !== currentConv()) return;
  convGrants = grants;
  paintGrantsShield();
  window.dispatchEvent(new CustomEvent('deloist:grants'));
}

function setGrantsOpen(open) {
  const pop = $('#grants-pop');
  grantsOpen = open && Boolean(pop.firstChild);
  pop.hidden = !grantsOpen;
  $('#grants-btn').setAttribute('aria-expanded', String(grantsOpen));
}

function paintGrantsShield() {
  const line = $('#grants-line');
  const rest = convGrants.filter((g) => !ownedByMessenger(g));
  line.hidden = !rest.length;
  if (!rest.length) {
    setGrantsOpen(false);
    return;
  }
  $('#grants-count').textContent = String(rest.length);
  const pop = $('#grants-pop');
  pop.replaceChildren();
  const head = document.createElement('div');
  head.className = 'grants-head';
  head.textContent = tr('Разрешено в разговоре');
  pop.append(head);
  for (const g of rest) {
    const item = document.createElement('div');
    item.className = 'grants-item';
    item.textContent = `${toolTitle(g.tool)}: ${placeLabel(g.place)}`;
    pop.append(item);
  }
  const off = document.createElement('button');
  off.type = 'button';
  off.className = 'grants-off';
  off.textContent = tr('Выключить все');
  off.addEventListener('click', async () => {
    await revokeGrants(rest);
  });
  pop.append(off);
  if (grantsOpen) pop.hidden = false;
}

async function revokeGrants(grants) {
  try {
    for (const g of grants) await api(`/api/permissions/grants/${encodeURIComponent(g.id)}`, { method: 'DELETE' });
  } catch (err) {
    toast(err.message);
  }
  await refreshGrantsLine();
}

window.DeloistGrants = {
  all: () => convGrants.slice(),
  toolTitle: (tool) => toolTitle(tool),
  place: (place) => placeLabel(place),
  revoke: (grants) => revokeGrants(grants),
};

function toolTitle(tool) {
  const titles = {
    browser_site: () => tr('действия на сайте'),
    browser_risky: () => tr('необратимое на сайте'),
    browser_upload: () => tr('загрузка файлов'),
    write_file: () => tr('запись файлов'),
    run_command: () => tr('команда'),
    generate_image: () => tr('картинки'),
  };
  if (titles[tool]) return titles[tool]();
  if (String(tool).startsWith('mcp__')) return tr('MCP: {name}', { name: String(tool).split('__').slice(2).join('__') });
  return tool;
}

// Настройки → «Мои скрипты»: что построили боты, выключатель предложений и удаление.
async function renderScripts() {
  const list = $('#scripts-list');
  if (!list) return;
  const view = await loadScripts();
  if (!view) {
    list.innerHTML = `<p class="note">${tr('Не удалось загрузить скрипты.')}</p>`;
    return;
  }
  const toggle = $('#optimizer-toggle');
  $('#optimizer-line').textContent = S.optimizerOn ? tr('Включено: Deloist смотрит журнал ботов и спрашивает') : tr('Выключено: предложений не будет');
  toggle.textContent = S.optimizerOn ? tr('Выключить') : tr('Включить');
  toggle.onclick = async () => {
    toggle.disabled = true;
    try {
      await api('/api/optimizer/enabled', { method: 'POST', body: { enabled: !S.optimizerOn } });
    } catch (err) {
      toast(err.message);
    }
    toggle.disabled = false;
    renderScripts();
  };
  list.innerHTML = '';
  if (!S.scriptsList.length) {
    list.innerHTML = `<p class="note">${tr('Пока ни одного. Когда бот сделает много одинаковых шагов, Deloist предложит ускорить это скриптом.')}</p>`;
    return;
  }
  for (const s of S.scriptsList) {
    const el = document.createElement('div');
    el.className = 'mcp-item';
    const bot = botById(s.botId)?.name || '';
    const state = s.verified ? tr('проверен, запусков: {n}', { n: s.runs }) : tr('бот ещё проверяет');
    el.innerHTML = `
      <span class="m-name">${escapeHtml(s.name)}</span>
      <span class="m-learned">${escapeHtml([bot, state].filter(Boolean).join(' · '))}</span>
      <span class="m-cmd" title="${escapeHtml(s.description || '')}">${escapeHtml(s.description || '')}</span>
      <div class="m-actions"><button class="icon-btn" data-del title="${tr('Удалить скрипт')}" aria-label="${tr('Удалить скрипт')}">${ICONS.close}</button></div>`;
    $('[data-del]', el).addEventListener('click', async () => {
      if (!confirm(tr('Удалить скрипт «{name}»? Боты забудут о нём.', { name: s.name }))) return;
      try {
        await api(`/api/optimizer/scripts/${encodeURIComponent(s.id)}`, { method: 'DELETE' });
      } catch (err) {
        toast(err.message);
      }
      renderScripts();
    });
    list.append(el);
  }
}

// Настройки → «Правила разрешений».
async function loadRules() {
  const box = $('#rules-list');
  if (!box) return;
  let rules = [];
  try {
    ({ rules } = await api('/api/permissions'));
  } catch {
    return;
  }
  S.rulesCount = rules.length;
  if (typeof updateSettingsLinks === 'function') updateSettingsLinks();
  box.innerHTML = '';
  if (!rules.length) {
    box.innerHTML = `<p class="note">${tr('Правил пока нет. Они появляются, когда на карточке нажимаешь «Всегда разрешать здесь».')}</p>`;
    return;
  }
  for (const rule of rules) {
    const row = document.createElement('div');
    row.className = 'rule-row';
    const bot = botById(rule.botId)?.name || rule.botId;
    const kind = KIND_LABELS[rule.kind]?.();
    row.innerHTML = `<span class="rule-text"><b>${escapeHtml(bot)}</b> · ${escapeHtml(toolTitle(rule.tool))} · ${escapeHtml(placeLabel(rule.place))}${kind ? ` · <span class="kind-chip">${escapeHtml(kind)}</span>` : ''}</span>
      <button type="button" class="ghost">${tr('Убрать')}</button>`;
    $('button', row).addEventListener('click', async () => {
      try {
        await api(`/api/permissions/rules/${encodeURIComponent(rule.id)}`, { method: 'DELETE' });
      } catch (err) {
        toast(err.message);
      }
      loadRules();
    });
    box.append(row);
  }
}

// Журнал решений на странице бота: что он сделал с чьего разрешения.
const HOW_LABELS = {
  rule: () => tr('разрешено правилом „всегда здесь“'),
  conversation: () => tr('разрешено в этом разговоре'),
  once: () => tr('ты разрешил один раз'),
  conversation_new: () => tr('ты разрешил в этом разговоре'),
  always: () => tr('ты разрешил всегда здесь'),
  deny: () => tr('ты запретил'),
};

function activityVerb(entry) {
  const label = entry.label || '';
  switch (entry.action) {
    case 'click': return label ? tr('нажал „{label}“', { label }) : tr('нажал на странице');
    case 'press': return tr('нажал клавиши {label}', { label });
    case 'type': return tr('ввёл текст');
    case 'drag': return tr('перетащил элемент');
    case 'dialog': return tr('ответил на диалог');
    case 'upload': return tr('загрузил файл');
    case 'site': return tr('начал действовать');
    case 'run': return tr('выполнил «{label}»', { label });
    case 'write': return tr('записал {label}', { label });
    default: return tr('вызвал {tool}', { tool: toolTitle(entry.tool) });
  }
}

function activityText(entry) {
  const showPlace = entry.place && entry.place !== '*' && entry.action !== 'run';
  const place = !showPlace ? '' : entry.action === 'write' ? ` ${tr('в {place}', { place: entry.place })}` : ` ${tr('на {place}', { place: entry.place })}`;
  const how = HOW_LABELS[entry.how]?.() || entry.how;
  return `${entry.bot || ''} ${activityVerb(entry)}${place} — ${how}`;
}

async function loadActivity(botId) {
  const box = $('#pf-activity');
  if (!box) return;
  let entries = [];
  try {
    ({ entries } = await api(`/api/bots/${encodeURIComponent(botId)}/activity`));
  } catch {
    return;
  }
  if (S.currentId !== botId) return;
  box.innerHTML = entries.length
    ? entries.map((e) => `<li><span class="act-time">${escapeHtml(timeOf(e.at))}</span> ${escapeHtml(activityText(e))}</li>`).join('')
    : `<li class="note">${tr('Пока пусто. Здесь видно, что бот делал по правилам и по твоим ответам на карточки.')}</li>`;
}

function onPermissionEvent(ev) {
  if (['approval', 'approval_closed', 'question', 'question_closed', 'turn_start', 'turn_end', 'queue_updated', 'task_update'].includes(ev.type)) refreshNeedsSoon();
  if (ev.type === 'turn_start' || ev.type === 'turn_end') renderQueue();
  if (ev.type === 'permissions_changed') {
    refreshGrantsLine();
    if ($('#modal-settings')?.open) loadRules();
  }
  if (ev.type === 'activity_logged' && ev.botId === S.currentId && drawerShown() && S.tab === 'profile') loadActivity(ev.botId);
}

// Заявка командной строки (`deloist team apply`) на добавление команды из файла. Карточка общая для всего окна:
// она не привязана к чату бота. Ничего не создаётся, пока человек не нажмёт «Добавить»; закрытие окна клавишей Esc
// считается отказом. Ключ программы такую карточку принять не может: адреса принятия закрыты для него в guard.rs.
function teamImportCard(request) {
  if (!request?.id || S.importCards?.has(request.id)) return;
  S.importCards ||= new Map();
  const preview = request.preview || {};
  const list = (items) => (Array.isArray(items) ? items : []);
  const MAX_SHOWN = 12;
  const section = (title, rows) => {
    if (!rows.length) return '';
    const shown = rows.slice(0, MAX_SHOWN).map((row) => `<li>${row}</li>`).join('');
    const more = rows.length > MAX_SHOWN ? `<li class="note">${tr('и ещё {n}', { n: rows.length - MAX_SHOWN })}</li>` : '';
    return `<div class="note"><b>${title}</b></div><ul class="team-list">${shown}${more}</ul>`;
  };
  // Начало текста: человек видит, какие указания получит бот, а не только имена.
  const sample = (text, chars) => (text ? `<div class="note team-excerpt">${escapeHtml(text)}${chars ? ` · ${tr('{n} знаков', { n: Number(chars) || 0 })}` : ''}</div>` : '');
  const bots = list(preview.bots).map((b) => `<b>${escapeHtml(b.name)}</b>${b.role ? ` — ${escapeHtml(b.role)}` : ''}${sample(b.excerpt, b.chars)}`);
  const skills = list(preview.skills).map((k) => `<b>${escapeHtml(k.name)}</b>${sample(k.excerpt, k.chars)}`);
  const rooms = list(preview.rooms).map((r) => `<b>${escapeHtml(r.name)}</b> · ${tr('участников: {n}', { n: Number(r.members) || 0 })}`);
  const routines = list(preview.routines).map((r) => `<b>${escapeHtml(r.title || '…')}</b> — ${escapeHtml(r.bot)}${sample(r.excerpt, r.chars)}`);
  const warnings = preview.warnings || {};
  const warn = [
    warnings.longText ? tr('Есть очень длинные персоны или навыки: прочитайте их целиком, прежде чем добавлять.') : '',
    warnings.manyBots ? tr('В файле больше пяти ботов.') : '',
  ].filter(Boolean).map((line) => `<p class="note team-access team-warn">⚠ ${line}</p>`).join('');
  const dlg = document.createElement('dialog');
  dlg.className = 'modal kit-dialog kit-form team-import-card';
  dlg.innerHTML = `<form method="dialog" class="modal-inner">
    <h3 tabindex="-1">${tr('Добавить команду из файла')}</h3>
    <p class="note">${tr('Командная строка (deloist team apply) просит добавить в Deloist команду из файла. Пока вы не нажмёте «Добавить», ничего не создаётся.')}</p>
    <p class="team-code">${tr('Код сверки')}: <b data-code>${escapeHtml(request.code || '')}</b> <span class="note">${tr('Он должен совпасть с кодом в вашем терминале. Не совпал или вы ничего не запускали — отклоните.')}</span></p>
    ${section(tr('Боты: {n}', { n: bots.length }), bots)}
    ${section(tr('Навыки: {n}', { n: skills.length }), skills)}
    ${section(tr('Команды: {n}', { n: rooms.length }), rooms)}
    ${section(tr('Рутины: {n}', { n: routines.length }), routines)}
    <p class="note">${tr('Всего текста указаний: {chars} знаков; навыков: {skills}.', { chars: Number(preview.totalChars) || 0, skills: skills.length })}</p>
    ${warn}
    <p class="note team-defaults">${tr('Новые боты начнут в режиме «спрашивать», без сайтов в браузере и без папок. Рутины придут выключенными.')}</p>
    <p class="note team-access team-warn">⚠ ${tr('Персоны и навыки из файла станут указаниями для ботов. Если вы не запускали эту команду, нажмите «Отклонить».')}</p>
    <p class="note" data-pending></p>
    <div class="modal-actions">
      <button type="button" class="ghost" data-reject-all>${tr('Отклонить все и не принимать заявки 1 час')}</button>
      <button type="button" class="ghost" data-decline>${tr('Отклонить')}</button>
      <button type="button" class="primary" data-accept>${tr('Добавить')}</button>
    </div>
  </form>`;
  document.body.appendChild(dlg);
  S.importCards.set(request.id, dlg);
  const paintPending = () => {
    const n = S.importCards.size;
    S.importCards.forEach((card) => {
      const line = card.querySelector('[data-pending]');
      if (line) line.textContent = tr('Ждут ответа заявок: {n}', { n });
    });
  };
  paintPending();
  let timer = null;
  const finish = () => {
    clearTimeout(timer);
    S.importCards.delete(request.id);
    if (dlg.open) dlg.close();
    dlg.remove();
    paintPending();
  };
  dlg.importFinish = finish;
  const resolve = async (action) => {
    $$('button', dlg).forEach((btn) => { btn.disabled = true; });
    try {
      const done = await api(`/api/team/import-request/${encodeURIComponent(request.id)}/${action}`, { method: 'POST' });
      finish();
      if (action === 'approve') {
        await refreshState();
        renderSkills();
        renderRoutines();
        const imported = done.imported || {};
        toast(tr('Добавлено. Ботов: {bots}, навыков: {skills}, команд: {rooms}, рутин: {routines} (выключены).', { bots: imported.bots || 0, skills: imported.skills || 0, rooms: imported.rooms || 0, routines: imported.routines || 0 }), 6000);
      }
    } catch (err) {
      if (err.status === 404 || err.status === 409 || err.status === 410) {
        finish();
        if (err.status !== 404) toast(trServer(err.message), 6000);
        return;
      }
      toast(tr('Не смог добавить: {error}', { error: trServer(err.message) }), 7000);
      $$('button', dlg).forEach((btn) => { btn.disabled = false; });
    }
  };
  dlg.addEventListener('cancel', (e) => {
    e.preventDefault();
    resolve('deny');
  });
  $('[data-accept]', dlg).addEventListener('click', () => resolve('approve'));
  $('[data-decline]', dlg).addEventListener('click', () => resolve('deny'));
  $('[data-reject-all]', dlg).addEventListener('click', async () => {
    $$('button', dlg).forEach((btn) => { btn.disabled = true; });
    try {
      await api('/api/team/import-requests/reject-all', { method: 'POST' });
      [...S.importCards.values()].forEach((card) => card.importFinish?.());
      toast(tr('Заявки отклонены, новые не принимаются час.'), 5000);
    } catch (err) {
      toast(trServer(err.message), 6000);
      $$('button', dlg).forEach((btn) => { btn.disabled = false; });
    }
  });
  // Просроченную заявку принять нельзя: карточка гаснет сама.
  timer = setTimeout(finish, Math.max(1, Number(request.expiresInSeconds) || 600) * 1000);
  dlg.showModal();
  $('[data-decline]', dlg)?.focus({ preventScroll: true });
  dlg.scrollTop = 0;
  dlg.querySelector('.modal-inner')?.scrollTo?.(0, 0);
}

// Заявки, поданные, пока окно было закрыто или связь прерывалась.
async function showImportRequests() {
  try {
    const { requests } = await api('/api/team/import-requests');
    for (const request of Array.isArray(requests) ? requests : []) teamImportCard(request);
  } catch {
    // не критично: заявка подаётся ещё раз командой
  }
}

function connectEvents() {
  const es = new EventSource('/api/events');
  es.onopen = () => showImportRequests();
  es.onmessage = (e) => {
    try {
      handleEvent(JSON.parse(e.data));
    } catch {
      // мусорное событие — пропускаем
    }
  };
  es.onerror = () => {
    // EventSource переподключится сам; подстрахуемся обновлением состояния.
    setTimeout(refreshState, 1500);
  };
}

async function refreshPreview(botId) {
  try {
    const data = await api('/api/state');
    S.bots = data.bots;
    S.mentionPicker?.update?.({ bots: S.bots });
    renderSidebar();
  } catch {
    // не критично
  }
}

// Сервер знает точно: события могли потеряться, пока окно было закрыто или связь прерывалась.
function syncRooms() {
  S.roomsBusy.clear();
  S.roomActive.clear();
  for (const room of S.rooms) {
    if (room.busy) S.roomsBusy.add(room.id);
    if (room.activeBot) S.roomActive.set(room.id, { botId: room.activeBot, name: botById(room.activeBot)?.name || '' });
  }
}

async function refreshState() {
  try {
    const data = await api('/api/state');
    S.bots = data.bots;
    S.mentionPicker?.update?.({ bots: S.bots });
    S.rooms = data.rooms || [];
    S.skills = data.skills || [];
    S.mcp = data.mcp || [];
    S.routines = data.routines;
    S.goals = data.goals || [];
    S.checkins = data.checkins || [];
    S.settings = data.settings;
    S.platform = data.platform || S.platform || {};
    S.levels = data.levels || S.levels || {};
    S.dataDir = data.dataDir;
    paintSecretsBanner(data.secretsUnavailable === true);
    for (const bot of S.bots) {
      if (bot.busy) S.busy.add(bot.id);
      const queue = S.queues.get(bot.id);
      if (queue) bot.queued = queue.items.length;
    }
    syncRooms();
    pruneAvatars();
    syncOrchStage();
    markLook();
    renderSidebar();
    syncWaiting();
    paintModeSwitch();
    // Модель бота или настроек могли смениться: шапка открытого чата не должна показывать старую.
    if (S.view?.kind === 'bot') paintHeadModel(botById(S.view.id));
    // Бот завёл рутину из разговора или её поправили в окне «Рутины»: вкладка панели показывает свежий список.
    if (drawerShown() && S.tab === 'routines') renderDrawer();
  } catch {
    // сервер мог перезапускаться
  }
}

// ---------------------------------------------------------------------------
// Очередь текущего бота
// ---------------------------------------------------------------------------

const QUEUE_ICONS = { grip: 'grip', up: 'arrow-up', down: 'arrow-down', edit: 'edit', remove: 'trash' };

let queueDrag = null; // {id, pointerId, handle, row, startX, startY, started, beforeId, drop, clientY}
let queueRenderPending = false;
let queueFocus = null; // {id, which}
let queueSignature = '';

function queueButton(label, action, title = '') {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'ghost queue-act';
  btn.textContent = label;
  if (title) btn.title = title;
  btn.addEventListener('click', action);
  return btn;
}

function queueIcon(name, label, action, disabled = false) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = `queue-icon queue-icon-${name}`;
  btn.title = label;
  btn.setAttribute('aria-label', label);
  btn.innerHTML = ic(QUEUE_ICONS[name]);
  btn.disabled = disabled;
  if (action) btn.addEventListener('click', action);
  return btn;
}

function queueGrip(id) {
  const grip = document.createElement('button');
  grip.type = 'button';
  grip.className = 'queue-grip';
  grip.title = tr('Перетащить');
  grip.setAttribute('aria-label', tr('Перетащить'));
  grip.innerHTML = ic(QUEUE_ICONS.grip);
  // С клавиатуры двигают стрелками, ручка нужна только мыши.
  grip.tabIndex = -1;
  grip.addEventListener('pointerdown', (e) => startQueueDrag(e, id, grip));
  return grip;
}

function queueEditField(item) {
  const input = document.createElement('input');
  input.className = 'queue-edit';
  input.type = 'text';
  input.value = S.queueEdit.value;
  input.setAttribute('aria-label', tr('Текст сообщения'));
  input.addEventListener('input', () => {
    if (!S.queueEdit || S.queueEdit.id !== item.id) return;
    S.queueEdit.value = input.value;
    S.queueEdit.start = input.selectionStart;
    S.queueEdit.end = input.selectionEnd;
  });
  let saving = false;
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (saving) return;
      saving = true;
      saveQueueText(item.id, input.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (saving) return;
      S.queueEdit = null;
      renderQueue(true);
    }
  });
  return input;
}

function queueRow(item, index, items) {
  const row = document.createElement('div');
  row.className = 'queue-item';
  row.dataset.qid = item.id;

  const single = items.length === 1;
  const editing = Boolean(S.queueEdit && S.queueEdit.id === item.id);

  if (!single) row.append(queueGrip(item.id));

  if (editing) {
    row.append(queueEditField(item));
  } else {
    const text = document.createElement('span');
    text.className = 'queue-text';
    const files = Array.isArray(item.attachments) ? item.attachments.map((f) => f.name).join(', ') : '';
    text.textContent = item.text || files;
    text.title = item.text || files;
    row.append(text);
  }

  row.append(
    queueButton(tr('Направить'), () => queueAction(item.id, 'steer'), tr('Отправить сразу, не дожидаясь конца хода')),
  );

  if (!single) {
    const prev = index > 0 ? items[index - 1].id : null;
    const afterNext = index + 2 < items.length ? items[index + 2].id : null;
    row.append(
      queueIcon('up', tr('Выше'), () => queueMove(item.id, prev, 'up'), index === 0),
      queueIcon('down', tr('Ниже'), () => queueMove(item.id, afterNext, 'down'), index === items.length - 1),
    );
  }

  row.append(
    queueIcon('edit', tr('Изменить'), () => editQueueItem(row, item)),
    queueIcon('remove', tr('Убрать'), () => queueAction(item.id, 'remove')),
  );
  return row;
}

// Текст правки не должен пропасть, если сообщение уже забрал бот.
function salvageQueueText(value, original) {
  const prompt = $('#input');
  if (value !== original && prompt && !prompt.value.trim()) {
    prompt.value = value;
    autosize();
    setSendState();
    toast(tr('Это сообщение уже ушло боту, твоя правка теперь в поле ввода'));
  } else {
    toast(tr('Это сообщение уже ушло боту'));
  }
}

function applyQueueFocus() {
  const list = $('#queue-list');
  if (!list) return;
  if (S.queueEdit) {
    const row = $$('.queue-item', list).find((item) => item.dataset.qid === S.queueEdit.id);
    const input = row ? $('.queue-edit', row) : null;
    if (input) {
      input.focus();
      const start = Number.isInteger(S.queueEdit.start) ? S.queueEdit.start : input.value.length;
      const end = Number.isInteger(S.queueEdit.end) ? S.queueEdit.end : start;
      input.setSelectionRange(start, end);
    }
    queueFocus = null;
    return;
  }
  if (!queueFocus) return;
  const row = $$('.queue-item', list).find((item) => item.dataset.qid === queueFocus.id);
  if (row) {
    const first = queueFocus.which === 'up' ? $('.queue-icon-up', row) : $('.queue-icon-down', row);
    const second = queueFocus.which === 'up' ? $('.queue-icon-down', row) : $('.queue-icon-up', row);
    const target = first && !first.disabled ? first : second;
    if (target) target.focus();
  }
  queueFocus = null;
}

function renderQueue(force = false) {
  const box = $('#queue');
  if (!box) return;
  // Пока тянут строку, список не пересобираем: иначе перетаскивание пропадёт.
  if (queueDrag) {
    queueRenderPending = true;
    return;
  }
  const data = S.currentId ? S.queues.get(S.currentId) : null;
  const items = data?.items || [];

  if (S.queueEdit && !items.some((item) => item.id === S.queueEdit.id)) {
    const edit = S.queueEdit;
    S.queueEdit = null;
    salvageQueueText(edit.value, edit.original);
  }

  const signature = JSON.stringify({
    id: S.currentId,
    paused: Boolean(data?.paused),
    pauseReason: data?.pauseReason,
    hold: S.currentId ? queueHold(S.currentId) : '',
    edit: S.queueEdit?.id || null,
    items: items.map((item) => [item.id, item.text]),
  });
  if (!force && signature === queueSignature) return;
  queueSignature = signature;

  box.hidden = !items.length;
  $('#queue-count').textContent = tr('В очереди: {count}', { count: items.length });
  const hold = S.currentId ? queueHold(S.currentId) : '';
  $('#queue-note').hidden = !hold;
  $('#queue-note').textContent = holdText(hold);
  $('#queue-resume').hidden = !data?.paused;

  const list = $('#queue-list');
  // Курсор мог сдвинуться без ввода, поэтому его место читаем прямо перед перерисовкой.
  const typing = $('.queue-edit', list);
  if (typing && S.queueEdit) {
    S.queueEdit.start = typing.selectionStart;
    S.queueEdit.end = typing.selectionEnd;
  }
  list.innerHTML = '';
  for (let index = 0; index < items.length; index += 1) list.append(queueRow(items[index], index, items));
  applyQueueFocus();
}

async function loadQueue(id) {
  if (!id) return;
  try {
    const data = await api(`/api/bots/${id}/queue`);
    const items = Array.isArray(data.items) ? data.items : [];
    S.queues.set(id, { paused: Boolean(data.paused), pauseReason: data.pauseReason, hold: data.holdReason, items });
    const bot = botById(id);
    if (bot) bot.queued = items.length;
    if (id === S.currentId) renderQueue();
    renderSidebar();
  } catch {
    // маршрута может не быть у старого сервера — просто не показываем очередь
  }
}

function editQueueItem(row, item) {
  if (S.queueEdit && S.queueEdit.id === item.id) return;
  S.queueEdit = { id: item.id, value: item.text, original: item.text };
  renderQueue(true);
}

async function saveQueueText(id, value) {
  const bot = S.currentId;
  if (!bot) return;
  const edit = S.queueEdit && S.queueEdit.id === id ? S.queueEdit : null;
  const original = edit ? edit.original : value;
  const text = value.trim();
  if (!text) {
    toast(tr('Пустое сообщение не сохраню'));
    if (S.queueEdit && S.queueEdit.id === id) S.queueEdit = null;
    renderQueue(true);
    return;
  }
  try {
    await api(`/api/bots/${bot}/queue/${encodeURIComponent(id)}`, { method: 'PATCH', body: { text } });
    if (bot !== S.currentId) return;
    if (S.queueEdit && S.queueEdit.id === id) S.queueEdit = null;
    await loadQueue(bot);
  } catch (err) {
    if (bot !== S.currentId) return;
    if (err.status === 404) {
      // Если события ещё не было, спасаем текст здесь; если было — уже спасли.
      if (S.queueEdit && S.queueEdit.id === id) {
        S.queueEdit = null;
        salvageQueueText(value, original);
      }
      await loadQueue(bot);
      return;
    }
    toast(err.message);
    renderQueue(true);
  }
}

async function queueAction(id, action) {
  const bot = S.currentId;
  if (!bot) return;
  const base = `/api/bots/${bot}/queue/${encodeURIComponent(id)}`;
  // Действие над сообщением в правке: это не «бот его забрал», спасать текст незачем.
  const edit = S.queueEdit?.id === id ? S.queueEdit : null;
  if (edit) S.queueEdit = null;
  try {
    if (action === 'remove') {
      await api(base, { method: 'DELETE' });
    } else {
      // «Направить» из правки отправляет то, что человек видит в поле.
      const text = edit?.value.trim();
      if (text && text !== edit.original) await api(base, { method: 'PATCH', body: { text } });
      await api(`${base}/steer`, { method: 'POST' });
    }
    if (bot === S.currentId) await loadQueue(bot);
  } catch (err) {
    if (bot === S.currentId) toast(err.message);
  }
}

async function queueMove(id, before, which) {
  const bot = S.currentId;
  if (!bot) return;
  try {
    const data = await api(`/api/bots/${bot}/queue/${encodeURIComponent(id)}/move`, {
      method: 'POST',
      body: { before },
    });
    if (bot !== S.currentId) return;
    const items = Array.isArray(data.items) ? data.items : [];
    S.queues.set(bot, { paused: Boolean(data.paused), pauseReason: data.pauseReason, items });
    const botData = botById(bot);
    if (botData) botData.queued = items.length;
    queueFocus = which ? { id, which } : null;
    renderQueue(true);
    renderSidebar();
  } catch (err) {
    if (bot !== S.currentId) return;
    toast(err.message);
    await loadQueue(bot);
  }
}

function clearQueueDragVisual(drag) {
  drag.row.classList.remove('queue-dragging');
  drag.handle.classList.remove('queue-grabbing');
  if (drag.drop) drag.drop.remove();
  const list = $('#queue-list');
  if (list) list.classList.remove('queue-moving');
  window.removeEventListener('pointermove', onQueueDragMove, true);
  window.removeEventListener('pointerup', onQueueDragEnd, true);
  window.removeEventListener('pointercancel', onQueueDragCancel, true);
  window.removeEventListener('keydown', queueDragKey);
}

function queueDropIsSame(id, before) {
  const items = S.queues.get(S.currentId)?.items || [];
  const index = items.findIndex((item) => item.id === id);
  if (index < 0) return false;
  const next = index + 1 < items.length ? items[index + 1].id : null;
  return before === next;
}

function endQueueDrag(commit) {
  const drag = queueDrag;
  if (!drag) return;
  queueDrag = null;
  clearQueueDragVisual(drag);
  const changed = commit && drag.started && !queueDropIsSame(drag.id, drag.beforeId);
  if (queueRenderPending) {
    queueRenderPending = false;
    renderQueue(true);
  }
  if (changed) queueMove(drag.id, drag.beforeId, null);
}

function abortQueueDrag() {
  const drag = queueDrag;
  if (!drag) return;
  queueDrag = null;
  queueRenderPending = false;
  clearQueueDragVisual(drag);
}

function queueDragKey(e) {
  if (e.key !== 'Escape' || !queueDrag) return;
  e.preventDefault();
  endQueueDrag(false);
}

function updateQueueDrop(drag) {
  const list = $('#queue-list');
  if (!list || !drag.drop) return;
  const rect = list.getBoundingClientRect();
  const rows = $$('.queue-item', list).filter((row) => row !== drag.row);
  let index = rows.length;
  for (let i = 0; i < rows.length; i += 1) {
    const box = rows[i].getBoundingClientRect();
    if (drag.clientY < box.top + box.height / 2) {
      index = i;
      break;
    }
  }
  drag.beforeId = index < rows.length ? rows[index].dataset.qid : null;
  let y = 0;
  if (index < rows.length) {
    y = rows[index].getBoundingClientRect().top - rect.top + list.scrollTop;
  } else if (rows.length) {
    y = rows[rows.length - 1].getBoundingClientRect().bottom - rect.top + list.scrollTop;
  }
  drag.drop.style.top = `${Math.max(0, y - 1)}px`;
}

function queueAutoScroll() {
  const drag = queueDrag;
  if (!drag || !drag.started) return;
  const list = $('#queue-list');
  if (list) {
    const rect = list.getBoundingClientRect();
    const edge = 18;
    let delta = 0;
    if (drag.clientY < rect.top + edge) delta = -Math.max(2, Math.ceil((rect.top + edge - drag.clientY) / 3));
    else if (drag.clientY > rect.bottom - edge) delta = Math.max(2, Math.ceil((drag.clientY - (rect.bottom - edge)) / 3));
    if (delta) {
      list.scrollTop += delta;
      updateQueueDrop(drag);
    }
  }
  if (queueDrag === drag) requestAnimationFrame(queueAutoScroll);
}

function onQueueDragMove(e) {
  const drag = queueDrag;
  if (!drag || e.pointerId !== drag.pointerId) return;
  // Кнопку отпустили там, где мы этого не увидели: иначе список так и останется замороженным.
  if (!e.buttons) {
    endQueueDrag(false);
    return;
  }
  if (!drag.started) {
    const dx = e.clientX - drag.startX;
    const dy = e.clientY - drag.startY;
    if (Math.hypot(dx, dy) <= 4) return;
    drag.started = true;
    drag.row.classList.add('queue-dragging');
    drag.handle.classList.add('queue-grabbing');
    const list = $('#queue-list');
    if (list) {
      list.classList.add('queue-moving');
      drag.drop = document.createElement('div');
      drag.drop.className = 'queue-drop';
      list.append(drag.drop);
    }
    requestAnimationFrame(queueAutoScroll);
  }
  drag.clientY = e.clientY;
  updateQueueDrop(drag);
}

function onQueueDragEnd(e) {
  if (!queueDrag || e.pointerId !== queueDrag.pointerId) return;
  queueDrag.clientY = e.clientY;
  if (queueDrag.started) updateQueueDrop(queueDrag);
  endQueueDrag(true);
}

function onQueueDragCancel(e) {
  if (!queueDrag || e.pointerId !== queueDrag.pointerId) return;
  endQueueDrag(false);
}

function startQueueDrag(e, id, grip) {
  if (e.button !== 0 || queueDrag) return;
  const row = grip.closest('.queue-item');
  if (!row) return;
  queueDrag = {
    id,
    pointerId: e.pointerId,
    handle: grip,
    row,
    startX: e.clientX,
    startY: e.clientY,
    started: false,
    beforeId: null,
    drop: null,
    clientY: e.clientY,
  };
  // Захват нужен, чтобы видеть указатель за краем окна. Сами события слушаем на окне:
  // без захвата отпускание над другой строкой иначе потерялось бы.
  try {
    grip.setPointerCapture(e.pointerId);
  } catch {
    // обойдёмся слушателями на окне
  }
  window.addEventListener('pointermove', onQueueDragMove, true);
  window.addEventListener('pointerup', onQueueDragEnd, true);
  window.addEventListener('pointercancel', onQueueDragCancel, true);
  window.addEventListener('keydown', queueDragKey);
  e.preventDefault();
}


// ---------------------------------------------------------------------------
// Бот: выбор, отправка
// ---------------------------------------------------------------------------

async function selectBot(id) {
  leaveTeam();
  if (S.currentId !== id) {
    // Правка и перетаскивание принадлежат прежнему боту.
    S.queueEdit = null;
    abortQueueDrag();
    queueFocus = null;
  }
  S.currentId = id;
  window.dispatchEvent(new CustomEvent('deloist:bot', { detail: id }));
  S.view = { kind: 'bot', id };
  attachFollowView();
  S.roomLive = null;
  $('#head-avatar').classList.remove('room-icon');
  setPanelTitle(tr('Профиль, память, файлы'));
  $('#btn-browser').hidden = false;
  $('#head-reassign').hidden = true;
  $('#head-rebuild').hidden = true;
  closeReassignMenu();
  S.unread.delete(id);
  S.live = null;
  const bot = botById(id);
  if (!bot) return;

  paintAvatar($('#head-avatar'), bot, 'head', { size: 76 });
  $('#head-name').textContent = bot.name;
  $('#head-role').textContent = bot.role || '';
  paintGuests();

  paintHeadModel(bot);
  paintModeSwitch();

  const input = $('#input');
  input.disabled = false;
  input.placeholder = tr('Напиши, что нужно сделать');

  renderSidebar();
  clearMessages();
  renderQueue();
  loadQueue(id);

  const cached = S.threads.get(id);
  if (cached?.length) renderThread(cached);
  try {
    const { messages } = await api(`/api/bots/${id}/thread?limit=300`);
    if (S.currentId !== id || inRoom()) return;
    S.threads.set(id, messages);
    renderThread(messages);
    // Сразу, без ожидания: иначе событие, пришедшее за это время, показалось бы дважды.
    if (S.busy.has(id)) {
      appendTyping();
      replayTurn(id);
    }
  } catch (err) {
    if (S.currentId !== id || inRoom()) return;
    if (!cached?.length) {
      const note = document.createElement('div');
      note.className = 'empty';
      note.textContent = tr('Не удалось загрузить чат. Закрой окно в трей и открой снова.');
      messagesEl().append(note);
    }
    toast(err.message);
    setSendState();
    return;
  }

  await showPending(id);
  refreshGrantsLine();
  loadOptimizerOffers(id);

  setSendState();
  renderSessionStats();
  loadSessionStats(id);
  showDrawerForBot();
  input.focus();
}

// ---------------------------------------------------------------------------
// Строка под полем ввода: ходы и шаги сеанса, скорость, токены, кэш и окно до сжатия
// ---------------------------------------------------------------------------

async function loadSessionStats(id) {
  try {
    const stats = await api(`/api/bots/${id}/stats`);
    S.sessionStats.set(id, stats);
    if (!inRoom() && S.currentId === id) renderSessionStats();
  } catch {
    // старый сервер без счётчиков: строки просто нет
  }
}

function fmtDuration(ms) {
  const v = Math.max(0, Number(ms || 0));
  if (v < 1000) return tr('{n} мс', { n: Math.round(v) });
  const sec = Math.round(v / 1000);
  if (sec < 60) return tr('{n} с', { n: (v / 1000).toLocaleString(window.LOCALE, { maximumFractionDigits: 1 }) });
  const min = Math.floor(sec / 60);
  if (min < 60) return tr('{m} мин {s} с', { m: min, s: sec % 60 });
  return tr('{h} ч {m} мин', { h: Math.floor(min / 60), m: min % 60 });
}

function statPop(title, value, rows, note) {
  const pop = document.createElement('span');
  pop.className = 'stat-pop';
  pop.setAttribute('role', 'tooltip');
  const head = document.createElement('span');
  head.className = 'stat-pop-head';
  const b = document.createElement('b');
  b.textContent = title;
  const v = document.createElement('span');
  v.textContent = value;
  head.append(b, v);
  pop.append(head);
  for (const [label, val] of rows) {
    const row = document.createElement('span');
    row.className = 'stat-pop-row';
    const l = document.createElement('span');
    l.textContent = label;
    const r = document.createElement('span');
    r.textContent = val;
    row.append(l, r);
    pop.append(row);
  }
  if (note) {
    const n = document.createElement('span');
    n.className = 'stat-pop-note';
    n.textContent = note;
    pop.append(n);
  }
  return pop;
}

function statItem(icon, text, pop) {
  const item = document.createElement('span');
  item.className = 'stat';
  item.tabIndex = 0;
  item.innerHTML = icon;
  const label = document.createElement('span');
  label.textContent = text;
  item.append(label, pop);
  return item;
}


function contextRing(percent) {
  const r = 7;
  const c = 2 * Math.PI * r;
  const filled = (Math.min(100, Math.max(0, percent)) / 100) * c;
  return `<svg class="ring" viewBox="0 0 18 18" aria-hidden="true"><circle cx="9" cy="9" r="${r}" class="ring-bg"/><circle cx="9" cy="9" r="${r}" class="ring-fg" stroke-dasharray="${filled.toFixed(2)} ${c.toFixed(2)}" transform="rotate(-90 9 9)"/></svg>`;
}

function renderSessionStats() {
  const el = $('#session-stats');
  const st = !inRoom() && S.currentId ? S.sessionStats.get(S.currentId) : null;
  const turns = Number(st?.turns || 0);
  const percent = Number(st?.context?.percent || 0);
  el.replaceChildren();
  el.hidden = !st || (!turns && !Number(st.context?.chars));
  if (el.hidden) return;

  // Один значок: кольцо контекста. Ходы, шаги, токены и кэш лежат в его всплывашке (кит, правка #7).
  const extra = [];
  if (turns) {
    const tps = Number(st.tps || 0);
    extra.push(
      [tr('Ходов'), numberRu(turns)],
      [tr('Шагов'), numberRu(st.steps)],
      [tr('Время модели'), fmtDuration(st.modelMs)],
      [tr('Работа инструментов'), fmtDuration(st.toolMs)],
      [tr('Среднее до первого токена'), st.ttftMs ? fmtDuration(st.ttftMs) : '—'],
      [tr('Токенов в секунду'), tps ? tr('{n} ток/с', { n: numberRu(tps) }) : '—'],
    );
    const prompt = Number(st.prompt || 0);
    const cached = Number(st.cached || 0);
    const completion = Number(st.completion || 0);
    if (prompt + completion) {
      extra.push(
        [tr('Использование токенов'), tr('{n} ток', { n: numberRu(prompt + completion) })],
        [tr('Попадание в кэш'), `${st.cacheRate}%`],
        [tr('Некэшированный ввод'), tr('{n} ток', { n: numberRu(prompt - cached) })],
        [tr('Кэшированный ввод'), tr('{n} ток', { n: numberRu(cached) })],
        [tr('Выход'), tr('{n} ток', { n: numberRu(completion) })],
      );
    }
  }

  const ctx = st.context || {};
  // Окно почти пустое, но не пустое: «0%» читалось бы как «истории нет».
  const shown = !percent && Number(ctx.chars) > 0 ? '<1' : String(percent);
  // У агентов ACP окно настоящее, в токенах: от самого агента или из таблицы моделей.
  const tokens = Number(ctx.windowTokens) > 0;
  const windowRow = tokens
    ? [tr('В окне'), tr('{used} из {total} ток', { used: fmtCount(ctx.usedTokens || 0), total: fmtCount(ctx.windowTokens) })]
    : [tr('В окне'), tr('{used} из {total} знаков', { used: fmtCount(ctx.chars), total: fmtCount(ctx.budget) })];
  const rows = [windowRow];
  if (!tokens && ctx.compactPercent) {
    rows.push([tr('Сжатие начнётся при'), tr('{p}% лимита, ≈ {n} ток', { p: ctx.compactPercent, n: fmtCount(ctx.compactAt) })]);
  }
  if (tokens) rows.push([tr('Откуда цифры'), ctx.source === 'agent' ? tr('сообщает агент') : tr('оценка по окну модели')]);
  rows.push(...extra);
  el.append(statItem(contextRing(percent), tr('Контекст {n}%', { n: shown }), statPop(tr('Контекст'), tr('Занято: {n}%', { n: shown }), rows,
    tr('Шкала идёт до порога сжатия: при 100% старая часть истории сожмётся в сводку. Порог выбирается в настройках бота.'))));
}

// Переключатель под полем: «свобода действий» (auto) включена или выключена (тогда ask/strict: бот спрашивает).
// Только в личном чате: в группе у каждого бота свой режим.
function paintModeSwitch() {
  const sw = $('#mode-switch');
  const bot = inRoom() ? null : botById(S.currentId);
  sw.hidden = !bot;
  if (!bot) return;
  const mode = bot.approval || 'ask';
  const free = mode === 'auto';
  sw.setAttribute('aria-checked', String(free));
  const state = free
    ? tr('Бот делает всё сам, без карточек подтверждения. Нажми, чтобы он снова спрашивал.')
    : mode === 'strict'
      ? tr('Бот спрашивает на всё, кроме чтения. Нажми, чтобы дать свободу действий.')
      : tr('Бот спрашивает, прежде чем менять что-то вне своей папки. В своей папке и при чтении не спрашивает. Нажми, чтобы дать свободу действий.');
  sw.title = state;
}

async function setMode(next) {
  const group = $('#mode-switch');
  const bot = inRoom() ? null : botById(S.currentId);
  if (!bot || group.getAttribute('aria-busy') === 'true') return;
  const before = bot.approval || 'ask';
  // «Спрашивать» включает и строгий режим из профиля: повторное нажатие его не сбрасывает.
  if ((before === 'auto') === (next === 'auto')) return;
  group.setAttribute('aria-busy', 'true');
  bot.approval = next;
  paintModeSwitch();
  try {
    await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: { approval: next } });
    await refreshState();
  } catch (err) {
    bot.approval = before;
    toast(err.message);
  } finally {
    group.removeAttribute('aria-busy');
    paintModeSwitch();
  }
}

// Висящие подтверждения и вопросы не должны теряться при переключении чатов и групп.
async function showPending(botId, roomId = null) {
  try {
    const { pending, offers } = await api(`/api/bots/${botId}/pending`);
    // Карточки предложений живут только в личном чате хозяина; после перезагрузки окна их рисуем из ответа сервера тем же кодом, что и по событию.
    if (roomId === null) {
      const shown = (id) => S.cards.get(id)?.isConnected === true;
      for (const ev of window.DeloistUI.offerEventsToShow(botId, offers, shown)) {
        S.cards.delete(ev.offer?.id || ev.proposal?.id);
        if (ev.type === 'specialist_offer') specialistOfferCard(ev);
        else teamProposalCard(ev);
      }
    }
    for (const item of pending) {
      // Карточка живёт в том разговоре, где её задали: группа или личный чат.
      const room = String(item.conv || '').startsWith('room:') ? item.conv.slice(5) : null;
      if (room !== roomId) continue;
      // Переключение чата убирает карточку со страницы, а память о ней остаётся: тогда рисуем заново.
      if (S.cards.get(item.id)?.isConnected) continue;
      S.cards.delete(item.id);
      if (item.kind === 'approval') approvalCard(item);
      else if (item.kind === 'question') questionCard(item);
    }
  } catch {
    // старый сервер без этого маршрута — не страшно
  }
}

function viewBusy() {
  return inRoom() ? S.roomsBusy.has(S.view.id) : S.busy.has(S.currentId);
}

// Пока бот работает, рядом с отправкой всегда есть «Стоп»: текст при этом уходит в очередь.
function setSendState() {
  attachFollowView();
  const btn = $('#btn-send');
  const stop = $('#btn-stop');
  stop.hidden = !viewBusy();
  stop.title = tr('Остановить (Esc)');
  stop.setAttribute('aria-label', tr('Остановить'));
  btn.title = viewBusy() ? tr('В очередь') : tr('Отправить');
  btn.setAttribute('aria-label', btn.title);
  btn.disabled = inRoom() ? !S.view.id : !S.currentId;
  $('#btn-attach').disabled = !attachEndpoint();
}

// Куда класть файлы открытого чата: в папку бота или во все папки участников группы.
function attachEndpoint() {
  if (inRoom()) {
    const room = currentRoom();
    return room && room.botIds?.length && !room.archived ? `/api/rooms/${room.id}/attach` : null;
  }
  return S.currentId ? `/api/bots/${S.currentId}/attach` : null;
}

// Чьё хранилище отдаёт файлы чата: бота или первого участника группы (у всех участников файлы одинаковые).
const attachOwner = () => (inRoom() ? currentRoom()?.botIds?.[0] : S.currentId) || '';

let attachBox = null;
let attachView = '';
// Недописанный текст у каждого бота и команды свой: уходя из чата, поле оставляет черновик ему,
// а в новом чате показывает его собственный.
const drafts = new Map();
// Вложения принадлежат открытому чату: при переходе в другой они не должны уехать не туда.
function attachFollowView() {
  const key = `${S.view.kind}:${S.view.id}`;
  if (key === attachView) return;
  const input = $('#input');
  if (input) {
    if (input.value) drafts.set(attachView, input.value);
    else drafts.delete(attachView);
    input.value = drafts.get(key) || '';
    autosize();
  }
  attachView = key;
  attachBox?.clear();
}

// Файлы сначала уезжают в папку бота, потом сообщение несёт их пути. Не ушло — и текст, и файлы возвращаются в поле.
async function postWithFiles(url, uploadUrl, text, files, input, extra = {}) {
  const view = attachView;
  // input === null — голосовая фраза: поле ввода и вложения остаются как были.
  if (input) { input.value = ''; autosize(); attachBox.clear(); }
  try {
    const attachments = files.length ? await attachBox.upload(files, uploadUrl) : [];
    const body = attachments.length ? { text, attachments } : { text };
    if (extra && extra.voice) body.voice = true;
    const res = await api(url, { method: 'POST', body });
    return { res };
  } catch (err) {
    toast(err.message);
    // Человек ушёл в другой чат, пока файлы ехали: текст и файлы чужому полю не отдаём.
    if (!input) return { error: err };
    if (view !== attachView) {
      // Текст возвращается в черновик того чата, откуда его отправляли.
      if (!drafts.get(view)) drafts.set(view, text);
      return { error: err };
    }
    if (!input.value) input.value = text;
    attachBox.restore(files);
    autosize();
    setSendState();
    return { error: err };
  }
}

// Карточка хода появляется в момент отправки и не ждёт сервера. Не ушло или встало в очередь —
// карточка, которую завела эта отправка, не должна остаться крутиться.
// Подсветка сроков и чипы под полем ввода чата (ChatDates.svelte); поле остаётся тем же <textarea>.
function mountChatDates(input) {
  const chipsHost = $('#in-chips');
  if (!window.DeloistUI?.mountChatDates || !chipsHost) return;
  window.DeloistUI.mountChatDates($('#in-field'), { input, chipsHost });
}

async function sendWithCard(key, url, uploadUrl, text, files, input, opts) {
  const had = turnCard.has(key);
  if (!had) {
    dropEmpty();
    appendTyping();
  }
  const out = await postWithFiles(url, uploadUrl, text, files, input, opts);
  if (had) return out;
  if (out?.error) turnCard.fail(key, out.error.message);
  else if (out?.res?.queued) turnCard.end(key);
  return out;
}

// «Стоп» в карточке: тот же останов, что у квадрата у поля ввода. Ход ещё не начался — карточка просто уходит.
async function stopFromCard() {
  const key = turnKey();
  await stopCurrent();
  if (!viewBusy()) endTurnCard(key);
}

async function sendMessage(opts = {}) {
  const input = $('#input');
  // Голосовая фраза идёт мимо поля ввода: черновик и прикреплённые файлы человека не трогаем.
  const spoken = Boolean(opts && opts.text);
  const text = (spoken ? String(opts.text) : input.value).trim();
  const files = spoken ? [] : attachBox.snapshot();
  // Адрес загрузки фиксируем до первого await: дальше открытый чат может смениться.
  const uploadUrl = attachEndpoint();
  if (!text && !files.length) return;
  if (inRoom()) {
    const room = currentRoom();
    if (!room) return;
    await sendWithCard(`room:${room.id}`, `/api/rooms/${room.id}/messages`, uploadUrl, text, files, spoken ? null : input, opts);
    return;
  }
  if (!S.currentId) return;
  const botId = S.currentId;
  // Услышал — кивнул.
  listenToInput(false);
  for (const avatar of botAvatars(botId)) avatar.react?.('nod');
  await sendWithCard(`bot:${botId}`, `/api/bots/${botId}/messages`, uploadUrl, text, files, spoken ? null : input, opts);
}

async function stopCurrent() {
  if (inRoom()) {
    try {
      await api(`/api/rooms/${S.view.id}/stop`, { method: 'POST' });
    } catch (err) {
      toast(err.message);
    }
    return;
  }
  if (!S.currentId) return;
  const id = S.currentId;
  try {
    await api(`/api/bots/${id}/stop`, { method: 'POST' });
    // Остановил человек: сначала удивление, потом грусть (один таймер, без опроса).
    if (S.busy.has(id)) {
      stoppedByHuman.add(id);
      feelBot(id, 'surprise', 0.8);
      setTimeout(() => feelBot(id, 'sad', 2), 800);
    }
  } catch (err) {
    toast(err.message);
  }
}

// Выделенное в просмотрщике файла с комментарием: кладём в поле ввода, не затирая черновик и не отправляя —
// человек может набрать несколько замечаний подряд и отправить их одним сообщением.
function quoteToChat({ text }) {
  const input = $('#input');
  if (!input || input.disabled) return toast(tr('Сначала открой чат с ботом'));
  const base = input.value.replace(/\s+$/, '');
  input.value = base ? `${base}\n\n${text}\n` : `${text}\n`;
  autosize();
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  toast(tr('Добавил в сообщение'));
}

function autosize() {
  const el = $('#input');
  // Подсветка сроков следит за текстом поля; код меняет его без события input.
  el.dispatchEvent(new Event('cd-sync'));
  el.style.height = 'auto';
  // scrollHeight без рамки, а высота задаётся с рамкой: без поправки появляется полоса прокрутки.
  el.style.height = `${Math.min(200, el.scrollHeight + el.offsetHeight - el.clientHeight)}px`;
}

// ---------------------------------------------------------------------------
// Панель бота
// ---------------------------------------------------------------------------

// У бота панель не закрывается, а сворачивается в полосу значков, как левая колонка.
// Свёрнута она или раскрыта — помнит localStorage; закрывается она только вместе с чатом бота.
const DRAWER_RAIL_KEY = 'deloist-drawer-rail';
const narrowWindow = () => window.matchMedia('(max-width: 820px)').matches;
const drawerRail = () => document.body.classList.contains('drawer-rail');
const drawerShown = () => !$('#drawer').hidden && !drawerRail();

function savedDrawerRail() {
  try {
    return localStorage.getItem(DRAWER_RAIL_KEY) !== 'open';
  } catch {
    return true;
  }
}

function setDrawerRail(rail, save = true) {
  document.body.classList.toggle('drawer-rail', rail);
  if (rail) setDrawerFull(false);
  drawerHead?.update({ rail });
  if (save) {
    try { localStorage.setItem(DRAWER_RAIL_KEY, rail ? 'rail' : 'open'); } catch {}
  }
  renderDrawer();
}

function openDrawer(tab = 'profile') {
  if (!S.currentId) { selectFirst(); if (!S.currentId) return; }
  S.tab = tab;
  $('#drawer').hidden = false;
  document.body.classList.add('drawer-open');
  mountDrawerHead().update({ value: tab });
  setDrawerRail(false);
}

// Открыли чат бота: панель уже на месте — перерисовать для него, нет — поставить так, как её оставили.
// На узком окне панель ложится поверх чата, поэтому сама там не появляется.
function showDrawerForBot() {
  if (!$('#drawer').hidden) {
    renderDrawer();
    return;
  }
  if (narrowWindow()) return;
  $('#drawer').hidden = false;
  document.body.classList.add('drawer-open');
  mountDrawerHead().update({ value: S.tab });
  setDrawerRail(savedDrawerRail(), false);
}

function collapseDrawer() {
  if (narrowWindow()) closeDrawer();
  else setDrawerRail(true);
}

// Шапка панели (вкладки и «Закрыть») — компонент кита; вкладка в S.tab, содержимое рисует renderDrawer.
let drawerHead = null;
function mountDrawerHead() {
  if (drawerHead) return drawerHead;
  drawerHead = window.DeloistUI.mountDrawerHead($('#drawer-head'), {
    value: S.tab,
    full: false,
    rail: drawerRail(),
    onchange: (tab) => {
      S.tab = tab;
      if (drawerRail()) {
        drawerHead.update({ value: tab });
        setDrawerRail(false);
      } else {
        renderDrawer();
      }
    },
    onclose: collapseDrawer,
    onexpand: () => setDrawerRail(false),
    onfull: setDrawerFull,
  });
  return drawerHead;
}

// «Развернуть»: панель ложится поверх чата до боковой колонки, чат под ней не сжимается.
function setDrawerFull(full) {
  document.body.classList.toggle('drawer-full', full);
  drawerHead?.update({ full });
}

function closeDrawer() {
  $('#drawer').hidden = true;
  document.body.classList.remove('drawer-open');
  setDrawerFull(false);
  clearPicker();
}

// Состав команды из списка слева: бота тащат на строку команды или в «Без команды».
// Из одной команды в другую — перенос; с Ctrl бот остаётся и в старой.
async function setTeamMembers(roomId, botIds) {
  await api(`/api/rooms/${roomId}`, { method: 'PATCH', body: { botIds } });
}

async function moveBotToTeam(botId, toId, fromId, copy = false) {
  const to = toId ? roomById(toId) : null;
  const from = fromId ? roomById(fromId) : null;
  const name = botById(botId)?.name || '';
  const leaving = from && !copy && from.id !== to?.id;
  if (leaving && from.botIds.length <= 1) {
    toast(tr('В команде нужен хотя бы один бот'));
    return;
  }
  try {
    if (to && !to.botIds.includes(botId)) await setTeamMembers(to.id, [...to.botIds, botId]);
    if (leaving) await setTeamMembers(from.id, from.botIds.filter((id) => id !== botId));
    await refreshState();
    if (to) toast(tr('{name} в команде «{team}»', { name, team: to.name }));
    else if (from) toast(tr('{name} больше не в команде «{team}»', { name, team: from.name }));
  } catch (err) {
    toast(err.message);
  }
}

function bindTeamDrop() {
  const list = $('#bot-list');
  if (PANE || !list) return;
  let drag = null;
  let lit = null;
  const light = (el) => {
    if (lit === el) return;
    lit?.classList.remove('sb-drop-on');
    lit = el;
    lit?.classList.add('sb-drop-on');
  };
  // Куда можно бросить: строка другой команды, где этого бота ещё нет, или «Без команды» для участника.
  const targetAt = (el) => {
    if (!drag || !el?.closest) return null;
    const team = el.closest('#bot-list .sb-row[data-team]');
    if (team) {
      const room = roomById(team.dataset.team);
      return room && !room.archived && !room.botIds.includes(drag.id) ? team : null;
    }
    const loose = el.closest('#bot-list [data-drop="loose"]');
    return loose && drag.from ? loose : null;
  };
  list.addEventListener('dragstart', (e) => {
    const row = e.target.closest?.('.sb-row[data-id]');
    drag = row && !row.dataset.team && row.dataset.kind !== 'room' ? { id: row.dataset.id, from: row.dataset.parent || '' } : null;
  });
  list.addEventListener('dragover', (e) => {
    const target = targetAt(e.target);
    light(target);
    if (!target) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = drag.from && !e.ctrlKey ? 'move' : 'copy';
  });
  list.addEventListener('dragleave', (e) => {
    if (!list.contains(e.relatedTarget)) light(null);
  });
  list.addEventListener('drop', (e) => {
    const target = targetAt(e.target);
    const was = drag;
    light(null);
    drag = null;
    if (!target) return;
    e.preventDefault();
    e.stopPropagation();
    moveBotToTeam(was.id, target.dataset.team || '', was.from, e.ctrlKey);
  });
  document.addEventListener('dragend', () => {
    light(null);
    drag = null;
  });
}

// Файл или папку из вкладки «Файлы» бросают в чат: ссылка на них встаёт в сообщение.
// В поле ввода её вставляет сам браузер (text/plain) туда, куда бросили; в остальной чат — в конец сообщения.
const FILE_REF_MIME = 'application/x-deloist-file';

function bindFileRefDrop() {
  const chat = $('#chat');
  const input = $('#input');
  if (!chat || !input) return;
  const carries = (e) => [...(e.dataTransfer?.types || [])].includes(FILE_REF_MIME);
  const lit = (on) => chat.classList.toggle('file-ref-drop', on);
  chat.addEventListener('dragover', (e) => {
    if (!carries(e) || input.disabled) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    lit(true);
  });
  chat.addEventListener('dragleave', (e) => {
    if (!chat.contains(e.relatedTarget)) lit(false);
  });
  chat.addEventListener('drop', (e) => {
    lit(false);
    if (!carries(e) || input.disabled) return;
    let ref = '';
    try {
      ref = JSON.parse(e.dataTransfer.getData(FILE_REF_MIME)).ref || '';
    } catch {}
    if (!ref) return;
    if (e.target === input) {
      // Браузер сам вставит текст в место броска; поле только подровнять по высоте.
      setTimeout(autosize);
      return;
    }
    e.preventDefault();
    const base = input.value.replace(/\s+$/, '');
    input.value = base ? `${base} ${ref} ` : `${ref} `;
    autosize();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  });
  document.addEventListener('dragend', () => lit(false));
}

// Ширину панели тянут за левый край; она в --drawer-w и в localStorage. Двойной щелчок — обычная ширина.
const DRAWER_W_KEY = 'deloist-drawer-w';
const DRAWER_MIN = 340;
const DRAWER_CHAT_MIN = 360;

function drawerMaxWidth() {
  const side = $('#sidebar')?.getBoundingClientRect().right || 0;
  return Math.max(DRAWER_MIN, window.innerWidth - side - DRAWER_CHAT_MIN - 24);
}

function setDrawerWidth(px, save = true) {
  const body = document.body;
  if (!px) {
    body.style.removeProperty('--drawer-w');
  } else {
    px = Math.round(Math.min(drawerMaxWidth(), Math.max(DRAWER_MIN, px)));
    body.style.setProperty('--drawer-w', `${px}px`);
  }
  $('#drawer-grip')?.setAttribute('aria-valuenow', String(Math.round($('#drawer')?.offsetWidth || px || 0)));
  if (!save) return;
  try {
    if (px) localStorage.setItem(DRAWER_W_KEY, String(px));
    else localStorage.removeItem(DRAWER_W_KEY);
  } catch {}
}

function initDrawerResize() {
  const grip = $('#drawer-grip');
  if (!grip) return;
  grip.setAttribute('aria-label', tr('Ширина панели бота'));
  grip.title = tr('Ширина панели бота');
  let saved = 0;
  try { saved = Number(localStorage.getItem(DRAWER_W_KEY)) || 0; } catch {}
  if (saved) setDrawerWidth(saved, false);

  let dragging = false;
  grip.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || document.body.classList.contains('drawer-full')) return;
    dragging = true;
    try { grip.setPointerCapture(e.pointerId); } catch {}
    document.body.classList.add('drawer-drag');
    e.preventDefault();
  });
  grip.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    const right = $('#drawer').getBoundingClientRect().right;
    setDrawerWidth(right - e.clientX, false);
  });
  const stop = () => {
    if (!dragging) return;
    dragging = false;
    document.body.classList.remove('drawer-drag');
    setDrawerWidth($('#drawer').offsetWidth);
  };
  grip.addEventListener('pointerup', stop);
  grip.addEventListener('pointercancel', stop);
  grip.addEventListener('lostpointercapture', stop);
  grip.addEventListener('dblclick', () => setDrawerWidth(0));
  grip.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 64 : 16;
    const now = $('#drawer').offsetWidth;
    let next = 0;
    if (e.key === 'ArrowLeft') next = now + step;
    else if (e.key === 'ArrowRight') next = now - step;
    else if (e.key === 'Home') next = drawerMaxWidth();
    else if (e.key === 'End') next = DRAWER_MIN;
    else return;
    e.preventDefault();
    setDrawerWidth(next);
  });
  // Окно сузили — сохранённая ширина не должна съесть чат.
  window.addEventListener('resize', () => {
    const w = parseInt(document.body.style.getPropertyValue('--drawer-w'), 10);
    if (w && w > drawerMaxWidth()) setDrawerWidth(w, false);
  });
  // Esc из «Развернуть» возвращает панель рядом с чатом; диалоги закрываются своим Esc.
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || e.defaultPrevented || !document.body.classList.contains('drawer-full')) return;
    if (document.querySelector('dialog[open]')) return;
    e.preventDefault();
    setDrawerFull(false);
  });
}

function clearPicker() {
  for (const avatar of S.picker) avatar.destroy();
  S.picker = [];
}

// Вкладка «Профиль»: разделы рисует DrawerProfile.svelte, здесь — данные и обработчики.
let profileView = null;
// Вкладки «Память», «Цели», «Файлы»: компоненты ящика, пересоздаются вместе с renderDrawer.
let tabView = null;

function renderProfile(bot, body) {
  let look = {};
  profileView = window.DeloistUI.mountDrawerProfile(body, {
    bot,
    skills: S.skills,
    levels: S.levels,
    petBlocked: S.bots.filter((b) => b.pet).length >= MAX_PETS,
    folder: `data/workspaces/${bot.id}`,
    mountLook: (host, onpick) => {
      look = mountLookPicker(host, bot, { onpick });
    },
    mountModel: (host, value, onselect) => {
      const handle = mountModelPicker(host, { value, onselect, hideLabel: true });
      return { set: (next) => handle.picker.update({ value: next }) };
    },
    contextInfo: async ({ driver, model }) => {
      try {
        return await api(`/api/context?${new URLSearchParams({ botId: bot.id, driver, model })}`);
      } catch {
        return null;
      }
    },
    onpet: (on) => setPet(bot.id, on),
    ongoogle: async () => {
      try {
        await api(`/api/bots/${bot.id}/browser/sign-in`, { method: 'POST', body: { site: 'google' } });
        toast(tr('Окно браузера открыто. Войди в Google и нажми «Готово» в карточке.'));
      } catch (err) {
        toast(err.message);
      }
    },
    onrevokesite: async (site) => {
      try {
        const filtered = (bot.browserSites || []).filter((s) => s !== site);
        await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: { browserSites: filtered } });
        await refreshState();
        renderDrawer();
        toast(tr('Разрешение забрал'));
      } catch (err) {
        toast(err.message);
      }
    },
    // Автосохранение: панель зовёт это после каждой правки. Тост только при ошибке; панель не перерисовываем,
    // иначе пропадут фокус и каретка. Ответ false — «не сохранилось», панель покажет это в шапке.
    onsave: async (fields) => {
      try {
        const patch = {
          ...fields,
          emoji: '',
          skin: look.skin ?? bot.skin ?? 0,
        };
        await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: patch });
        await refreshState();
        if (S.currentId === bot.id) {
          if (patch.name) $('#head-name').textContent = patch.name;
          $('#head-role').textContent = patch.role;
        }
        return true;
      } catch (err) {
        toast(err.message);
        return false;
      }
    },
    // Подтверждение — окно кита в самой панели (DrawerProfile), здесь только удаление.
    ondelete: async () => {
      try {
        await api(`/api/bots/${bot.id}`, { method: 'DELETE' });
        await refreshState();
        showTeam();
        toast(tr('Бот удалён'));
      } catch (err) {
        toast(err.message);
      }
    },
  });
  loadActivity(bot.id);
}

// Выбор внешности в ящике: большой живой просмотр и двенадцать шаров, у каждого свои глаза. Возвращает выбор для сохранения.
// Шары в сетке не анимируются без наведения: их двенадцать.
function mountLookPicker(host, bot, { prefix = 'pf', owned = S.picker, size = 168, onpick } = {}) {
  const orbs = window.CrewAvatar ? CrewAvatar.ORBS : [];
  const choice = { skin: bot.skin ?? 0 };
  const buttons = orbs
    .map((orb, i) => `<button type="button" class="look-btn" data-skin="${i}" role="radio" title="${escapeHtml(tr(orb.name))}" aria-label="${escapeHtml(tr(orb.name))}"><span class="avatar live look-host"></span></button>`)
    .join('');
  host.innerHTML = `
    <div class="look-picker">
      <div class="avatar live look-big" id="${prefix}-avatar"></div>
      <div class="look-group"><span class="look-label" id="${prefix}-orb-name"></span><div class="look-row look-row-4" role="radiogroup" aria-label="${tr('Шар')}">${buttons}</div></div>
    </div>`;
  const big = window.CrewAvatar ? CrewAvatar.mount($(`#${prefix}-avatar`, host), { size, ...lookOf(bot), skin: choice.skin }) : null;
  if (!big) {
    $('.look-picker', host).hidden = true;
    return choice;
  }
  owned.push(big);
  const minis = $$('.look-btn', host).map((btn) => {
    const skin = Number(btn.dataset.skin);
    owned.push(CrewAvatar.mount($('.look-host', btn), { size: 34, still: true, ...lookOf(bot), skin }));
    btn.addEventListener('click', () => {
      choice.skin = skin;
      refresh();
      onpick?.(skin);
    });
    return { btn, skin };
  });
  function refresh() {
    big.set({ skin: choice.skin });
    for (const { btn, skin } of minis) {
      btn.classList.toggle('on', skin === choice.skin);
      btn.setAttribute('aria-checked', String(skin === choice.skin));
    }
    $(`#${prefix}-orb-name`, host).textContent = tr(orbs[choice.skin]?.name || '');
  }
  refresh();
  return choice;
}

function renderDrawer() {
  const bot = botById(S.currentId);
  if (!bot) return;
  const body = $('#drawer-body');
  clearPicker();
  profileView?.destroy();
  profileView = null;
  tabView?.destroy();
  tabView = null;
  body.innerHTML = '';
  S.drawerMemoryLoad = null;
  // Свёрнутая панель — только значки разделов: содержимое не рисуем.
  if (drawerRail()) return;

  if (S.tab === 'routines') renderDrawerRoutines(bot, body);

  // Профиль — форма настроек: карточки на фоне окна.
  body.classList.toggle('is-forms', S.tab === 'profile');
  if (S.tab === 'profile') {
    renderProfile(bot, body);
  }

  // Память, цели и файлы рисуют компоненты ящика (ui/src/components/drawer); здесь — данные и перерисовка.
  if (S.tab === 'memory') {
    tabView = window.DeloistUI.mountDrawerMemory(body, {
      bot,
      api,
      // SSE «память изменилась» зовёт перечитывание открытой вкладки.
      registerReload: (reload) => { S.drawerMemoryLoad = reload; },
    });
  }

  if (S.tab === 'goals') {
    tabView = window.DeloistUI.mountDrawerGoals(body, {
      bot,
      goals: (S.goals || []).filter((g) => g.botId === bot.id).slice().reverse(),
      checkin: (S.checkins || []).find((c) => c.botId === bot.id) || null,
      api,
      onchanged: () => refreshState().then(() => {
        if (drawerShown() && S.tab === 'goals') renderDrawer();
      }),
    });
  }

  if (S.tab === 'files') {
    const initial = S.openFile || '';
    S.openFile = '';
    tabView = window.DeloistUI.mountDrawerFiles(body, { bot, api, initial, onquote: quoteToChat });
  }
}

// Вкладка «Рутины»: что этот бот делает сам по расписанию. Те же строки, что в окне «Рутины»;
// новая — той же формой, где бот уже выбран. Боту можно и сказать в чате — рутина появится здесь.
function renderDrawerRoutines(bot, body) {
  const mine = ordinaryRoutines().filter((r) => r.botId === bot.id);
  const head = document.createElement('div');
  head.className = 'drawer-rt-head';
  head.innerHTML = `
    <p class="note">${mine.length
      ? tr('Что {name} делает сам по расписанию.', { name: escapeHtml(bot.name) })
      : tr('Рутин пока нет. Добавь здесь или скажи боту в чате, например: «каждое утро в 9 собирай новости».')}</p>
    <button type="button" class="primary" id="drawer-rt-new">${tr('Новая рутина')}</button>`;
  $('#drawer-rt-new', head).addEventListener('click', () => openRoutineFormFor(bot.id));
  body.append(head);
  if (!mine.length) return;
  const list = document.createElement('div');
  list.className = 'drawer-rt-list';
  const rerender = () => {
    if (drawerShown() && S.tab === 'routines') renderDrawer();
    if (routinesOpen()) renderRoutines();
  };
  for (const r of mine) list.append(routineRow(r, rerender));
  body.append(list);
}

function openRoutineFormFor(botId) {
  openRoutines('routines');
  $('#rt-new').open = true;
  $('#routine-bot').value = botId;
  $('#form-routine').title.focus();
}

// ---------------------------------------------------------------------------
// Память: журнал правок и общая память
// ---------------------------------------------------------------------------

function memoryWho(who) {
  if (!who) return '';
  if (who.kind === 'bot') return who.name || tr('Бот');
  if (who.kind === 'human') return tr('Ты');
  if (who.kind === 'doctor') return tr('Доктор памяти');
  if (who.kind === 'account') return who.name || tr('Аккаунт');
  return who.name || '';
}

const MEMORY_ACTIONS = {
  add: () => tr('Дописал в память'),
  edit: () => tr('Поправил память'),
  doctor: () => tr('Навёл порядок'),
  undo: () => tr('Отменил правку'),
  remove: () => tr('Убрал из памяти'),
};

// Один список для памяти бота и общей памяти. onUndo(id) сам перечитывает экран.
function memoryJournal(entries, onUndo) {
  const box = document.createElement('div');
  box.className = 'mem-journal-list';
  if (!Array.isArray(entries) || !entries.length) {
    const empty = document.createElement('p');
    empty.className = 'note';
    empty.textContent = tr('Пока без правок.');
    box.append(empty);
    return box;
  }
  for (const e of entries) {
    const row = document.createElement('div');
    row.className = 'jr';
    const head = document.createElement('div');
    head.className = 'jr-head';
    const when = document.createElement('span');
    when.className = 'jr-when';
    when.textContent = whenText(e.at);
    const who = document.createElement('span');
    who.className = 'jr-bot';
    who.textContent = memoryWho(e.who);
    head.append(when, who);
    if (e.undoable) {
      const undo = document.createElement('button');
      undo.type = 'button';
      undo.className = 'ghost mem-undo';
      undo.textContent = tr('Отменить');
      undo.addEventListener('click', async () => {
        undo.disabled = true;
        try {
          await onUndo(e.id);
        } catch (err) {
          undo.disabled = false;
          toast(err.message);
        }
      });
      head.append(undo);
    }
    row.append(head);
    const note = document.createElement('div');
    note.className = 'jr-title';
    note.textContent = e.note || (MEMORY_ACTIONS[e.action] ? MEMORY_ACTIONS[e.action]() : '');
    row.append(note);
    const lines = [
      ...(e.added || []).slice(0, 3).map((t) => ['+ ', t]),
      ...(e.removed || []).slice(0, 3).map((t) => ['\u2212 ', t]),
      ...(e.edited || []).slice(0, 3).map((t) => ['~ ', t]),
    ];
    for (const [sign, text] of lines) {
      const line = document.createElement('div');
      line.className = 'mem-line';
      line.textContent = sign + text;
      row.append(line);
    }
    box.append(row);
  }
  return box;
}

const MEMORY_KINDS = ['user', 'feedback', 'project', 'reference'];

const memoryKindLabel = (kind) =>
  ({
    user: tr('О человеке'),
    feedback: tr('Поправка'),
    project: tr('Дела и решения'),
    reference: tr('Ссылки и кабинеты'),
  }[kind] || kind);

const memoryFormOpen = (root) => Boolean(root?.querySelector('.mem-form'));

// Форма факта: и для нового, и для правки. Пока она открыта, экран не перерисовывается.
function factForm(fact, ctx) {
  const form = document.createElement('form');
  form.className = 'mem-form';
  const kinds = MEMORY_KINDS.map((k) => `<option value="${k}"${(fact?.kind || 'user') === k ? ' selected' : ''}>${escapeHtml(memoryKindLabel(k))}</option>`).join('');
  form.innerHTML = `
    <label class="field"><span>${tr('Заголовок')}</span><input name="title" value="${escapeHtml(fact?.title || '')}" required></label>
    <label class="field"><span>${tr('Текст')}</span><textarea name="body" rows="4">${escapeHtml(fact?.body || '')}</textarea></label>
    <div class="row">
      <label class="field"><span>${tr('Тип')}</span><select name="kind">${kinds}</select></label>
      <label class="field"><span>${tr('Про что (через запятую)')}</span><input name="entities" value="${escapeHtml((fact?.entities || []).join(', '))}"></label>
    </div>
    <div class="modal-actions">
      <button type="button" class="ghost" data-cancel>${tr('Отмена')}</button>
      <button type="submit" class="primary">${tr('Сохранить')}</button>
    </div>`;
  $('[data-cancel]', form).addEventListener('click', () => {
    form.remove();
    ctx.reload();
  });
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const body = {
      title: form.title.value.trim(),
      body: form.body.value.trim(),
      kind: form.kind.value,
      entities: form.entities.value.split(',').map((x) => x.trim()).filter(Boolean),
    };
    try {
      if (fact) await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'PATCH', body });
      else await api('/api/memory/facts', { method: 'POST', body: { ...body, scope: ctx.scope } });
      toast(tr('Факт сохранён'));
      form.remove();
      await ctx.reload();
    } catch (err) {
      toast(err.message);
    }
  });
  return form;
}

function factRow(fact, ctx) {
  const row = document.createElement('div');
  row.className = 'mem-fact';
  const head = document.createElement('div');
  head.className = 'mem-fact-head';
  const id = document.createElement('span');
  id.className = 'mem-fact-id';
  id.textContent = `[${fact.id}]`;
  const title = document.createElement('span');
  title.className = 'mem-fact-title';
  title.textContent = fact.title;
  head.append(id, title);
  row.append(head);

  if (fact.body) {
    const text = document.createElement('div');
    text.className = 'mem-fact-body';
    text.textContent = fact.body;
    text.title = tr('Нажми, чтобы развернуть');
    text.addEventListener('click', () => text.classList.toggle('open'));
    row.append(text);
  }
  if ((fact.entities || []).length) {
    const ents = document.createElement('div');
    ents.className = 'mem-ents';
    for (const name of fact.entities) {
      const chip = document.createElement('span');
      chip.className = 'mem-ent';
      chip.textContent = name;
      ents.append(chip);
    }
    row.append(ents);
  }
  if ((fact.links || []).length) {
    const links = document.createElement('div');
    links.className = 'mem-fact-links';
    links.textContent = tr('связано: {ids}', { ids: fact.links.join(', ') });
    row.append(links);
  }

  const actions = document.createElement('div');
  actions.className = 'mem-fact-actions';
  const small = (label, fn) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'ghost';
    b.textContent = label;
    b.addEventListener('click', fn);
    return b;
  };
  actions.append(
    small(tr('Изменить'), () => {
      if (memoryFormOpen(ctx.root)) return;
      row.replaceWith(factForm(fact, ctx));
    }),
    small(tr('Удалить'), async () => {
      try {
        await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'DELETE' });
        toast(tr('Удалено. Отменить можно в журнале'));
        await ctx.reload();
      } catch (err) {
        toast(err.message);
      }
    }),
  );
  const move = async (scope, done) => {
    try {
      await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'PATCH', body: { scope } });
      toast(done);
      await ctx.reload();
    } catch (err) {
      toast(err.message);
    }
  };
  if (ctx.scope === 'shared') {
    if (S.bots.length) {
      const pick = document.createElement('select');
      pick.className = 'mem-move';
      pick.setAttribute('aria-label', tr('В память бота'));
      pick.innerHTML = `<option value="">${tr('В память бота')}</option>${S.bots.map((b) => `<option value="${escapeHtml(b.id)}">${escapeHtml(b.name)}</option>`).join('')}`;
      pick.addEventListener('change', () => {
        if (pick.value) move(pick.value, tr('Перенёс в память бота'));
      });
      actions.append(pick);
    }
  } else {
    actions.append(small(tr('В общую память'), () => move('shared', tr('Перенёс в общую память'))));
  }
  row.append(actions);
  return row;
}

// Один список для общей памяти и памяти бота: группы по типу, счётчик, добавление.
// ctx: {scope, used, limit, reload}
function factList(facts, ctx) {
  const root = document.createElement('div');
  root.className = 'mem-fact-list';
  ctx = { ...ctx, root };
  const bar = document.createElement('div');
  bar.className = 'mem-foot';
  const count = document.createElement('span');
  count.className = 'note';
  count.textContent = ctx.limit
    ? tr('{n} из {limit} знаков', { n: numberRu(ctx.used || 0), limit: numberRu(ctx.limit) })
    : '';
  const add = document.createElement('button');
  add.type = 'button';
  add.className = 'ghost';
  add.textContent = tr('Добавить факт');
  const slot = document.createElement('div');
  add.addEventListener('click', () => {
    if (memoryFormOpen(root)) return;
    slot.append(factForm(null, ctx));
    $('input', slot)?.focus();
  });
  bar.append(count, add);
  root.append(bar, slot);

  const list = Array.isArray(facts) ? facts : [];
  if (!list.length) {
    const empty = document.createElement('p');
    empty.className = 'note';
    empty.textContent = tr('Фактов пока нет.');
    root.append(empty);
  }
  for (const kind of MEMORY_KINDS) {
    const group = list.filter((f) => f.kind === kind);
    if (!group.length) continue;
    const title = document.createElement('h4');
    title.className = 'mem-group';
    title.textContent = `${memoryKindLabel(kind)} · ${group.length}`;
    root.append(title);
    for (const f of group) root.append(factRow(f, ctx));
  }
  return root;
}

const sharedMemoryOpen = () => $('#modal-memory').open;

async function loadSharedMemory() {
  const box = $('#sm-facts');
  try {
    const data = await api('/api/memory');
    if (!sharedMemoryOpen()) return;
    const limits = data.limits || {};
    // Открытую форму не трогаем: чужая правка не должна стереть набранный текст.
    if (!memoryFormOpen(box)) {
      box.replaceChildren(factList(data.facts, { scope: 'shared', used: data.used, limit: limits.shared, reload: loadSharedMemory }));
    }
    $('#sm-index').textContent = data.index || '';
    $('#sm-index-count').textContent = limits.index
      ? tr('{n} из {limit} знаков', { n: numberRu((data.index || '').length), limit: numberRu(limits.index) })
      : '';
    const doctor = data.doctor || {};
    const parts = [doctor.lastRun ? tr('Последняя проверка: {date}', { date: whenText(doctor.lastRun) }) : tr('Ещё не проверялась')];
    if (doctor.lastError && Number(doctor.lastErrorAt) > Number(doctor.lastRun || 0)) parts.push(doctor.lastError);
    $('#sm-doctor').textContent = parts.join(' · ');
    const run = $('#sm-doctor-run');
    run.disabled = Boolean(doctor.running) || S.memoryRunning;
    run.textContent = run.disabled ? tr('Проверяю…') : tr('Проверить сейчас');
    $('#sm-journal').replaceChildren(memoryJournal(data.journal, async (id) => {
      await api(`/api/memory/undo/${encodeURIComponent(id)}`, { method: 'POST' });
      await loadSharedMemory();
    }));
  } catch (err) {
    toast(err.message);
  }
}

// «Использование и лимиты»: экран монтируется при каждом открытии и снимается при закрытии,
// чтобы в покое не держать запросов и состояния.
let usageView = null;
function openUsage() {
  const ui = window.DeloistUI;
  if (!ui?.mountUsageScreen) return;
  usageView?.destroy();
  usageView = ui.mountUsageScreen($('#usage-screen'), {
    onOpen: (receipt) => {
      $('#modal-usage').close();
      if (receipt.botId) selectBot(receipt.botId);
    },
  });
  openModal('modal-usage');
}

function openSharedMemory() {
  $('#sm-facts').replaceChildren();
  openModal('modal-memory');
  loadSharedMemory();
}

function bindSharedMemory() {
  $('#btn-memory').addEventListener('click', openSharedMemory);
  $('#sm-doctor-run').addEventListener('click', async () => {
    const run = $('#sm-doctor-run');
    S.memoryRunning = true;
    run.disabled = true;
    run.textContent = tr('Проверяю…');
    try {
      const { report } = await api('/api/memory/doctor', { method: 'POST' });
      if (report?.note) toast(report.note);
    } catch (err) {
      toast(err.message);
    } finally {
      S.memoryRunning = false;
      if (sharedMemoryOpen()) loadSharedMemory();
    }
  });
}

// ---------------------------------------------------------------------------
// Навыки, MCP-серверы, файл команды
// ---------------------------------------------------------------------------

function renderSkills() {
  const list = $('#skills-list');
  if (!list) return;
  updateSettingsLinks();
  list.innerHTML = '';
  if (!S.skills.length) {
    list.innerHTML = `<p class="note">${tr('Пока ни одного. Навык можно добавить здесь или получить вместе с файлом команды.')}</p>`;
    return;
  }
  for (const s of S.skills) {
    const el = document.createElement('div');
    el.className = 'mcp-item';
    el.innerHTML = `
      <span class="m-name">${escapeHtml(s.name)}</span>
      ${s.origin === 'learned' ? `<span class="m-learned">${escapeHtml(botById(s.botId) ? tr('выучил {name}', { name: botById(s.botId).name }) : tr('выучен ботом'))}</span>` : ''}
      <span class="m-cmd" title="${escapeHtml(s.description || '')}">${escapeHtml(s.description || '')}</span>
      <div class="m-actions">${s.origin === 'learned' || (s.previous || []).length ? `<button type="button" class="ghost m-undo">${tr('Отменить правку')}</button>` : ''}<button class="icon-btn" data-del title="${tr('Удалить навык')}" aria-label="${tr('Удалить навык')}">${ICONS.close}</button></div>`;
    $('.m-undo', el)?.addEventListener('click', async (e) => {
      const btn = e.currentTarget;
      btn.disabled = true;
      try {
        await undoSkill(s.id);
      } catch (err) {
        btn.disabled = false;
        toast(err.message);
      }
    });
    $('[data-del]', el).addEventListener('click', async () => {
      if (!confirm(tr('Удалить навык «{name}»? Он отвяжется от ботов.', { name: s.name }))) return;
      try {
        await api(`/api/skills/${s.id}`, { method: 'DELETE' });
        await refreshState();
        renderSkills();
        toast(tr('Навык удалён'));
      } catch (err) {
        toast(err.message);
      }
    });
    list.append(el);
  }
}

// Режим «Разработка Deloist»: checkout и боты, которым он открыт.
let selfDev = null;

async function loadSelfDev() {
  try {
    selfDev = await api('/api/selfdev');
  } catch (err) {
    selfDev = null;
  }
  renderSelfDev();
}

function renderSelfDev() {
  const status = $('#selfdev-status');
  const list = $('#selfdev-bots');
  if (!status || !list) return;
  const input = $('#selfdev-checkout');
  if (document.activeElement !== input) input.value = selfDev?.checkout || '';
  input.placeholder = selfDev?.detected || 'C:\\Projects\\deloist';
  if (!selfDev) {
    status.textContent = '';
  } else if (selfDev.problem) {
    status.textContent = tr('Папка не подходит: {problem}', { problem: selfDev.problem });
  } else if (selfDev.effective) {
    status.textContent = tr('Исходники: {path}', { path: selfDev.effective });
  } else {
    status.textContent = tr('Исходники не найдены: укажи папку или скачай их.');
  }
  list.innerHTML = '';
  const on = new Set(selfDev?.bots || []);
  for (const bot of S.bots) {
    const el = document.createElement('label');
    el.className = 'mcp-item';
    el.innerHTML = `<input type="checkbox" ${on.has(bot.id) ? 'checked' : ''} ${selfDev?.effective ? '' : 'disabled'} /><span class="m-name">${escapeHtml(bot.name)}</span><span class="m-cmd">${escapeHtml(tr('режим разработки'))}</span>`;
    $('input', el).addEventListener('change', async (e) => {
      try {
        selfDev = await api(`/api/selfdev/bots/${bot.id}`, { method: 'POST', body: { on: e.target.checked } });
        renderSelfDev();
      } catch (err) {
        e.target.checked = !e.target.checked;
        toast(err.message);
      }
    });
    list.append(el);
  }
}

function bindSelfDev() {
  $('#selfdev-save')?.addEventListener('click', async () => {
    try {
      selfDev = await api('/api/selfdev', { method: 'PUT', body: { checkout: $('#selfdev-checkout').value.trim() } });
      renderSelfDev();
      toast(tr('Папка исходников сохранена'));
    } catch (err) {
      toast(err.message);
    }
  });
  $('#selfdev-preset')?.addEventListener('click', async () => {
    try {
      const res = await api('/api/selfdev/preset', { method: 'POST' });
      selfDev = res.selfdev;
      await refreshState();
      renderSkills();
      renderSelfDev();
      toast(tr('Бот «{name}» создан', { name: res.bot.name }));
    } catch (err) {
      toast(err.message);
    }
  });
  $('#selfdev-clone')?.addEventListener('click', async (e) => {
    const url = $('#selfdev-url').value.trim();
    const dest = $('#selfdev-dest').value.trim();
    if (!url || !dest) {
      toast(tr('Нужны адрес репозитория и папка'));
      return;
    }
    const btn = e.currentTarget;
    btn.disabled = true;
    toast(tr('Скачиваю исходники…'));
    try {
      selfDev = await api('/api/selfdev/clone', { method: 'POST', body: { url, dest } });
      renderSelfDev();
      toast(tr('Исходники скачаны'));
    } catch (err) {
      toast(err.message);
    } finally {
      btn.disabled = false;
    }
  });
}

// Что показать в списке вместо команды: у HTTP-сервера только адрес.
// Параметры запроса прячем: в них может лежать ключ доступа.
function mcpAddress(s) {
  if (s.url) return String(s.url).replace(/\?.*$/, '');
  return [s.command, ...(s.args || [])].join(' ');
}

function mcpBadge(s) {
  const transport = s.url ? 'http' : 'stdio';
  return `<span class="m-cmd" data-testid="mcp-badge" title="${tr('Транспорт: {transport}', { transport })}" style="flex:0 0 auto;padding:1px 7px;border:1px solid var(--color-border-default);border-radius:999px;font-size:10.5px;letter-spacing:.04em;text-transform:uppercase">${transport}</span>`;
}

function renderMcp() {
  const list = $('#mcp-list');
  if (!list) return;
  updateSettingsLinks();
  list.innerHTML = '';
  if (!S.mcp.length) {
    list.innerHTML = `<p class="note">${tr('Пока ни одного сервера.')}</p>`;
    return;
  }
  for (const s of S.mcp) {
    const el = document.createElement('div');
    const address = mcpAddress(s);
    el.className = `mcp-item${s.enabled ? '' : ' off'}`;
    el.innerHTML = `
      <span class="m-name">${escapeHtml(s.name)}</span>
      ${mcpBadge(s)}
      <span class="m-cmd" title="${escapeHtml(address)}">${escapeHtml(address)}</span>
      ${s.login ? `<span class="m-login" data-testid="mcp-login-state" data-state="${s.login.state}">${s.login.state === 'connected' ? tr('Вход выполнен') : tr('Нужно войти')}</span>` : ''}
      <div class="m-actions">
        ${s.login ? `<button type="button" class="ghost rt-run" data-login>${s.login.state === 'connected' ? tr('Войти заново') : tr('Войти')}</button>` : ''}
        <button type="button" class="ghost rt-run" data-check>${tr('Проверить')}</button>
        <label class="switch"><input type="checkbox" ${s.enabled ? 'checked' : ''}><span></span></label>
        <button class="icon-btn" title="${tr('Удалить сервер')}" aria-label="${tr('Удалить сервер')}">${ICONS.close}</button>
      </div>`;
    $('[data-login]', el)?.addEventListener('click', () => startMcpLogin(s, el));
    $('[data-check]', el).addEventListener('click', async (e) => {
      const btn = e.currentTarget;
      btn.disabled = true;
      btn.textContent = tr('Стартую…');
      try {
        const { tools } = await api(`/api/mcp/${s.id}/check`, { method: 'POST' });
        toast(tools.length ? tr('Инструменты: {tools}', { tools: tools.join(', ') }) : tr('Сервер ответил, но инструментов нет'), 6000);
      } catch (err) {
        toast(tr('Не подключился: {error}', { error: trServer(err.message) }), 8000);
      } finally {
        btn.disabled = false;
        btn.textContent = tr('Проверить');
      }
    });
    $('.switch input', el).addEventListener('change', async (e) => {
      try {
        await api(`/api/mcp/${s.id}`, { method: 'PATCH', body: { enabled: e.target.checked } });
        s.enabled = e.target.checked;
        el.classList.toggle('off', !s.enabled);
      } catch (err) {
        toast(err.message);
        e.target.checked = s.enabled;
      }
    });
    $('.m-actions .icon-btn', el).addEventListener('click', async () => {
      if (!confirm(tr('Удалить сервер «{name}»?', { name: s.name }))) return;
      try {
        await api(`/api/mcp/${s.id}`, { method: 'DELETE' });
        await refreshState();
        renderMcp();
      } catch (err) {
        toast(err.message);
      }
    });
    list.append(el);
  }
}

// Вход в свой HTTP-сервер по OAuth: ссылка открывается в браузере, а если у сервера нет регистрации
// клиентов, под строкой сервера появляется форма для client ID (и секрета, если он есть).
async function startMcpLogin(s, el, manual) {
  try {
    const data = await api(`/api/mcp/${s.id}/oauth/start`, { method: 'POST', body: { open: false, ...manual } });
    el.querySelector('.mcp-login-form')?.remove();
    toast(data.authHost ? tr('Откроется вход на {host}. Вернитесь сюда: статус обновится сам.', { host: data.authHost }) : tr('Окно входа открылось в браузере. Вернитесь сюда: статус обновится сам.'), 8000);
    window.open(data.url, '_blank', 'noopener');
  } catch (err) {
    if (err.data?.needsClientId) {
      showMcpClientForm(s, el, err.data.authHost);
      return;
    }
    toast(trServer(err.message), 8000);
  }
}

function showMcpClientForm(s, el, host) {
  el.querySelector('.mcp-login-form')?.remove();
  el.classList.add('has-form');
  const form = document.createElement('form');
  form.className = 'mcp-login-form';
  form.dataset.testid = 'mcp-client-form';
  form.innerHTML = `
    <p class="note">${escapeHtml(tr('Сервер входа {host} не умеет регистрировать приложения сам. Создайте приложение в его настройках и впишите client ID.', { host: host || '' }))}</p>
    <input class="input" name="id" autocomplete="off" spellcheck="false" placeholder="Client ID" aria-label="Client ID" required>
    <input class="input" name="secret" type="password" autocomplete="off" spellcheck="false" placeholder="${escapeHtml(tr('Client secret (если есть)'))}" aria-label="Client secret">
    <input class="input" name="port" inputmode="numeric" autocomplete="off" placeholder="${escapeHtml(tr('Порт возврата (если сервер просит точный адрес)'))}" aria-label="${escapeHtml(tr('Порт возврата (если сервер просит точный адрес)'))}">
    <button type="submit" class="ghost rt-run">${tr('Продолжить вход')}</button>`;
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const data = new FormData(form);
    const manual = { clientId: String(data.get('id') || '').trim() };
    if (data.get('secret')) manual.clientSecret = String(data.get('secret'));
    const port = String(data.get('port') || '').trim();
    if (port) manual.redirectPort = /^\d+$/.test(port) ? Number(port) : port;
    startMcpLogin(s, el, manual);
  });
  el.append(form);
}

// Переключатель «Команда / URL»: у HTTP-сервера нет команды и аргументов.
function syncMcpTransport() {
  const select = $('#mcp-transport');
  if (!select) return;
  const http = select.value === 'http';
  const commandField = $('#mcp-command-field');
  const argsField = $('#mcp-args-field');
  const urlField = $('#mcp-url-field');
  if (commandField) commandField.hidden = http;
  if (argsField) argsField.hidden = http;
  if (urlField) urlField.hidden = !http;
}

async function addMcp() {
  const name = $('#mcp-name').value.trim();
  const transport = $('#mcp-transport').value;
  const url = $('#mcp-url').value.trim();
  const command = $('#mcp-command').value.trim();
  if (transport === 'http' && !url) {
    toast(tr('Нужен URL MCP-сервера'));
    return;
  }
  if (transport !== 'http' && !command) {
    toast(tr('Нужна команда запуска'));
    return;
  }
  // Аргументы вводим строкой: кавычки сохраняем, чтобы пути с пробелами работали.
  const raw = $('#mcp-args').value.trim();
  const args = raw ? raw.match(/"[^"]*"|'[^']*'|\S+/g).map((a) => a.replace(/^["']|["']$/g, '')) : [];
  const autoApprove = $('#mcp-auto').checked;
  const body = transport === 'http'
    ? { name, url, command: '', args: [], autoApprove }
    : { name: name || command, command, args, autoApprove };
  try {
    await api('/api/mcp', { method: 'POST', body });
    $('#mcp-name').value = '';
    $('#mcp-command').value = '';
    $('#mcp-url').value = '';
    $('#mcp-args').value = '';
    $('#mcp-auto').checked = false;
    await refreshState();
    renderMcp();
    toast(tr('Сервер добавлен'));
  } catch (err) {
    toast(err.message);
  }
}

async function exportTeam() {
  try {
    const { team } = await api('/api/team/export');
    const blob = new Blob([JSON.stringify(team, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'deloist-team.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    $('#team-note').textContent = tr('Выгружено. Ботов: {bots}, навыков: {skills}, команд: {rooms}.', { bots: team.bots?.length || 0, skills: team.skills?.length || 0, rooms: team.rooms?.length || 0 });
  } catch (err) {
    toast(err.message);
  }
}

async function importTeam(file) {
  try {
    const text = await file.text();
    let team;
    try {
      team = JSON.parse(text);
    } catch {
      toast(tr('Это не файл команды Deloist'), 7000);
      return;
    }
    const size = (key) => (Array.isArray(team?.[key]) ? team[key].length : 0);
    const asked = tr('В файле: ботов {bots}, навыков {skills}, команд {rooms}, рутин {routines}.\n\nПерсоны и навыки из файла станут указаниями для твоих ботов, поэтому загружай только то, чему доверяешь. Боты будут спрашивать разрешение на опасные действия, рутины придут выключенными.\n\nЗагрузить?', {
      bots: size('bots'),
      skills: size('skills'),
      rooms: size('rooms'),
      routines: size('routines'),
    });
    if (!confirm(asked)) return;
    const { imported } = await api('/api/team/import', { method: 'POST', body: { team } });
    await refreshState();
    renderSkills();
    renderMcp();
    renderRoutines();
    $('#team-note').textContent = tr('Добавлено. Ботов: {bots}, навыков: {skills}, команд: {rooms}, рутин: {routines} (выключены).', { bots: imported.bots, skills: imported.skills, rooms: imported.rooms, routines: imported.routines });
    toast(tr('Команда загружена'));
  } catch (err) {
    toast(tr('Не смог загрузить: {error}', { error: trServer(err.message) }), 7000);
  }
}

// ---------------------------------------------------------------------------
// Модалки
// ---------------------------------------------------------------------------

function openModal(id) {
  const dlg = $(`#${id}`);
  if (id === 'modal-new') prepareNewBot();
  if (typeof dlg.showModal === 'function') dlg.showModal();
}

// Разделы окна настроек. Подписи и заголовок справа — одна и та же строка.
const SETTINGS_SECTIONS = [
  { id: 'main', label: tr('Основные') },
  { id: 'providers', label: tr('Модели и подключения') },
  { id: 'screen', label: tr('Боты на экране') },
  { id: 'voice', label: tr('Голос') },
  { id: 'agents', label: tr('Агенты и подключения') },
  { id: 'browser', label: tr('Браузер и безопасность') },
  { id: 'updates', label: tr('Обновления и о программе') },
];

// Прежние id разделов (ссылки data-open-settings, сохранённый выбор) ведут в новые.
const LEGACY_SETTINGS_SECTIONS = {
  model: 'providers',
  look: 'main',
  skills: 'agents',
  security: 'browser',
  night: 'browser',
  advanced: 'updates',
};

let settingsNav = null;
// Открытый подэкран: { id, parent } или null. Он занимает правую колонку вместо раздела.
let settingsSub = null;
const settingsLinks = {};
let telegramView = null;
let discordView = null;
let slackView = null;
let vaultView = null;
let cloudView = null;

function settingsSectionById(sectionId) {
  const id = LEGACY_SETTINGS_SECTIONS[sectionId] || sectionId;
  return SETTINGS_SECTIONS.find((section) => section.id === id) || SETTINGS_SECTIONS[0];
}

// Показываем ровно один раздел справа и пишем его название в шапку.
function showSettingsSection(sectionId) {
  const section = settingsSectionById(sectionId);
  settingsSub = null;
  $$('#form-settings [data-settings-section]').forEach((node) => {
    node.hidden = node.dataset.settingsSection !== section.id;
  });
  $$('#form-settings .settings-sub').forEach((node) => { node.hidden = true; });
  const back = $('#settings-back');
  if (back) back.hidden = true;
  const title = $('#settings-title');
  if (title) title.textContent = section.label;
  const body = $('#settings-body');
  if (body) body.scrollTop = 0;
  if (section.id === 'providers') {
    DeloistProviders.showList();
    DeloistProviders.load();
  }
}

// Подэкран: форма остаётся той же, меняется только то, что видно в правой колонке.
function openSettingsSub(subId) {
  const sub = $(`#form-settings .settings-sub[data-sub="${subId}"]`);
  if (!sub) return;
  settingsSub = { id: subId, parent: sub.dataset.parent };
  $$('#form-settings [data-settings-section]').forEach((node) => { node.hidden = true; });
  $$('#form-settings .settings-sub').forEach((node) => { node.hidden = node !== sub; });
  const back = $('#settings-back');
  if (back) back.hidden = false;
  const title = $('#settings-title');
  if (title) title.textContent = tr(sub.dataset.title || '');
  const body = $('#settings-body');
  if (body) body.scrollTop = 0;
  loadSubscreen(subId);
}

function closeSettingsSub() {
  if (!settingsSub) return;
  showSettingsSection(settingsSub.parent);
}

// Данные, которые раньше грузились при раскрытии группы, теперь грузим при показе экрана.
function loadSubscreen(subId) {
  if (subId === 'rules') loadRules();
  if (subId === 'skills') renderSkills();
  if (subId === 'scripts') renderScripts();
  if (subId === 'mcp') renderMcp();
  if (subId === 'dock') loadDockDisplays();
  if (subId === 'telegram') mountTelegramSettings();
  if (subId === 'discord') mountDiscordSettings();
  if (subId === 'slack') mountSlackSettings();
  if (subId === 'vault') mountVaultSettings();
  if (subId === 'devices') mountDevicesSettings();
  if (subId === 'cloud') mountCloudSettings();
}

// Экран Telegram монтируем один раз: при следующем открытии передаём свежий список ботов.
function mountTelegramSettings() {
  const host = $('#sub-telegram');
  if (!host || !window.DeloistUI?.mountTelegramSettings) return;
  const props = {
    api,
    bots: S.bots,
    onchanged: () => refreshState(),
  };
  if (telegramView) telegramView.update(props);
  else telegramView = window.DeloistUI.mountTelegramSettings(host, props);
}

// Экран Discord — по тому же правилу, что и Telegram.
function mountDiscordSettings() {
  const host = $('#sub-discord');
  if (!host || !window.DeloistUI?.mountDiscordSettings) return;
  const props = {
    api,
    bots: S.bots,
    onchanged: () => refreshState(),
  };
  if (discordView) discordView.update(props);
  else discordView = window.DeloistUI.mountDiscordSettings(host, props);
}

// Экран Slack — по тому же правилу: два токена приложения и привязка личных сообщений.
function mountSlackSettings() {
  const host = $('#sub-slack');
  if (!host || !window.DeloistUI?.mountSlackSettings) return;
  const props = {
    api,
    bots: S.bots,
    onchanged: () => refreshState(),
  };
  if (slackView) slackView.update(props);
  else slackView = window.DeloistUI.mountSlackSettings(host, props);
}

// Экран «Облако»: машина Instabox. Секреты остаются на ПК, окно получает только состояние.
function mountCloudSettings() {
  const host = $('#sub-cloud');
  if (!host || !window.DeloistUI?.mountCloudSettings) return;
  const props = { api };
  if (cloudView) cloudView.update(props);
  else cloudView = window.DeloistUI.mountCloudSettings(host, props);
}

// Экран «Менеджер паролей»: данные только из окна, пароли и токены живут в полях ввода.
function mountVaultSettings() {
  const host = $('#sub-vault');
  if (!host || !window.DeloistUI?.mountVaultSettings) return;
  const props = { api };
  if (vaultView) vaultView.update(props);
  else vaultView = window.DeloistUI.mountVaultSettings(host, props);
}

// Экран «Устройства»: допущенные телефоны, связь с сайтом и другие ПК аккаунта.
let devicesView = null;
function mountDevicesSettings() {
  const host = $('#sub-devices');
  if (!host || !window.DeloistUI?.mountDevicesSettings) return;
  if (!devicesView) devicesView = window.DeloistUI.mountDevicesSettings(host, { api });
}

window.DeloistSettings = { openSub: openSettingsSub, closeSub: closeSettingsSub };

// «Бесплатный сервер» Instabox: карточка в левой панели и кружок в ряду лимитов открывают экран «Облако».
function openCloudSettings() {
  openSettings('agents');
  openSettingsSub('cloud');
}

function mountInstabox() {
  const ui = window.DeloistUI;
  if (!ui?.mountInstaboxCard) return;
  const props = { api, onopen: openCloudSettings };
  ui.mountInstaboxCard($('#instabox-card'), props);
  ui.mountInstaboxRing($('#instabox-ring'), props);
  // Вернулись с экрана «Облако»: могли подключить или удалить машину.
  $('#modal-settings')?.addEventListener('close', () => ui.instaboxFeed(api).refresh(true));
}
mountInstabox();

// Строки-ссылки (Svelte) и «?» с подсказкой: места заданы в index.html атрибутами data-s-link и data-hint.
const SETTINGS_LINKS = {
  dock: { label: tr('Подробнее'), sub: tr('Положение, дисплей, край, автоскрытие, боты в доке') },
  skills: { label: tr('Навыки'), sub: tr('Правила и приёмы, которые бот получает в начале хода') },
  scripts: { label: tr('Мои скрипты'), sub: tr('Что бот научился делать в разы быстрее') },
  mcp: { label: tr('MCP-серверы'), sub: tr('Свои инструменты из внешних серверов') },
  images: { label: tr('Картинки'), sub: tr('Сервис, по которому бот рисует') },
  team: { label: tr('Команда'), sub: tr('Боты, навыки и рутины одним файлом') },
  telegram: { label: tr('Telegram'), sub: tr('Писать боту из своего Telegram') },
  discord: { label: tr('Discord'), sub: tr('Писать боту из личных сообщений Discord') },
  slack: { label: tr('Slack'), sub: tr('Писать боту из личных сообщений Slack') },
  cloud: { label: tr('Облако'), sub: tr('Боты в облаке, даже когда компьютер выключен') },
  vault: { label: tr('Менеджер паролей'), sub: tr('Bitwarden и 1Password: бот входит на сайты сам') },
  devices: { label: tr('Устройства'), sub: tr('Телефон, второй ПК и боты других ваших ПК') },
  rules: { label: tr('Правила разрешений'), sub: tr('Что бот делает без вопроса') },
  engines: { label: tr('Chrome, Obscura и Moli'), sub: tr('Чем отличаются и что показали замеры') },
  chrome: { label: tr('Мой основной Chrome'), sub: tr('Для опытных: боты работают в твоём браузере') },
};

function mountSettingsExtras() {
  if (!window.DeloistUI?.mountSettingsLink) return;
  $$('#form-settings [data-s-link]').forEach((host) => {
    const id = host.dataset.sLink;
    if (settingsLinks[id] || !SETTINGS_LINKS[id]) return;
    const info = SETTINGS_LINKS[id];
    settingsLinks[id] = window.DeloistUI.mountSettingsLink(host, {
      id,
      label: info.label,
      sublabel: info.sub,
      value: '',
      onclick: () => openSettingsSub(id),
    });
  });
  $$('#form-settings [data-hint]').forEach((host) => {
    if (host.dataset.mounted) return;
    host.dataset.mounted = '1';
    window.DeloistUI.mountSettingsHint(host, { text: tr(host.dataset.hint), label: tr('Подробнее') });
  });
}

// Текущие значения справа от строк-ссылок: «Навыки: 5 ›».
function updateSettingsLinks() {
  const set = (id, value) => settingsLinks[id]?.update({ value });
  set('skills', String(S.skills?.length ?? 0));
  set('mcp', String(S.mcp?.length ?? 0));
  set('scripts', S.scriptsList ? String(S.scriptsList.length) : '');
  const st = S.settings || {};
  set('images', st.imageBaseUrl ? st.imageModel || tr('настроены') : tr('не настроены'));
  set('rules', S.rulesCount === undefined ? '' : String(S.rulesCount));
}

// Боты со своей моделью: она молча перекрывает основную. Показываем, у кого, и даём вернуть на основную.
function botsWithOwnModel() {
  return (S.bots || []).filter((b) => b.driver || b.model);
}

function ownModelLabel(bot) {
  const names = { anthropic: 'Claude', gemini: 'Gemini', api: 'API', deloist: 'Deloist', cli: tr('программа'), demo: tr('демо') };
  const driver = bot.driver || '';
  const provider = providerDrivers.find((p) => p.value === driver);
  const kind = names[driver] || provider?.label || (driver.startsWith('acp:') ? driver.slice(4) : driver);
  return [kind, bot.model].filter(Boolean).join(' · ');
}

function ownModelsSentence(bots) {
  const list = bots.map((b) => `${b.name} (${ownModelLabel(b)})`).join(', ');
  return bots.length === 1
    ? tr('У {count} бота своя модель: {list}', { count: bots.length, list })
    : tr('У {count} ботов своя модель: {list}', { count: bots.length, list });
}

function refreshOwnModels() {
  const row = $('#own-models');
  if (!row) return;
  const bots = botsWithOwnModel();
  row.hidden = !bots.length;
  if (bots.length) $('#own-models-text').textContent = ownModelsSentence(bots);
}

async function moveBotsToMainModel() {
  const bots = botsWithOwnModel();
  if (!bots.length) return;
  if (!confirm(tr('Перевести на основную модель: {names}?', { names: bots.map((b) => b.name).join(', ') }))) return;
  try {
    for (const bot of bots) await api(`/api/bots/${bot.id}`, { method: 'PATCH', body: { driver: '', model: '' } });
    await refreshState();
    toast(tr('Боты теперь отвечают основной моделью'));
  } catch (err) {
    toast(err.message);
  }
  refreshOwnModels();
}

// После смены основной модели: тост с тем же предложением и кнопкой.
function offerOwnModels() {
  const bots = botsWithOwnModel();
  if (!bots.length) return false;
  const el = $('#toast');
  el.textContent = ownModelsSentence(bots);
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'ghost';
  button.textContent = tr('Перевести на основную');
  button.addEventListener('click', () => {
    el.hidden = true;
    moveBotsToMainModel();
  });
  el.append(' ', button);
  el.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => { el.hidden = true; }, 12000);
  return true;
}

// Что есть на этой системе и в этой сборке (ответ /api/state, поле platform). Чего нет, того не показываем:
// док и питомцы - окна Windows, горячие клавиши и снимок экрана - тоже.
function platformHas(feature) {
  const features = S.platform?.features;
  return !features || features[feature] !== false;
}

function applyPlatform() {
  const hide = (selector, off) => document.querySelectorAll(selector).forEach((el) => { el.hidden = off; });
  hide('#set-dock', !platformHas('dock'));
  // Подэкран дока открывается только ссылкой: снимать с него hidden здесь нельзя, он вылез бы под любым разделом.
  if (!platformHas('dock')) hide('#sub-dock', true);
  hide('#set-pets', !platformHas('pets'));
  hide('#set-hotkeys', !platformHas('hotkeys'));
}

function mountSettingsNav() {
  if (settingsNav) return;
  const host = $('#settings-nav');
  if (!host || !window.DeloistUI?.mountSettingsNav) return;
  const screenless = !platformHas('dock') && !platformHas('pets');
  settingsNav = window.DeloistUI.mountSettingsNav(host, {
    sections: SETTINGS_SECTIONS.filter((section) => !(screenless && section.id === 'screen')),
    onselect: showSettingsSection,
  });
}

// Бывшие аккордеоны (горячие клавиши, голос, мой Chrome) всегда раскрыты: сворачивать их больше нечем,
// а раскрытие запускает загрузку их данных. Возврат после закрытия окна оставляем,
// чтобы группы не считались открытыми в фоне.
function expandSettingsGroups() {
  $$('#form-settings details.group').forEach((group) => {
    if (!group.dataset.alwaysOpen) {
      group.dataset.alwaysOpen = 'true';
      group.addEventListener('toggle', () => {
        if (!group.open && $('#modal-settings').open) group.open = true;
      });
    }
    group.open = true;
  });
}

function openSettings(sectionId) {
  const requested = sectionId || window.DeloistUI?.settingsNav?.current?.() || 'main';
  const section = settingsSectionById(requested);
  window.DeloistUI?.settingsNav?.select?.(section.id);
  showSettingsSection(section.id);
  if ($('#modal-settings').open) return;
  mountSettingsExtras();
  fillSettings();
  markLook();
  markDock();
  loadDockDisplays();
  loadRules();
  loadScripts();
  expandSettingsGroups();
  applyPlatform();
  mountSettingsNav();
  openModal('modal-settings');
}
window.openProviderSettings = () => openSettings('providers');

// Сколько питомцев можно вывести разом, как MAX_PETS в src/pets.rs.
const MAX_PETS = 8;

async function setPet(id, on) {
  try {
    await api(`/api/bots/${id}`, { method: 'PATCH', body: { pet: on } });
    await refreshState();
  } catch (err) {
    toast(err.message);
  }
  markPets();
}

// Выбор питомцев в настройках и в профиле бота. Применяется сразу: окно появляется или исчезает на глазах.
function markPets() {
  const full = S.bots.filter((b) => b.pet).length >= MAX_PETS;
  const limit = tr('Не больше {count} питомцев разом', { count: MAX_PETS });
  const form = $('#form-settings');
  const mode = S.settings.petsMode || 'off';
  if (form) form.petsMode.value = mode;
  const box = $('#pet-picks');
  if (box) {
    $('#pet-picks-field').hidden = mode === 'off';
    box.innerHTML = S.bots
      .map((b) => {
        const blocked = full && !b.pet;
        return `<label class="skill-check"${blocked ? ` title="${escapeHtml(limit)}"` : ''}><input type="checkbox" value="${escapeHtml(b.id)}"${b.pet ? ' checked' : ''}${blocked ? ' disabled' : ''}><span>${escapeHtml(b.name)}${b.role ? ` · ${escapeHtml(b.role)}` : ''}</span></label>`;
      })
      .join('');
    $$('input', box).forEach((input) => {
      input.addEventListener('change', () => setPet(input.value, input.checked));
    });
  }
  // Переключатель «Питомец» во вкладке профиля: показывает, что на самом деле сохранилось.
  profileView?.update({ petOn: Boolean(botById(S.currentId)?.pet), petBlocked: full });
}

// Док ботов у края экрана. Применяется сразу, как питомцы: док меняется на глазах.
let dockDisplays = null;

async function loadDockDisplays() {
  try {
    const { displays } = await api('/api/dock/displays');
    dockDisplays = displays || [];
  } catch {
    dockDisplays = [];
  }
  markDock();
}

function dockDisplayOptions() {
  const chosen = S.settings.dockDisplay || '';
  const list = dockDisplays || [];
  const options = [`<option value="">${escapeHtml(tr('Вместе с окном Deloist'))}</option>`];
  list.forEach((display, index) => {
    const parts = [tr('Дисплей {n}', { n: index + 1 }), `${display.width}×${display.height}`];
    if (display.primary) parts.push(tr('основной'));
    options.push(`<option value="${escapeHtml(display.id)}">${escapeHtml(parts.join(' · '))}</option>`);
  });
  // Выбранный монитор сейчас не подключён: выбор не теряем, док тем временем стоит у окна.
  if (chosen && !list.some((display) => display.id === chosen)) {
    options.push(`<option value="${escapeHtml(chosen)}">${escapeHtml(tr('Дисплей не подключён'))}</option>`);
  }
  return options.join('');
}

function markDock() {
  const box = $('#set-dock');
  if (!box || !S.settings) return;
  const s = S.settings;
  box.classList.toggle('is-off', !s.dockEnabled);
  $('#dock-enabled').checked = Boolean(s.dockEnabled);
  $('#dock-layer').value = s.dockLayer || 'top';
  $('#dock-edge').value = s.dockEdge || 'right';
  $('#dock-autohide').checked = Boolean(s.dockAutohide);
  $('#dock-results').checked = s.dockResults !== false;
  $('#dock-activity').checked = s.dockActivity !== false;
  const display = $('#dock-display');
  display.innerHTML = dockDisplayOptions();
  display.value = s.dockDisplay || '';
  const list = $('#dock-bots');
  list.innerHTML = S.bots
    .map(
      (b) =>
        `<label class="prov-line"><div class="dock-bot"><div class="dock-bot-face" data-bot="${escapeHtml(b.id)}"></div><span class="dock-bot-name">${escapeHtml(b.name)}</span></div><span class="switch"><input type="checkbox" value="${escapeHtml(b.id)}"${b.dock !== false ? ' checked' : ''} /><span></span></span></label>`,
    )
    .join('');
  $$('.dock-bot-face', list).forEach((el) => {
    const bot = botById(el.dataset.bot);
    if (bot) paintAvatar(el, bot, `dock:${bot.id}`, { still: true });
  });
  $$('input', list).forEach((input) => {
    input.addEventListener('change', () => setDockBot(input.value, input.checked));
  });
}

async function setDockBot(id, on) {
  try {
    await api(`/api/bots/${id}`, { method: 'PATCH', body: { dock: on } });
    await refreshState();
  } catch (err) {
    toast(err.message);
  }
  markDock();
}

// Лёгкий браузер Obscura: ставится только по кнопке, пока качается — опрашиваем состояние.
let obscuraState = null;
let obscuraTimer = 0;

function renderObscura() {
  const st = obscuraState;
  const line = $('#obscura-line');
  const button = $('#obscura-action');
  const hint = $('#browser-engine-hint');
  if (!st || !line) return;
  const busy = st.state === 'downloading' || st.state === 'installing';
  if (st.state === 'downloading') line.textContent = tr('скачивается: {n}%', { n: st.percent });
  else if (st.state === 'installing') line.textContent = tr('распаковывается…');
  else if (st.state === 'error') line.textContent = st.error;
  else if (st.installed) line.textContent = tr('версия {v} установлена', { v: st.version });
  else line.textContent = tr('не установлен, архив {mb} МБ', { mb: st.downloadMb });
  button.disabled = busy;
  button.textContent = st.installed && !busy ? tr('Удалить') : st.state === 'error' ? tr('Повторить') : tr('Скачать');
  hint.hidden = !($('#form-settings').browserEngine.value === 'obscura' && !st.installed);
  clearTimeout(obscuraTimer);
  if (busy) obscuraTimer = setTimeout(loadObscura, 700);
}

async function loadObscura() {
  try {
    obscuraState = await api('/api/browser/obscura');
    renderObscura();
  } catch (err) {
    toast(err.message);
  }
}

// Лёгкий браузер Moli: так же по кнопке. В облаке он вшит в образ: кнопка скрыта, а пустая настройка значит Moli.
let moliState = null;
let moliTimer = 0;

function renderMoli() {
  const st = moliState;
  const line = $('#moli-line');
  const button = $('#moli-action');
  const hint = $('#browser-engine-hint-moli');
  if (!st || !line) return;
  const busy = st.state === 'downloading' || st.state === 'installing';
  if (st.state === 'downloading') line.textContent = tr('скачивается: {n}%', { n: st.percent });
  else if (st.state === 'installing') line.textContent = tr('распаковывается…');
  else if (st.state === 'error') line.textContent = st.error;
  else if (st.baked) line.textContent = tr('встроен в программу, версия {v}', { v: st.version });
  else if (st.installed) line.textContent = tr('версия {v} установлена', { v: st.version });
  else if (!st.supported) line.textContent = tr('для этой системы сборки нет');
  else line.textContent = tr('не установлен, архив {mb} МБ', { mb: st.downloadMb });
  button.hidden = st.baked || st.cloud;
  button.disabled = busy || !st.supported;
  button.textContent = st.installed && !busy ? tr('Удалить') : st.state === 'error' ? tr('Повторить') : tr('Скачать');
  hint.hidden = !($('#form-settings').browserEngine.value === 'moli' && !st.installed);
  clearTimeout(moliTimer);
  if (busy) moliTimer = setTimeout(loadMoli, 700);
}

async function loadMoli() {
  try {
    moliState = await api('/api/browser/moli');
    renderMoli();
  } catch (err) {
    toast(err.message);
  }
}

async function moliAction() {
  try {
    if (moliState && moliState.installed) {
      const res = await api('/api/browser/moli', { method: 'DELETE' });
      moliState = res.status;
      S.settings = res.settings;
      $('#form-settings').browserEngine.value = 'chrome';
      toast(tr('Moli удалён, бот снова работает в Chrome'));
    } else {
      moliState = await api('/api/browser/moli/install', { method: 'POST' });
    }
    renderMoli();
  } catch (err) {
    toast(err.message);
  }
}

async function obscuraAction() {
  try {
    if (obscuraState && obscuraState.installed) {
      const res = await api('/api/browser/obscura', { method: 'DELETE' });
      obscuraState = res.status;
      S.settings = res.settings;
      $('#form-settings').browserEngine.value = 'chrome';
      toast(tr('Obscura удалён, бот снова работает в Chrome'));
    } else {
      obscuraState = await api('/api/browser/obscura/install', { method: 'POST' });
    }
    renderObscura();
  } catch (err) {
    toast(err.message);
  }
}

let knowMeHandle = null;
let whatsNewHandle = null;

function openKnowMe() {
  if (!window.DeloistUI?.mountKnowMe) return;
  if (knowMeHandle) {
    try { knowMeHandle.destroy(); } catch {}
    knowMeHandle = null;
  }
  const host = document.createElement('div');
  document.body.append(host);
  knowMeHandle = window.DeloistUI.mountKnowMe(host, {
    onimported: () => {
      refreshState();
      toast(tr('Боты из Hermes перенесены'));
    },
    onclose: () => {
      try { knowMeHandle?.destroy(); } catch {}
      knowMeHandle = null;
      host.remove();
    },
  });
}

// Окно «Что нового». Полный журнал берётся с сервера по ссылке «Все изменения».
function openWhatsNew(data) {
  if (!window.DeloistUI?.mountWhatsNew) return;
  if (whatsNewHandle) {
    try { whatsNewHandle.destroy(); } catch {}
    whatsNewHandle = null;
  }
  const host = document.createElement('div');
  document.body.append(host);
  whatsNewHandle = window.DeloistUI.mountWhatsNew(host, {
    current: data?.current || '',
    from: data?.from || '',
    sections: data?.sections || [],
    startAll: Boolean(data?.all),
    loadAll: async () => (await api('/api/whats-new?all=1')).sections || [],
    onclose: () => {
      try { whatsNewHandle?.destroy(); } catch {}
      whatsNewHandle = null;
      host.remove();
    },
  });
}

function fillSettings() {
  const f = $('#form-settings');
  renderSkills();
  loadSelfDev();
  renderMcp();
  syncMcpTransport();
  // Список агентов приходит с задержкой: пока его нет, выбранный драйвер и модель агента всё равно должны быть в полях,
  // иначе «Сохранить» в первые секунды отправит пустой драйвер и сбросит основную модель.
  ensureOption(f.driver, S.settings.driver);
  f.driver.value = S.settings.driver || 'demo';
  if ((S.settings.driver || '').startsWith('acp:')) {
    f.acpModel.innerHTML = providerModelOptions(S.settings.driver, S.settings.model || '');
    f.acpModel.value = S.settings.model || '';
  }
  f.skillLearning.value = S.settings.skillLearning || 'auto';
  fillProviderDrivers(f);
  f.model.value = S.settings.model || '';
  f.planModel.value = S.settings.model === 'gpt-6.1-terra' ? 'gpt-6.1-terra' : 'deepseek-flash';
  f.baseUrl.value = S.settings.baseUrl || '';
  f.apiKey.value = '';
  f.apiKey.placeholder = S.settings.hasKey ? tr('ключ сохранён') : 'sk-…';
  f.anthropicApiKey.value = '';
  f.anthropicApiKey.placeholder = S.settings.hasAnthropicKey ? tr('ключ сохранён') : 'sk-ant-…';
  f.geminiApiKey.value = '';
  f.geminiApiKey.placeholder = S.settings.hasGeminiKey ? tr('ключ сохранён') : 'AIza…';
  f.apiKeyEnv.value = S.settings.apiKeyEnv || '';
  f.cliCommand.value = S.settings.cliCommand || '';
  f.acpCommand.value = S.settings.acpCommand || '';
  f.fallbackDriver.value = S.settings.fallbackDriver || '';
  f.fallbackModel.value = S.settings.fallbackModel || '';
  f.fallbackBaseUrl.value = S.settings.fallbackBaseUrl || '';
  f.fallbackApiKey.value = '';
  f.fallbackApiKey.placeholder = S.settings.hasFallbackKey ? tr('ключ сохранён') : 'sk-…';
  f.fallbackCliCommand.value = S.settings.fallbackCliCommand || '';
  f.temperature.value = S.settings.temperature ?? 0.7;
  f.historyLimit.value = S.settings.historyLimit ?? 30;
  f.vision.checked = Boolean(S.settings.vision);
  f.browserPath.value = S.settings.browserPath || '';
  f.browserEngine.value = ['obscura', 'moli'].includes(S.settings.browserEngine) ? S.settings.browserEngine : 'chrome';
  loadObscura();
  loadMoli().then(() => {
    // В облаке пустая настройка значит Moli: так и показываем, пока человек не выбрал другое.
    if (moliState && moliState.cloud && !S.settings.browserEngine) {
      f.browserEngine.value = 'moli';
      renderMoli();
    }
  });
  f.imageBaseUrl.value = S.settings.imageBaseUrl || '';
  f.imageModel.value = S.settings.imageModel || '';
  f.imageDaily.value = S.settings.imageDaily ?? 50;
  f.imageApiKey.value = '';
  f.imageApiKey.placeholder = S.settings.hasImageKey ? tr('ключ сохранён') : tr('пусто — основной ключ, если адрес тот же');
  f.dailyTokenLimit.value = S.settings.dailyTokenLimit ?? 0;
  f.autostart.checked = Boolean(S.settings.autostart);
  f.keepAwake.checked = Boolean(S.settings.keepAwake);
  f.checkUpdates.checked = S.settings.checkUpdates !== false;
  f.autoUpdate.checked = S.settings.autoUpdate !== false;
  markPets();
  $('#settings-path').textContent = tr('Данные: {path}', { path: S.dataDir });
  const current = { driver: S.settings.driver || 'demo', model: S.settings.model || '' };
  if (settingsPicker) {
    settingsPicker.picker.update({ value: current, settings: S.settings });
    settingsPicker.reload();
  } else {
    settingsPicker = mountModelPicker($('#set-model'), { value: current, allowDefault: false, onselect: applySettingsModel });
  }
  syncSettingsFields();
  updateSettingsLinks();
  refreshOwnModels();
}

// Общий выбор модели (Svelte, ui/src/components/ModelPicker.svelte): строка «Модель» и окно со всеми
// моделями подключённых провайдеров. Выбор отдаёт пару { driver, model} — те же два поля, что раньше
// вводили руками, так что сохранённые боты и настройки читаются без изменений.
const modelPickers = new Set();

function pruneModelPickers() {
  for (const entry of modelPickers) {
    if (entry.host.isConnected) continue;
    entry.picker.destroy();
    modelPickers.delete(entry);
  }
}

// Модель в окне «Новый бот»: карточки на виду, остальные модели — под «Ещё».
function mountNewBotModel(host, { value, onselect }) {
  pruneModelPickers();
  const picker = DeloistUI.mountNewBotModel(host, {
    ...(DeloistProviders.snapshot() || { providers: [], plan: null }),
    settings: S.settings,
    value,
    onselect: (next) => {
      picker.update({ value: next });
      onselect?.(next);
    },
    onconnect: () => DeloistProviders.open(),
  });
  const entry = {
    host,
    picker,
    async reload() {
      const data = await DeloistProviders.choices();
      if (modelPickers.has(entry) && host.isConnected) picker.update({ providers: data.providers, plan: data.plan, settings: S.settings });
    },
  };
  modelPickers.add(entry);
  if (DeloistProviders.started()) entry.reload().catch(() => {});
  return entry;
}

// Внешность в окне «Новый бот»: тот же выбор шара, что в ящике бота (mountLookPicker), один клик.
// Первым выбран шар, которого ещё нет у других ботов: так бот сразу отличается от соседей.
let newLook = { owned: [], pick: () => ({}) };

function clearNewLook() {
  for (const avatar of newLook.owned) avatar.destroy();
  newLook = { owned: [], pick: () => ({}) };
}

function freeSkin() {
  const count = window.CrewAvatar?.ORBS.length || 12;
  const used = new Set(S.bots.map((bot) => bot.skin));
  const free = [...Array(count).keys()].filter((i) => !used.has(i));
  const from = free.length ? free : [...Array(count).keys()];
  return from[Math.floor(Math.random() * from.length)];
}

function prepareNewBot() {
  clearNewLook();
  const owned = [];
  const picker = mountLookPicker($('#nb-look'), { name: '', color: BOT_COLORS[0], skin: freeSkin() }, { prefix: 'nb', owned, size: 96 });
  newLook = { owned, pick: () => ({ skin: picker.skin }) };
  fillNewBotTeams();
}

// «Команда» в окне нового бота: есть команды — можно сразу положить бота в одну из них.
// Открыли из команды — она и выбрана.
function fillNewBotTeams() {
  const field = $('#nb-team-field');
  const select = $('#nb-team');
  if (!field || !select) return;
  const rooms = S.rooms.filter((r) => !r.archived);
  field.hidden = !rooms.length;
  select.replaceChildren(
    new Option(tr('Без команды'), ''),
    ...rooms.map((r) => new Option(r.name, r.id)),
  );
  const here = currentRoom();
  select.value = here && !here.archived ? here.id : '';
}

// Поля внешности для POST /api/bots: те же, что ящик бота сохраняет в профиле.
function newBotLookFields() {
  const { skin } = newLook.pick();
  return skin === undefined ? {} : { skin };
}

function mountModelPicker(host, { value, allowDefault = true, hideLabel = false, onselect }) {
  pruneModelPickers();
  const picker = DeloistUI.mountModelPicker(host, {
    ...(DeloistProviders.snapshot() || { providers: [], plan: null }),
    settings: S.settings,
    value,
    allowDefault,
    hideLabel,
    onselect: (next) => {
      picker.update({ value: next });
      onselect?.(next);
    },
    onconnect: () => DeloistProviders.open(),
  });
  const entry = {
    host,
    picker,
    async reload() {
      const data = await DeloistProviders.choices();
      if (modelPickers.has(entry) && host.isConnected) picker.update({ providers: data.providers, plan: data.plan, settings: S.settings });
    },
  };
  modelPickers.add(entry);
  // Провайдеры подключаются в конце boot(): раньше этого выбор модели загрузит reloadModelPickers.
  if (DeloistProviders.started()) entry.reload().catch(() => {});
  return entry;
}

// Провайдер подключили, включили или скрыли модель: открытые выборы подхватывают новый список.
function reloadModelPickers() {
  pruneModelPickers();
  for (const entry of modelPickers) entry.reload().catch(() => {});
}

// Скачанные агенты («Подключить свою ИИ») в выборе «Откуда брать ответы» и их модели.
let providerDrivers = [];

let newBotPicker = null;
let settingsPicker = null;

function ensureOption(select, value) {
  if (value && ![...select.options].some((o) => o.value === value)) select.append(new Option(value, value));
}

// Выбор в общих настройках: пишем в те же поля формы, на которые смотрит остальной код настроек.
function applySettingsModel(next) {
  const f = $('#form-settings');
  ensureOption(f.driver, next.driver);
  f.driver.value = next.driver;
  // Обработчик смены драйвера сбрасывает модель агента, поэтому модель выставляем после него.
  f.driver.dispatchEvent(new Event('change'));
  f.model.value = next.model;
  if (next.driver === 'deloist' && next.model) {
    ensureOption(f.planModel, next.model);
    f.planModel.value = next.model;
  } else if (next.driver.startsWith('acp:')) {
    ensureOption(f.acpModel, next.model);
    f.acpModel.value = next.model;
  }
}

async function providersChanged(change) {
  await refreshState();
  if (change?.defaultChanged && $('#modal-settings').open) {
    const value = { driver: S.settings.driver || 'demo', model: S.settings.model || '' };
    applySettingsModel(value);
    settingsPicker?.picker.update({ value, settings: S.settings });
  }
  reloadModelPickers();
}

function providerModelOptions(driver, current) {
  const provider = providerDrivers.find((p) => p.value === driver);
  const models = provider?.models || [];
  const options = [`<option value="">${tr('как решит агент')}</option>`]
    .concat(models.map((m) => `<option value="${escapeHtml(m.id)}">${escapeHtml(m.name)}</option>`));
  if (current && !models.some((m) => m.id === current)) options.push(`<option value="${escapeHtml(current)}">${escapeHtml(current)}</option>`);
  return options.join('');
}

function addProviderOptions(select, current) {
  select.querySelectorAll('option[data-provider]').forEach((o) => o.remove());
  for (const p of providerDrivers) {
    const option = document.createElement('option');
    option.value = p.value;
    option.textContent = p.label;
    option.dataset.provider = '1';
    select.append(option);
  }
  // Основной агент мог быть выключен или удалён: всё равно показываем, что выбрано.
  if (current && current.startsWith('acp:') && !providerDrivers.some((p) => p.value === current)) {
    const option = document.createElement('option');
    option.value = current;
    option.textContent = current.slice(4);
    option.dataset.provider = '1';
    select.append(option);
  }
  select.value = current;
}

async function fillProviderDrivers(f) {
  providerDrivers = await DeloistProviders.drivers();
  const current = S.settings.driver || 'demo';
  addProviderOptions(f.driver, current);
  f.acpModel.innerHTML = providerModelOptions(current, S.settings.model || '');
  f.acpModel.value = current.startsWith('acp:') ? (S.settings.model || '') : '';
  syncSettingsFields();
}

// Запасная модель показывает только поля выбранного способа; значения остальных не теряются.
// Ключи и адрес основной модели живут в окне «Подключить свою ИИ», а здесь скрытые поля пустые и ничего не стирают.
function syncSettingsFields() {
  const f = $('#form-settings');
  $$('[data-fb]', f).forEach((el) => { el.hidden = !el.dataset.fb.split(' ').includes(f.fallbackDriver.value); });
}

const DAY_NAMES = [tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')];

const plural = (n, one, few, many) => {
  if (window.LANG === 'en') return tr(n === 1 ? one : many);
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
};

const STATUS_LABELS = {
  ok: tr('готово'),
  quiet: tr('тихо: сообщать нечего'),
  needs: tr('ждёт решения'),
  failed: tr('ошибка'),
  stopped: tr('остановлено'),
  limit: tr('упёрлось в предел токенов'),
  skipped: tr('пропущено: компьютер спал'),
  error: tr('не запустилась'),
  running: tr('идёт'),
  interrupted: tr('прервалась: программа закрылась'),
  gave_up: tr('не выполнилась за сутки'),
};
// Чип прошлого запуска с тревожным значком: рутина не дошла до «готово».
const BAD_STATUSES = ['failed', 'error', 'limit', 'needs', 'interrupted', 'gave_up'];
const statusClass = (status) => (STATUS_LABELS[status] ? `st-${status}` : '');

const isTime = (v) => typeof v === 'string' && /^\d{2}:\d{2}$/.test(v);

function scheduleText(r) {
  if (r.hook) return tr('по внешнему вызову');
  const parts = [isTime(r.at) ? tr('в {time}', { time: r.at }) : tr('каждые {minutes} мин', { minutes: Number(r.everyMinutes) || 60 })];
  if (!isTime(r.at) && isTime(r.windowFrom) && isTime(r.windowTo)) parts.push(`${r.windowFrom}–${r.windowTo}`);
  const days = [...new Set((r.days || []).map(Number).filter((d) => d >= 1 && d <= 7))].sort((a, b) => a - b);
  const key = days.join('');
  if (key === '12345') parts.push(tr('по будням'));
  else if (key === '67') parts.push(tr('по выходным'));
  else if (days.length && days.length < 7) parts.push(days.map((d) => DAY_NAMES[d - 1]).join(' '));
  return parts.join(', ');
}

const sameDay = (a, b) => a.toDateString() === b.toDateString();

/** Адрес вебхука: порт тот же, что у страницы. */
function hookUrl(r) {
  const port = location.port || '80';
  return `${location.protocol}//127.0.0.1:${port}/hooks/${r.id}/${r.hookToken || ''}`;
}

function nextText(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return '';
  const d = new Date(n);
  if (sameDay(d, new Date())) return timeOf(n);
  return `${d.toLocaleDateString(window.LOCALE, { weekday: 'short' })} ${timeOf(n)}`;
}

function whenText(ts) {
  const n = Number(ts);
  if (!Number.isFinite(n) || n <= 0) return '';
  const d = new Date(n);
  if (sameDay(d, new Date())) return timeOf(n);
  return `${d.toLocaleDateString(window.LOCALE, { day: 'numeric', month: 'short' })} ${timeOf(n)}`;
}

async function runRoutine(id) {
  try {
    await api(`/api/routines/${encodeURIComponent(id)}/run`, { method: 'POST' });
    toast(tr('Запустил, смотри в чате бота'));
  } catch (err) {
    toast(err.message);
  }
}

const routinesOpen = () => $('#modal-routines').open;

// Напоминания живут на своей вкладке: во вкладке «Рутины» их нет (у них нет бота и задания).
const isReminder = (r) => r.kind === 'reminder';
const ordinaryRoutines = () => S.routines.filter((r) => !isReminder(r));

// Название-суть и чипы рутины (бот, расписание, следующий и прошлый запуск): один вид и в списке рутин, и карточкой в чате.
function routineParts(r) {
  const bot = botById(r.botId);
  const next = r.hook ? tr('по вебхуку') : r.enabled ? nextText(r.nextRun) : '';
  const last = STATUS_LABELS[r.lastStatus];
  const icon = (name) => ic(name);
  const gist = String(r.prompt || '').replace(/\s+/g, ' ').trim().slice(0, 140);
  const nextTitle = r.hook ? tr('Адрес: {url}', { url: hookUrl(r) }) : tr('Следующий запуск: {time}', { time: next });
  const retry = Number(r.retryAttempt) > 0 && r.enabled && !r.hook
    ? tr('Повтор {n} в {time}', { n: Number(r.retryAttempt) + 1, time: nextText(r.nextRun) })
    : '';
  const lastTitle = r.lastRun
    ? `${tr('Последний запуск {time}', { time: whenText(r.lastRun) })}${last ? `: ${last}` : ''}${retry ? `. ${retry}` : ''}`
    : '';
  const lastIcon = BAD_STATUSES.includes(r.lastStatus) ? 'alert-circle' : r.lastStatus === 'running' ? 'clock' : 'check-long';
  const chips = [
    `<span class="rt-chip rt-bot" title="${tr('Бот')}">${icon('user')}${escapeHtml(bot?.name || tr('бот удалён'))}</span>`,
    `<span class="rt-chip" title="${tr('Когда')}">${icon('repeat')}${escapeHtml(scheduleText(r))}</span>`,
    next || r.hook ? `<span class="rt-chip" title="${escapeHtml(nextTitle)}">${icon(r.hook ? 'link' : 'clock')}${r.hook ? tr('вебхук') : escapeHtml(next)}</span>` : '',
    r.lastRun ? `<span class="rt-chip rt-last ${statusClass(r.lastStatus)}" title="${escapeHtml(lastTitle)}">${icon(lastIcon)}${escapeHtml(whenText(r.lastRun))}</span>` : '',
  ].join('');
  return { gist, chips };
}

// Строка рутины: одна и та же в окне «Рутины» и во вкладке панели бота.
function routineRow(r, rerender) {
  const bot = botById(r.botId);
  const el = document.createElement('div');
  el.className = `routine${r.enabled ? '' : ' off'}`;
  const icon = (name) => ic(name);
  const { gist, chips } = routineParts(r);
  el.innerHTML = `
    <div class="rt-text">
      <div class="rt-title" title="${escapeHtml(r.title)}">${escapeHtml(r.title)}</div>
      ${gist ? `<div class="rt-gist" title="${escapeHtml(gist)}">${escapeHtml(gist)}</div>` : ''}
    </div>
    <div class="rt-actions">
      <button type="button" class="icon-btn rt-quiet${r.quiet ? ' on' : ''}" aria-pressed="${r.quiet}" title="${r.quiet ? tr('только важное') : tr('писать всё')}: ${tr('Писать в чат, только если есть что сказать')}" aria-label="${tr('Писать в чат, только если есть что сказать')}">${icon(r.quiet ? 'bell' : 'bell-off')}</button>
      <button type="button" class="icon-btn rt-run" title="${tr('Запустить')}" aria-label="${tr('Запустить')}">${icon('play')}</button>
      <label class="switch"><input type="checkbox" ${r.enabled ? 'checked' : ''}><span></span></label>
      <button class="icon-btn" title="${tr('Удалить')}" aria-label="${tr('Удалить')}">${ICONS.close}</button>
    </div>
    <div class="rt-chips">${chips}</div>`;
  $('.rt-run', el).addEventListener('click', () => runRoutine(r.id));
  $('.rt-quiet', el).addEventListener('click', async () => {
    try {
      const { routine } = await api(`/api/routines/${r.id}`, { method: 'PATCH', body: { quiet: !r.quiet } });
      r.quiet = routine.quiet;
      rerender();
    } catch (err) {
      toast(err.message);
    }
  });
  $('.switch input', el).addEventListener('change', async (e) => {
    try {
      await api(`/api/routines/${r.id}`, { method: 'PATCH', body: { enabled: e.target.checked } });
      r.enabled = e.target.checked;
      rerender();
    } catch (err) {
      toast(err.message);
      e.target.checked = r.enabled;
    }
  });
  $('.rt-actions button:last-of-type', el).addEventListener('click', async () => {
    try {
      await api(`/api/routines/${r.id}`, { method: 'DELETE' });
      S.routines = S.routines.filter((x) => x.id !== r.id);
      rerender();
    } catch (err) {
      toast(err.message);
    }
  });
  return el;
}

function renderRoutines() {
  const list = $('#routines-list');
  list.innerHTML = '';
  if (!ordinaryRoutines().length) {
    list.innerHTML = `<p class="note">${tr('Пока ни одной. Добавь первую, и бот начнёт работать сам.')}</p>`;
  }
  // Рутины поменялись здесь — вкладка панели бота показывает те же.
  const both = () => {
    renderRoutines();
    if (drawerShown() && S.tab === 'routines') renderDrawer();
  };
  for (const r of ordinaryRoutines()) list.append(routineRow(r, both));
  syncRoutineTabs();
}

// ---------------------------------------------------------------------------
// Журнал ночной смены
// ---------------------------------------------------------------------------

const numberRu = (n) => Number(n || 0).toLocaleString(window.LOCALE);

function renderJournal() {
  const { entries, usage, limit } = S.journal;
  const usageEl = $('#journal-usage');
  if (usage) {
    const spent = `${usage.estimated ? tr('примерно ') : ''}${numberRu(usage.tokens)}`;
    const cache = Number(usage.cacheRate) > 0 ? ` · ${tr('из кэша {n}%', { n: numberRu(usage.cacheRate) })}` : '';
    const total = Number(limit) > 0
      ? tr('Сегодня потрачено {spent} токенов из {limit}', { spent, limit: numberRu(limit) })
      : tr('Сегодня потрачено {spent} токенов', { spent });
    usageEl.textContent = `${total}${cache}`;
  } else {
    usageEl.textContent = '';
  }

  const list = $('#journal-list');
  list.innerHTML = '';
  if (!entries.length) {
    list.innerHTML = `<p class="note">${tr('Пока тихо. Здесь появится всё, что боты сделали без тебя.')}</p>`;
    return;
  }

  // Тихие проверки не карточки, а одна строка: иначе они вытеснят важное.
  const quiet = entries.filter((e) => e.status === 'quiet');
  const important = entries.filter((e) => e.status !== 'quiet');
  if (quiet.length) {
    const box = document.createElement('div');
    box.className = 'jr-quiet';
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'jr-quiet-summary';
    btn.setAttribute('aria-expanded', String(S.journalQuietOpen));
    const count = `${quiet.length} ${plural(quiet.length, 'тихая проверка', 'тихие проверки', 'тихих проверок')}`;
    btn.textContent = important.length ? tr('Ещё {count}, сообщать нечего', { count }) : tr('{count}, сообщать нечего', { count });
    btn.addEventListener('click', () => {
      S.journalQuietOpen = !S.journalQuietOpen;
      renderJournal();
    });
    box.append(btn);
    if (S.journalQuietOpen) {
      const rows = document.createElement('div');
      rows.className = 'jr-quiet-list';
      for (const e of quiet) {
        const row = document.createElement('div');
        row.className = 'jr-quiet-row';
        const when = document.createElement('span');
        when.className = 'jr-when';
        when.textContent = whenText(e.at);
        const bot = document.createElement('span');
        bot.className = 'jr-bot';
        bot.textContent = e.botName || '';
        const title = document.createElement('span');
        title.className = 'jr-title';
        title.textContent = trServer(e.title || '');
        row.append(when, bot, title);
        rows.append(row);
      }
      box.append(rows);
    }
    list.append(box);
  }
  if (!important.length) return;

  for (const e of important.slice(0, 30)) {
    const el = document.createElement('div');
    el.className = 'jr';
    const missed = Array.isArray(e.missed) ? e.missed : [];
    const canRerun = missed.length && S.routines.some((r) => r.id === e.routineId);
    el.innerHTML = `
      <div class="jr-head">
        <span class="jr-when">${escapeHtml(whenText(e.at))}</span>
        <span class="jr-bot">${escapeHtml(e.botName)}</span>
        <span class="jr-title">${escapeHtml(trServer(e.title))}</span>
        <span class="jr-status ${statusClass(e.status)}">${escapeHtml(STATUS_LABELS[e.status] || '')}</span>
      </div>
      ${e.text ? `<div class="jr-text">${escapeHtml(trServer(e.text))}</div>` : ''}
      ${e.fallback ? `<div class="jr-fallback">${tr('на запасной: {name}', { name: escapeHtml(e.fallback) })}</div>` : ''}
      ${Number(e.tokens) > 0 ? `<div class="jr-tokens">${numberRu(e.tokens)} ${plural(Number(e.tokens), 'токен', 'токена', 'токенов')}</div>` : ''}
      ${
        missed.length
          ? `<div class="jr-missed"><b>${tr('Ждёт твоего решения:')}</b><ul>${missed.map((m) => `<li>${escapeHtml(m)}</li>`).join('')}</ul>${
              canRerun ? `<button type="button" class="ghost">${tr('Запустить снова')}</button>` : ''
            }</div>`
          : ''
      }`;
    $('.jr-missed .ghost', el)?.addEventListener('click', () => runRoutine(e.routineId));
    list.append(el);
  }
}

const newJournalEntries = () => S.journal.entries.filter((e) => e.status !== 'quiet' && Number(e.at) > Number(S.journal.seenAt || 0));

function updateRoutinesDot() {
  $('#routines-dot').hidden = !newJournalEntries().length;
}

async function loadJournal() {
  try {
    const data = await api('/api/journal?limit=200');
    S.journal = {
      entries: data.entries || [],
      seenAt: Number(data.seenAt) || 0,
      usage: data.usage || null,
      limit: Number(data.limit) || 0,
    };
    updateRoutinesDot();
    return true;
  } catch {
    return false;
  }
}

async function markJournalSeen() {
  const now = Date.now();
  S.journal.seenAt = now;
  updateRoutinesDot();
  try {
    const { settings } = await api('/api/settings', { method: 'PUT', body: { journalSeenAt: now } });
    if (settings) S.settings = settings;
  } catch {
    // отметка не сохранилась: в следующий раз точка просто покажется снова
  }
}

// Правая панель «Задачи и автоматизация»: «Рутины» и «Журнал» рисует этот файл, «Задачи», «Напоминания» и «Привычки» — компоненты Svelte (ui/src/components/routines).
let routineTab = 'routines';
let routineTabsView = null;
let remindersView = null;
let habitsView = null;
let todosView = null;
let routinesCloseBound = false;

function openTelegramFromRoutines() {
  $('#modal-routines').close();
  openSettings();
  openSettingsSub('telegram');
}

function reminderProps() {
  return {
    api,
    routines: S.routines.filter(isReminder),
    onchanged: async () => {
      await refreshState();
      syncRoutineTabs();
    },
    onopentelegram: openTelegramFromRoutines,
  };
}

// Список напоминаний приходит из общего состояния: после его обновления передаём свежий.
function syncRoutineTabs() {
  if (routineTab === 'reminders' && remindersView) remindersView.update({ routines: S.routines.filter(isReminder) });
}

function dropRoutineTabViews() {
  remindersView?.destroy();
  remindersView = null;
  habitsView?.destroy();
  habitsView = null;
  todosView?.destroy();
  todosView = null;
}

function setRoutineTab(id) {
  const ui = window.DeloistUI;
  routineTab = id;
  $('#rt-pane-routines').hidden = id !== 'routines';
  $('#rt-pane-journal').hidden = id !== 'journal';
  $('#rt-pane-reminders').hidden = id !== 'reminders';
  $('#rt-pane-todos').hidden = id !== 'todos';
  $('#rt-pane-habits').hidden = id !== 'habits';
  $('#rt-ics').closest('.sheet-foot').hidden = id !== 'routines';
  dropRoutineTabViews();
  if (id === 'reminders' && ui?.mountRemindersTab) remindersView = ui.mountRemindersTab($('#rt-pane-reminders'), reminderProps());
  if (id === 'habits' && ui?.mountHabitsTab) habitsView = ui.mountHabitsTab($('#rt-pane-habits'), { api, onopentelegram: openTelegramFromRoutines });
  if (id === 'todos' && ui?.mountTodosTab) todosView = ui.mountTodosTab($('#rt-pane-todos'), { api });
  routineTabsView?.update({ value: id });
}

function mountRoutineTabs() {
  const ui = window.DeloistUI;
  const host = $('#rt-tabs');
  if (routineTabsView || !ui?.mountComponent || !host) return;
  routineTabsView = ui.mountComponent(ui.Tabs, host, {
    label: tr('Разделы'),
    value: 'routines',
    items: [
      { id: 'routines', label: tr('Рутины'), panelId: 'rt-pane-routines' },
      { id: 'journal', label: tr('Журнал'), panelId: 'rt-pane-journal' },
      { id: 'todos', label: tr('Задачи'), panelId: 'rt-pane-todos' },
      { id: 'reminders', label: tr('Напоминания'), panelId: 'rt-pane-reminders' },
      { id: 'habits', label: tr('Привычки'), panelId: 'rt-pane-habits' },
    ],
    onchange: setRoutineTab,
  });
  const tabIcons = { routines: 'repeat', journal: 'list-bullets', todos: 'todo', reminders: 'bell', habits: 'flame' };
  for (const tab of host.querySelectorAll('.k-tab[data-id]')) {
    if (tabIcons[tab.dataset.id]) tab.insertAdjacentHTML('afterbegin', ic(tabIcons[tab.dataset.id]));
  }
}

// Клик по кнопке отдаёт сюда событие, поэтому вкладка принимается только строкой.
async function openRoutines(tab) {
  fillRoutineBots();
  renderRoutines();
  renderJournal();
  $('#rt-new').open = !ordinaryRoutines().length;
  mountRoutineTabs();
  if (!routinesCloseBound) {
    routinesCloseBound = true;
    $('#modal-routines').addEventListener('close', dropRoutineTabViews);
  }
  setRoutineTab(typeof tab === 'string' && ['routines', 'journal', 'todos', 'reminders', 'habits'].includes(tab) ? tab : 'routines');
  openModal('modal-routines');
  if (await loadJournal()) renderJournal();
  markJournalSeen();
}

async function onJournalEvent() {
  await refreshState();
  await loadJournal();
  scheduleTeam();
  if (routinesOpen()) {
    renderRoutines();
    renderJournal();
    markJournalSeen();
  }
}

// Что случилось, пока окно было закрыто, показывает экран команды: запоминаем один раз при запуске.
async function noteAway() {
  if (!(await loadJournal())) return;
  const fresh = newJournalEntries();
  if (fresh.length) {
    const needs = fresh.filter((e) => e.status === 'needs').length;
    S.teamAway = tr('Пока тебя не было: {count}', { count: `${fresh.length} ${plural(fresh.length, 'запуск', 'запуска', 'запусков')}` });
    if (needs) S.teamAway += tr(', {count} твоего решения', { count: `${needs} ${plural(needs, 'ждёт', 'ждут', 'ждут')}` });
    markJournalSeen();
  }
  scheduleTeam();
}

// ---------------------------------------------------------------------------
// Экран команды
// ---------------------------------------------------------------------------

const teamOpen = () => !$('#team').hidden;

let teamTimer = 0;

function scheduleTeam() {
  if (!teamOpen() || teamTimer) return;
  teamTimer = setTimeout(() => {
    teamTimer = 0;
    if (teamOpen()) renderTeam();
  }, 150);
}

function dropTeamAvatars() {
  for (const [slot, avatar] of S.avatars) {
    if (slot.startsWith('team:')) {
      avatar.destroy();
      S.avatars.delete(slot);
    }
  }
}

function leaveTeam() {
  closeOrchStage();
  leaveCatalog();
  leaveRemote();
  leaveWorkflows();
  if (!teamOpen()) return;
  dropTeamAvatars();
  $('#team').hidden = true;
  $('#chat').hidden = false;
}

// Чат больше не показывает бота: экран команды или каталога занимает его место.
function clearChatView() {
  if (!$('#drawer').hidden) closeDrawer();
  S.queueEdit = null;
  abortQueueDrag();
  S.view = { kind: 'bot', id: '' };
  attachFollowView();
  S.currentId = null;
  S.live = null;
  S.roomLive = null;
  $('#head-avatar').classList.remove('room-icon');
  setPanelTitle(tr('Профиль, память, файлы'));
  $('#head-reassign').hidden = true;
  $('#head-rebuild').hidden = true;
  closeOrchStage();
  closeReassignMenu();
  clearMessages();
  S.avatars.get('head')?.destroy();
  S.avatars.delete('head');
  $('#chat').hidden = true;
  closeRemoteView();
  $('#sidebar').classList.remove('open');
}

// ---------------------------------------------------------------------------
// Боты других ПК аккаунта: разделы в боковой панели и чат с удалённым ботом (Svelte, ui/src/components/remote)
// ---------------------------------------------------------------------------

let remoteChatView = null;
let remotePcsView = null;
S.remote = null;

function remotePcsProps() {
  return { api, current: S.remote, onopen: showRemote };
}

function mountRemote() {
  const plaques = $('#pair-plaques');
  if (plaques && window.DeloistUI?.mountPairPlaques) window.DeloistUI.mountPairPlaques(plaques, { api });
  const host = $('#remote-pcs');
  if (host && window.DeloistUI?.mountRemotePcs) remotePcsView = window.DeloistUI.mountRemotePcs(host, remotePcsProps());
}

function closeRemoteView() {
  if ($('#remote').hidden) return;
  remoteChatView?.destroy();
  remoteChatView = null;
  $('#remote-root').textContent = '';
  $('#remote').hidden = true;
  S.remote = null;
  remotePcsView?.update(remotePcsProps());
}

// Закрыть чат удалённого бота и вернуть обычный чат на место.
function leaveRemote() {
  if ($('#remote').hidden) return;
  closeRemoteView();
  $('#chat').hidden = false;
}

function showRemote(pc, bot) {
  if (!window.DeloistUI?.mountRemoteChat) return;
  clearChatView();
  hideCatalog();
  if (teamOpen()) {
    dropTeamAvatars();
    $('#team').hidden = true;
  }
  remoteChatView?.destroy();
  $('#remote-root').textContent = '';
  S.remote = { pc: pc.id, bot };
  $('#remote').hidden = false;
  remoteChatView = window.DeloistUI.mountRemoteChat($('#remote-root'), {
    api,
    pc: { id: pc.id, name: pc.name, online: pc.online },
    bot,
    onmenu: () => $('#sidebar').classList.toggle('open'),
  });
  $('#sidebar').classList.remove('open');
  remotePcsView?.update(remotePcsProps());
  renderSidebar();
}

function showTeam() {
  clearChatView();
  hideCatalog();
  hideWorkflows();
  $('#team').hidden = false;
  renderSidebar();
  renderTeam();
}

// ---------------------------------------------------------------------------
// Каталог: навыки, агенты, приложения (Svelte, ui/src/components/catalog)
// ---------------------------------------------------------------------------

const catalogOpen = () => !$('#catalog').hidden;

function catalogProps() {
  return {
    page: S.catalogPage || 'skills',
    bots: S.bots.map((b) => ({ id: b.id, name: b.name, color: b.color, emoji: b.emoji, skills: b.skills || [] })),
    skills: S.skills,
    stamp: S.catalogStamp || 0,
    onpage: (page) => showCatalog(page),
    onopenbot: (id) => id && selectBot(id),
  };
}

function markCatalogNav() {
  const open = catalogOpen();
  $('#btn-catalog').classList.toggle('on', open);
  $('#btn-catalog').setAttribute('aria-expanded', String(open));
  $('#catalog-nav').hidden = !open;
  $$('#catalog-nav [data-catalog]').forEach((b) => b.classList.toggle('on', open && b.dataset.catalog === (S.catalogPage || 'skills')));
}

function showCatalog(page = S.catalogPage || 'skills') {
  if (!catalogOpen()) {
    clearChatView();
    hideWorkflows();
    if (teamOpen()) {
      dropTeamAvatars();
      $('#team').hidden = true;
    }
    $('#catalog').hidden = false;
  }
  S.catalogPage = page;
  if (!window.DeloistUI?.mountCatalog) {
    $('#catalog-root').textContent = tr('Каталог не загрузился: перезапусти приложение.');
  } else if (S.catalogUi) S.catalogUi.update(catalogProps());
  else S.catalogUi = window.DeloistUI.mountCatalog($('#catalog-root'), catalogProps());
  renderSidebar();
  markCatalogNav();
}

// Компонент остаётся смонтированным, но спрятанным: повторное открытие мгновенное и без запроса.
function hideCatalog() {
  if (!catalogOpen()) return;
  $('#catalog').hidden = true;
  markCatalogNav();
}

function leaveCatalog() {
  if (!catalogOpen()) return;
  hideCatalog();
  $('#chat').hidden = false;
}

// ---------------------------------------------------------------------------
// Рабочие процессы: список и холст на Svelte Flow (ui/src/components/workflows)
// ---------------------------------------------------------------------------

const workflowsOpen = () => !$('#workflows').hidden;

function workflowBots() {
  return S.bots.map((b) => ({ id: b.id, name: b.name, color: b.color, emoji: b.emoji || '' }));
}

function markWorkflowsNav() {
  $('#btn-workflows').classList.toggle('on', workflowsOpen());
}

// Раздел собран отдельным файлом (web/ui/workflows.js): грузим при первом открытии, а не на каждой странице.
function loadWorkflowsBundle() {
  if (window.DeloistWorkflows) return Promise.resolve();
  if (S.workflowsLoad) return S.workflowsLoad;
  S.workflowsLoad = new Promise((resolve, reject) => {
    const css = document.createElement('link');
    css.rel = 'stylesheet';
    css.href = '/ui/workflows.css';
    document.head.append(css);
    const script = document.createElement('script');
    script.src = '/ui/workflows.js';
    script.onload = resolve;
    script.onerror = () => {
      S.workflowsLoad = null;
      reject(new Error('workflows.js'));
    };
    document.head.append(script);
  });
  return S.workflowsLoad;
}

// Выбор модели для шагов «Модель»: тот же список провайдеров, что в окне настроек.
function workflowModels() {
  return { ...(DeloistProviders.snapshot() || { providers: [], plan: null, loading: true }), settings: S.settings };
}

function workflowProps() {
  return {
    bots: workflowBots(),
    models: workflowModels,
    onconnect: () => DeloistProviders.open(),
    onsettings: (sub) => {
      openSettings();
      openSettingsSub(sub);
    },
    onmenu: () => $('#sidebar').classList.toggle('open'),
  };
}

function showWorkflows() {
  if (!workflowsOpen()) {
    clearChatView();
    hideCatalog();
    if (teamOpen()) {
      dropTeamAvatars();
      $('#team').hidden = true;
    }
    $('#workflows').hidden = false;
  }
  S.wfApprovals?.clear();
  markWorkflowsDot();
  if (S.workflowsUi) S.workflowsUi.update({ bots: workflowBots() });
  else {
    loadWorkflowsBundle()
      .then(() => {
        if (!S.workflowsUi) S.workflowsUi = window.DeloistWorkflows.mount($('#workflows-root'), workflowProps());
      })
      .catch(() => {
        $('#workflows-root').textContent = tr('Рабочие процессы не загрузились: перезапусти приложение.');
      });
  }
  renderSidebar();
  markWorkflowsNav();
}

// Бот просит решения шага «Одобрение», а человек на другом экране: значок на кнопке и уведомление.
function markWorkflowsDot() {
  const dot = $('#workflows-dot');
  if (dot) dot.hidden = !(S.wfApprovals?.size && !workflowsOpen());
}

function workflowApprovalEvent(ev) {
  (S.wfApprovals ||= new Set()).add(ev.step_id);
  markWorkflowsDot();
  if (workflowsOpen()) return;
  const what = ev.question || ev.title || '';
  window.DeloistUI?.toast?.(what ? tr('Процесс ждёт решения: {q}', { q: what }) : tr('Процесс ждёт вашего решения'), {
    action: { label: tr('Открыть'), onclick: () => showWorkflows() },
    duration: 9000,
  });
}

// Экран остаётся смонтированным, но спрятанным: холст не перестраивается при возврате.
function hideWorkflows() {
  if (!workflowsOpen()) return;
  $('#workflows').hidden = true;
  markWorkflowsNav();
  markWorkflowsDot();
}

function leaveWorkflows() {
  if (!workflowsOpen()) return;
  hideWorkflows();
  $('#chat').hidden = false;
}

function refreshWorkflowBots() {
  if (S.workflowsUi) S.workflowsUi.update({ bots: workflowBots() });
}

// Состояние поменялось: открытый каталог перечитывает, что стоит у человека.
function refreshCatalog() {
  if (!catalogOpen() || !S.catalogUi) return;
  S.catalogStamp = (S.catalogStamp || 0) + 1;
  S.catalogUi.update(catalogProps());
}

// Чем срочнее, тем выше: человек открывает экран, чтобы понять, куда идти первым делом.
function memberState(bot) {
  const id = bot.id;
  if (S.waiting.get(id)?.size) return { rank: 0, status: tr('Ждёт твоего решения'), detail: trServer(S.waitTitle.get(id) || ''), urgent: true };
  if (bot.preview?.error && !S.busy.has(id)) {
    const error = explainError(bot.preview.text);
    return { rank: 1, status: error.title, detail: error.hint || error.raw, urgent: true };
  }
  if (S.busy.has(id)) return { rank: 2, status: tr('Работает'), detail: bot.role || '' };
  const queued = queuedCount(id);
  if (queued) return { rank: 3, status: tr('В очереди: {count}', { count: queued }), detail: bot.role || '' };
  return { rank: 4, status: tr('Свободен'), detail: bot.role || '' };
}

function teamHeadline(states) {
  if (!S.bots.length) return tr('Ботов пока нет');
  const count = (rank) => states.filter((s) => s.rank === rank).length;
  const waiting = count(0);
  if (waiting) return `${waiting} ${plural(waiting, 'бот ждёт твоего решения', 'бота ждут твоего решения', 'ботов ждут твоего решения')}`;
  const failed = count(1);
  if (failed) return `${failed} ${plural(failed, 'бот споткнулся', 'бота споткнулись', 'ботов споткнулись')}`;
  const working = count(2);
  if (working) return `${working} ${plural(working, 'бот работает', 'бота работают', 'ботов работают')}`;
  return tr('Все свободны, можно давать задачу');
}

function teamSectionHead(title, action) {
  const head = document.createElement('div');
  head.className = 'team-section-head';
  const h2 = document.createElement('h2');
  h2.textContent = title;
  head.append(h2);
  if (action) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'ghost';
    button.textContent = action;
    button.addEventListener('click', openRoutines);
    head.append(button);
  }
  return head;
}

function teamRow(when, title, side, { urgent = false, onClick = null } = {}) {
  const row = document.createElement(onClick ? 'button' : 'div');
  row.className = 'team-row';
  if (onClick) {
    row.type = 'button';
    row.addEventListener('click', onClick);
  }
  const whenEl = document.createElement('span');
  whenEl.className = 'team-row-when';
  whenEl.textContent = when;
  const titleEl = document.createElement('span');
  titleEl.className = 'team-row-title';
  titleEl.textContent = title;
  const sideEl = document.createElement('span');
  sideEl.className = `team-row-side${urgent ? ' urgent' : ''}`;
  sideEl.textContent = side;
  row.append(whenEl, titleEl, sideEl);
  return row;
}

function renderTeam() {
  const body = $('#team-body');
  const focused = document.activeElement?.closest?.('.team-member')?.dataset.id;
  $('#team-demo').hidden = S.settings.driver !== 'demo';

  const members = S.bots
    .map((bot) => ({ bot, state: memberState(bot) }))
    .sort((a, b) => a.state.rank - b.state.rank || Number(b.bot.pinned) - Number(a.bot.pinned) || (b.bot.preview?.at || 0) - (a.bot.preview?.at || 0));

  const parts = [];
  const headline = document.createElement('h1');
  headline.className = 'team-headline';
  headline.textContent = teamHeadline(members.map((m) => m.state));
  parts.push(headline);
  const liveTeam = window.DeloistTeam?.teamBlock();
  if (liveTeam) parts.push(liveTeam);
  if (S.teamAway) {
    const away = document.createElement('p');
    away.className = 'team-away';
    away.textContent = S.teamAway;
    parts.push(away);
  }

  if (!S.bots.length) {
    const empty = document.createElement('p');
    empty.className = 'team-empty';
    empty.textContent = tr('Создай первого бота: у него будет своя папка и своя память, а перед опасным действием он спросит тебя.');
    const create = document.createElement('button');
    create.type = 'button';
    create.className = 'primary';
    create.textContent = tr('Создать бота');
    create.addEventListener('click', () => openModal('modal-new'));
    parts.push(empty, create);
    body.replaceChildren(...parts);
    return;
  }

  const roster = document.createElement('div');
  roster.className = 'team-roster';
  for (const { bot, state } of members) {
    const member = document.createElement('button');
    member.type = 'button';
    member.className = 'team-member';
    member.dataset.id = bot.id;
    const avatar = document.createElement('div');
    avatar.className = 'avatar';
    const name = document.createElement('div');
    name.className = 'team-name';
    name.textContent = bot.name;
    const status = document.createElement('div');
    status.className = `team-status${state.urgent ? ' urgent' : ''}`;
    status.textContent = state.status;
    const detail = document.createElement('div');
    detail.className = 'team-detail';
    detail.textContent = state.detail;
    member.append(avatar, name, status, detail);
    member.addEventListener('click', () => selectBot(bot.id));
    roster.append(member);
    // Живых аватаров дюжина: дальше они оживают только под мышью, иначе съедят процессор.
    paintAvatar(avatar, bot, `team:${bot.id}`, { size: 72, still: S.bots.length > 12 });
  }
  parts.push(roster);

  const next = S.routines
    .filter((r) => r.enabled && Number(r.nextRun) > 0)
    .sort((a, b) => Number(a.nextRun) - Number(b.nextRun))
    .slice(0, 3);
  const schedule = document.createElement('section');
  schedule.className = 'team-section';
  if (next.length) {
    schedule.append(teamSectionHead(tr('Дальше по расписанию'), tr('Все рутины')));
    for (const r of next) schedule.append(teamRow(nextText(r.nextRun), r.title, botById(r.botId)?.name || ''));
  } else {
    schedule.append(teamSectionHead(tr('Дальше по расписанию'), tr('Настроить рутину')));
    const none = document.createElement('p');
    none.className = 'team-quiet';
    none.textContent = tr('Пока ничего. Рутина запускает бота сама, например утром или ночью.');
    schedule.append(none);
  }
  parts.push(schedule);

  const important = S.journal.entries.filter((e) => e.status !== 'quiet');
  const quiet = S.journal.entries.length - important.length;
  if (S.journal.entries.length) {
    const log = document.createElement('section');
    log.className = 'team-section';
    log.append(teamSectionHead(tr('Что было')));
    for (const e of important.slice(0, 6)) {
      const urgent = ['needs', 'failed', 'limit'].includes(e.status);
      const open = botById(e.botId) ? () => selectBot(e.botId) : null;
      log.append(teamRow(whenText(e.at), trServer(e.title), STATUS_LABELS[e.status] || e.botName || '', { urgent, onClick: open }));
    }
    if (quiet) {
      const line = document.createElement('p');
      line.className = 'team-quiet';
      const count = `${quiet} ${plural(quiet, 'тихая проверка', 'тихие проверки', 'тихих проверок')}`;
      line.textContent = important.length ? tr('И ещё {count}, сообщать было нечего', { count }) : tr('{count}, сообщать было нечего', { count });
      log.append(line);
    }
    parts.push(log);
  }

  body.replaceChildren(...parts);
  if (focused) $(`.team-member[data-id="${CSS.escape(focused)}"]`, body)?.focus();
}

function fillRoutineBots() {
  const sel = $('#routine-bot');
  sel.innerHTML = S.bots.map((b) => `<option value="${b.id}">${escapeHtml(b.name)}</option>`).join('');
  if (S.currentId) sel.value = S.currentId;
}

// ---------------------------------------------------------------------------
// Старт
// ---------------------------------------------------------------------------

// Новая установка: «Главная команда» с одним оркестратором ждёт расспроса — её и открываем, если больше открывать нечего.
function firstRunRoom() {
  if (S.bots.some((b) => !isOrchestrator(b))) return null;
  return S.rooms.find((r) => interviewBot(r)) || null;
}

window.addEventListener('deloist:team-chosen', async () => {
  await refreshState();
  if (inRoom() || (S.currentId && !teamOpen())) return;
  const first = firstRunRoom();
  if (first) selectRoom(first.id);
});

function selectFirst() {
  const intro = firstRunRoom();
  if (intro) {
    selectRoom(intro.id);
    return;
  }
  const first = [...S.bots].sort((a, b) => Number(b.pinned) - Number(a.pinned))[0];
  if (first) selectBot(first.id);
  else {
    $('#head-name').textContent = tr('Ботов пока нет');
    $('#head-role').textContent = tr('создай первого бота кнопкой +');
    $('#input').disabled = true;
    $('#btn-send').disabled = true;
  }
}

function bindUi() {
  $('#btn-search-toggle').addEventListener('click', () => {
    if (!$('#sidebar-search').hidden && !$('#search').value) setSearchOpen(false);
    else setSearchOpen(true);
  });
  $('#search').addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !e.currentTarget.value) { e.preventDefault(); setSearchOpen(false); $('#btn-search-toggle').focus(); }
  });
  $('#search').addEventListener('input', (e) => {
    S.filter = e.target.value;
    renderSidebar();
  });

  $('#btn-new').addEventListener('click', toggleNewMenu);
  bindSelfDev();
  $('#skill-add').addEventListener('click', async () => {
    const name = $('#skill-name').value.trim();
    const body = $('#skill-body').value.trim();
    if (!name || !body) {
      toast(tr('Нужны название и правила'));
      return;
    }
    try {
      await api('/api/skills', { method: 'POST', body: { name, description: $('#skill-desc').value.trim(), body } });
      $('#skill-name').value = '';
      $('#skill-desc').value = '';
      $('#skill-body').value = '';
      await refreshState();
      renderSkills();
      toast(tr('Навык добавлен'));
    } catch (err) {
      toast(err.message);
    }
  });
  $('#mcp-add').addEventListener('click', addMcp);
  $('#mcp-transport').addEventListener('change', syncMcpTransport);
  $('#team-export').addEventListener('click', exportTeam);
  $('#team-import').addEventListener('click', () => $('#team-file').click());
  $('#team-file').addEventListener('change', (e) => {
    const file = e.target.files?.[0];
    if (file) importTeam(file);
    e.target.value = '';
  });
  $('#btn-settings').addEventListener('click', () => openSettings());
  $('#btn-know-me')?.addEventListener('click', () => { openKnowMe(); });
  const wnBtn = $('#upd-whatsnew');
  if (wnBtn) wnBtn.addEventListener('click', async () => {
    try {
      const data = await api('/api/whats-new');
      openWhatsNew(data);
    } catch (err) {
      toast(err.message, 5000);
    }
  });
  $('#btn-needs')?.addEventListener('click', () => {
    $('#needs-list').hidden = !$('#needs-list').hidden;
    renderNeeds();
  });
  // Назад: из карточки провайдера к списку, из подэкрана к его разделу.
  $('#settings-back')?.addEventListener('click', () => {
    if (!settingsSub && DeloistProviders.inDetail()) DeloistProviders.showList();
    else closeSettingsSub();
  });
  $('#own-models-fix')?.addEventListener('click', moveBotsToMainModel);
  $('#form-settings').driver.addEventListener('change', () => {
    const form = $('#form-settings');
    if (form.driver.value.startsWith('acp:')) form.acpModel.innerHTML = providerModelOptions(form.driver.value, '');
    const native = form.driver.value === 'anthropic' || form.driver.value === 'gemini';
    // Адрес OpenAI не должен уехать Anthropic или Gemini: пустое поле значит «адрес провайдера».
    if (native && form.baseUrl.value.includes('api.openai.com')) form.baseUrl.value = '';
    syncSettingsFields();
  });
  $('#form-settings').fallbackDriver.addEventListener('change', syncSettingsFields);
  // Питомцы применяются сразу, а не по кнопке «Сохранить»: так видно, что выбираешь.
  $('#obscura-action').addEventListener('click', obscuraAction);
  $('#moli-action').addEventListener('click', moliAction);
  $('#browser-engine').addEventListener('change', () => {
    renderObscura();
    renderMoli();
  });
  $('#form-settings').petsMode.addEventListener('change', async (e) => {
    await saveLook({ petsMode: e.target.value });
    await refreshState();
    markPets();
  });
  const saveDock = async (body) => {
    await saveLook(body);
    markDock();
  };
  $('#dock-enabled').addEventListener('change', (e) => saveDock({ dockEnabled: e.target.checked }));
  $('#dock-layer').addEventListener('change', (e) => saveDock({ dockLayer: e.target.value }));
  $('#dock-display').addEventListener('change', (e) => saveDock({ dockDisplay: e.target.value }));
  $('#dock-edge').addEventListener('change', (e) => saveDock({ dockEdge: e.target.value }));
  $('#dock-autohide').addEventListener('change', (e) => saveDock({ dockAutohide: e.target.checked }));
  $('#dock-results').addEventListener('change', (e) => saveDock({ dockResults: e.target.checked }));
  $('#dock-activity').addEventListener('change', (e) => saveDock({ dockActivity: e.target.checked }));
  $('#head-model').addEventListener('click', openHeadModelPopover);
  $('#head-reassign').addEventListener('click', openReassignMenu);
  $('#head-rebuild').addEventListener('click', rebuildTeam);
  document.addEventListener('click', (e) => {
    const trigger = e.target.closest('[data-open-settings]');
    if (!trigger) return;
    // Настройки открывает главное окно: во второй панели им тесно.
    if (PANE) parent.postMessage({ deloist: 'pane-settings', section: trigger.dataset.openSettings || 'model' }, location.origin);
    else openSettings(trigger.dataset.openSettings || 'model');
  });
  // У ссылок на файлы настоящий href (для «Копировать ссылку»), но окно по file: не переходит нигде:
  // ни вне переписки (карточки разрешений), ни средней кнопкой в новое окно.
  for (const type of ['click', 'auxclick']) {
    document.addEventListener(type, (e) => {
      if (e.target.closest?.('a.md-file')) e.preventDefault();
    });
  }
  // Ссылка на файл из ответа бота: открываем его в ящике «Файлы» (путь проверяет сервер), а не по file:.
  $('#messages').addEventListener('click', async (e) => {
    const link = e.target.closest('a.md-file');
    if (!link) return;
    e.preventDefault();
    const ref = window.DeloistUI.workspaceRef(link.dataset.file);
    if (!ref || !botById(ref.owner)) {
      toast(tr('Файл лежит вне папок ботов, открыть его здесь нельзя'));
      return;
    }
    if (S.currentId !== ref.owner || inRoom()) await selectBot(ref.owner);
    S.openFile = ref.rel;
    openDrawer('files');
  });
  // Пока окно закрыто, группы не должны выглядеть открытыми: иначе «Мой Chrome»
  // продолжает опрашивать состояние в фоне.
  $('#modal-settings').addEventListener('close', () => {
    $$('#form-settings details.group').forEach((group) => { group.open = false; });
  });
  $('#btn-routines').addEventListener('click', openRoutines);
  $('#btn-usage').addEventListener('click', openUsage);
  $('#modal-usage').addEventListener('close', () => {
    usageView?.destroy();
    usageView = null;
  });
  bindSharedMemory();
  $('#btn-compact').addEventListener('click', compactCurrent);
  bindRoutineForm();
  // Рутины правили в окне «Рутины» — вкладка панели бота показывает то же.
  $('#modal-routines').addEventListener('close', () => {
    if (drawerShown() && S.tab === 'routines') renderDrawer();
  });
  $('#btn-panel').addEventListener('click', () => {
    // У группы панель — это её настройки: профиля, памяти и файлов у неё нет.
    if (inRoom()) openRoomModal(currentRoom());
    else if (drawerShown()) collapseDrawer();
    else openDrawer(S.tab);
  });
  $('#btn-menu').addEventListener('click', () => $('#sidebar').classList.toggle('open'));
  $('#btn-team-menu').addEventListener('click', () => $('#sidebar').classList.toggle('open'));
  $('#btn-team').addEventListener('click', showTeam);
  $('#btn-catalog').addEventListener('click', () => showCatalog());
  $$('#catalog-nav [data-catalog]').forEach((b) => b.addEventListener('click', () => showCatalog(b.dataset.catalog)));
  $('#btn-catalog-menu').addEventListener('click', () => $('#sidebar').classList.toggle('open'));
  $('#btn-workflows').addEventListener('click', showWorkflows);
  $$('#theme-switch [data-value]').forEach((btn) => btn.addEventListener('click', () => chooseTheme(btn.dataset.value)));
  $$('.theme-toggle').forEach((btn) => btn.addEventListener('click', toggleTheme));
  initSidebarCollapse();
  initDrawerResize();
  bindTeamDrop();
  bindFileRefDrop();
  watchNewRows();
  Toc.init();
  window.addEventListener('keydown', arcShortcuts);
  $$('#font-switch [data-value]').forEach((btn) => btn.addEventListener('click', () => chooseFont(btn.dataset.value)));
  $$('#lang-switch [data-value]').forEach((btn) => btn.addEventListener('click', () => chooseLang(btn.dataset.value)));

  const input = $('#input');
  const mentionHost = document.createElement('div');
  mentionHost.id = 'mention-host';
  document.body.appendChild(mentionHost);
  S.mentionPicker = window.DeloistUI?.mountMentionPopover
    ? window.DeloistUI.mountMentionPopover(mentionHost, { bots: S.bots || [], input })
    : null;
  input.addEventListener('input', () => {
    autosize();
    setSendState();
    listenToInput(Boolean(input.value.trim()));
  });
  mountChatDates(input);
  input.addEventListener('keydown', (e) => {
    // Список «@/+Имя» уже забрал Enter (MentionPopover): сообщение не уходит.
    if (e.defaultPrevented) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    } else if (e.key === 'Escape' && viewBusy()) {
      e.preventDefault();
      stopCurrent();
    }
  });

  $('#btn-send').addEventListener('click', () => sendMessage());
  attachBox = Attach.composer({
    root: $('#chat'),
    textarea: input,
    pick: $('#btn-attach'),
    picker: $('#attach-input'),
    list: $('#attach-list'),
    hint: $('#drop-hint'),
    endpoint: attachEndpoint,
    post: (url, body) => api(url, { method: 'POST', body }),
    notify: toast,
    onChange: setSendState,
  });
  // Голос и снимок экрана с горячих клавиш (voice.js): открыть нужного бота и приложить готовый файл.
  window.DeloistApp = { selectBot, currentBot: () => S.currentId, addSaved: (item) => attachBox.addSaved(item), sendMessage: (opts) => sendMessage(opts || {}) };
  $('#btn-stop').addEventListener('click', () => stopCurrent());

  $('#mode-switch').addEventListener('click', () => {
    setMode($('#mode-switch').getAttribute('aria-checked') === 'true' ? 'ask' : 'auto');
  });
  $('#grants-btn').addEventListener('click', () => setGrantsOpen(!grantsOpen));
  document.addEventListener('click', (e) => {
    if (grantsOpen && !e.target.closest('#grants-line')) setGrantsOpen(false);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && grantsOpen) {
      setGrantsOpen(false);
      $('#grants-btn').focus();
    }
  });
  // Список мессенджеров в шапке меняется: часть разрешений уходит в их карточки или возвращается в щит.
  window.addEventListener('deloist:messengers', paintGrantsShield);

  $('#queue-resume').addEventListener('click', async () => {
    if (!S.currentId) return;
    try {
      await api(`/api/bots/${S.currentId}/queue/resume`, { method: 'POST' });
      await loadQueue(S.currentId);
    } catch (err) {
      toast(err.message);
    }
  });

  $$('[data-close]').forEach((b) =>
    b.addEventListener('click', () => b.closest('dialog')?.close()),
  );

  const newForm = $('#form-new');
  newBotPicker = mountNewBotModel($('#nb-model'), {
    value: { driver: '', model: '' },
    onselect: (next) => {
      newForm.driver.value = next.driver;
      newForm.model.value = next.model;
    },
  });
  $('#modal-new').addEventListener('close', clearNewLook);
  $('#modal-providers').addEventListener('close', reloadModelPickers);
  $('#form-new').addEventListener('submit', async (e) => {
    e.preventDefault();
    const f = e.target;
    try {
      const { bot } = await api('/api/bots', {
        method: 'POST',
        body: {
          name: f.name.value.trim(),
          role: f.role.value.trim(),
          persona: f.persona.value.trim(),
          driver: f.driver.value,
          model: f.model.value.trim(),
          ...newBotLookFields(),
        },
      });
      const team = f.team?.value ? roomById(f.team.value) : null;
      $('#modal-new').close();
      f.reset();
      newBotPicker.picker.update({ value: { driver: '', model: '' } });
      // Бот уже создан: если команда не приняла его, он просто остаётся «без команды».
      if (team) {
        try {
          await setTeamMembers(team.id, [...team.botIds, bot.id]);
        } catch (err) {
          toast(err.message);
        }
      }
      await refreshState();
      await selectBot(bot.id);
      toast(team ? tr('{name} в команде «{team}»', { name: bot.name, team: team.name }) : tr('{name} в команде', { name: bot.name }));
    } catch (err) {
      toast(err.message);
    }
  });

  $('#btn-browser').addEventListener('click', async () => {
    if (!S.currentId) return;
    // Экран бота в программе; окно Chrome выводится оттуда же кнопкой «Открыть окно».
    if (window.DeloistScreen) {
      window.DeloistScreen.toggle(S.currentId);
      return;
    }
    try {
      await api(`/api/bots/${S.currentId}/browser`, { method: 'POST' });
      toast(tr('Окно браузера открыто. Войди в кабинет и просто закрой его: вход сохранится.'));
    } catch (err) {
      toast(err.message);
    }
  });

  $('#form-settings').addEventListener('submit', async (e) => {
    e.preventDefault();
    const f = e.target;
    // Пустой драйвер (поля ещё не заполнены) значит «не менять», а не «сбросить».
    const driver = f.driver.value || S.settings.driver || 'demo';
    const body = {
      driver,
      // У подписки Deloist модель выбирается из списка, а не вписывается руками.
      model: !f.driver.value ? S.settings.model || '' : driver === 'deloist' ? f.planModel.value : driver.startsWith('acp:') ? f.acpModel.value : f.model.value.trim(),
      baseUrl: f.baseUrl.value.trim(),
      apiKeyEnv: f.apiKeyEnv.value.trim(),
      cliCommand: f.cliCommand.value.trim(),
      acpCommand: f.acpCommand.value.trim(),
      fallbackDriver: f.fallbackDriver.value,
      fallbackModel: f.fallbackModel.value.trim(),
      fallbackBaseUrl: f.fallbackBaseUrl.value.trim(),
      fallbackCliCommand: f.fallbackCliCommand.value.trim(),
      temperature: Number(f.temperature.value),
      historyLimit: Number(f.historyLimit.value),
      vision: f.vision.checked,
      browserPath: f.browserPath.value.trim(),
      browserEngine: f.browserEngine.value,
      imageBaseUrl: f.imageBaseUrl.value.trim(),
      imageModel: f.imageModel.value.trim(),
      imageDaily: Math.max(0, Math.floor(Number(f.imageDaily.value) || 0)),
      dailyTokenLimit: Math.max(0, Math.floor(Number(f.dailyTokenLimit.value) || 0)),
      autostart: f.autostart.checked,
      keepAwake: f.keepAwake.checked,
      checkUpdates: f.checkUpdates.checked,
      autoUpdate: f.autoUpdate.checked,
      petsMode: f.petsMode.value,
      skillLearning: f.skillLearning.value,
    };
    if (f.apiKey.value.trim()) body.apiKey = f.apiKey.value.trim();
    if (f.anthropicApiKey.value.trim()) body.anthropicApiKey = f.anthropicApiKey.value.trim();
    if (f.geminiApiKey.value.trim()) body.geminiApiKey = f.geminiApiKey.value.trim();
    if (f.fallbackApiKey.value.trim()) body.fallbackApiKey = f.fallbackApiKey.value.trim();
    if (f.imageApiKey.value.trim()) body.imageApiKey = f.imageApiKey.value.trim();
    try {
      const before = `${S.settings.driver}|${S.settings.model}`;
      const { settings } = await api('/api/settings', { method: 'PUT', body });
      S.settings = settings;
      $('#modal-settings').close();
      toast(settings.hasKey ? tr('Настройки сохранены, ключ на месте') : tr('Настройки сохранены'));
      if (before !== `${settings.driver}|${settings.model}`) offerOwnModels();
      if (S.currentId) selectBot(S.currentId);
    } catch (err) {
      toast(err.message);
    }
  });
}

function selectedDays() {
  return $$('#rt-days .day-btn')
    .filter((b) => b.getAttribute('aria-pressed') === 'true')
    .map((b) => Number(b.dataset.day));
}

function syncRoutineMode() {
  const hook = $('#rt-hook')?.checked;
  const atMode = $('#routine-mode').value === 'at';
  $('#rt-every-box').hidden = hook || atMode;
  $('#rt-at-box').hidden = hook || !atMode;
  const days = $('#rt-days').closest('.field');
  if (days) days.hidden = Boolean(hook);
}

function bindRoutineForm() {
  $('#routine-mode').addEventListener('change', syncRoutineMode);
  $('#rt-hook').addEventListener('change', () => {
    const hook = $('#rt-hook').checked;
    $('#rt-hook-box').hidden = !hook || !$('#rt-hook-url').value;
    // Расписание и вебхук взаимоисключающие: одно и то же не настраиваем дважды.
    $('#routine-mode').disabled = hook;
    $('#rt-every-box').hidden = hook || $('#routine-mode').value === 'at';
    $('#rt-at-box').hidden = hook || $('#routine-mode').value !== 'at';
    $('#rt-days').closest('.field').hidden = hook;
  });
  $$('#rt-days .day-btn').forEach((b) =>
    b.addEventListener('click', () => b.setAttribute('aria-pressed', String(b.getAttribute('aria-pressed') !== 'true'))),
  );
  $('#rt-night').addEventListener('click', () => {
    const f = $('#form-routine');
    f.windowFrom.value = '22:00';
    f.windowTo.value = '07:00';
  });

  $('#form-routine').addEventListener('submit', async (e) => {
    e.preventDefault();
    const f = e.target;
    const atMode = f.mode.value === 'at';
    if (atMode && !f.at.value) {
      toast(tr('Выбери время запуска'));
      return;
    }
    const body = {
      botId: f.botId.value,
      title: f.title.value.trim(),
      prompt: f.prompt.value.trim(),
      everyMinutes: Number(f.everyMinutes.value) || 60,
      days: selectedDays(),
      at: atMode ? f.at.value : '',
      windowFrom: atMode ? '' : f.windowFrom.value,
      windowTo: atMode ? '' : f.windowTo.value,
      quiet: f.quiet.checked,
      hook: $('#rt-hook').checked,
    };
    try {
      const { routine } = await api('/api/routines', { method: 'POST', body });
      S.routines.push(routine);
      if (drawerShown() && S.tab === 'routines') renderDrawer();
      renderRoutines();
      f.reset();
      $$('#rt-days .day-btn').forEach((b) => b.setAttribute('aria-pressed', 'false'));
      // Форма вернулась в обычный вид: расписание снова доступно.
      $('#routine-mode').disabled = false;
      $('#rt-hook-box').hidden = true;
      $('#rt-hook-url').value = '';
      syncRoutineMode();
      fillRoutineBots();
      if (routine.hook) {
        // Адрес показываем сразу: именно его нужно вставить во внешний сервис.
        $('#rt-hook-box').hidden = false;
        $('#rt-hook-url').value = hookUrl(routine);
        $('#rt-hook-url').select?.();
        toast(tr('Адрес для вызова на экране'));
      } else {
        toast(tr('Рутина добавлена'));
      }
    } catch (err) {
      toast(err.message);
    }
  });
}

// Баланс подписки Deloist в окне аккаунта. Пополнение видно только у вошедших.
function paintBalance() {
  const account = window.DeloistAccount || {};
  const balance = document.getElementById('account-balance');
  const topup = document.getElementById('account-topup');
  if (!account.signedIn) {
    balance.hidden = true;
    topup.hidden = true;
    return Promise.resolve();
  }
  topup.hidden = false;
  return api('/api/account/wallet')
    .then((w) => {
      const rub = (Number(w.balance_kop) || 0) / 100;
      document.getElementById('account-balance-sum').textContent =
        rub.toLocaleString(window.LOCALE, { style: 'currency', currency: 'RUB', minimumFractionDigits: 2 });
      balance.hidden = false;
    })
    .catch(() => {});
}

// Возврат денег, выгрузка данных, цены и удаление аккаунта (компонент кита).
let accountRights = null;
function paintRights(account) {
  const box = document.getElementById('account-rights');
  accountRights?.destroy();
  accountRights = null;
  box.hidden = !account.signedIn;
  if (!account.signedIn || !window.DeloistUI?.mountAccountRights) return;
  accountRights = DeloistUI.mountAccountRights(box, {
    api,
    siteUrl: account.siteUrl || '',
    onnotice: (text) => toast(text, 8000),
    ondeleted: () => location.reload(),
  });
}

function openAccount() {
  const account = window.DeloistAccount || {};
  document.getElementById('account-name').textContent = account.name || account.email || tr('Вход не выполнен');
  document.getElementById('account-email').textContent = account.name ? account.email || '' : '';
  document.getElementById('account-logout').hidden = !account.signedIn;
  document.getElementById('account-balance').hidden = true;
  paintRights(account);
  const modal = document.getElementById('modal-account');
  if (!modal.open) modal.showModal();
  paintBalance();
}

function bindAccount() {
  const modal = document.getElementById('modal-account');
  document.getElementById('btn-account').addEventListener('click', openAccount);
  // Оплата идёт в браузере: когда человек вернулся в окно, баланс перечитываем.
  window.addEventListener('focus', () => { if (modal.open) paintBalance(); });
  document.querySelectorAll('#account-topup [data-topup]').forEach((btn) => btn.addEventListener('click', async () => {
    btn.disabled = true;
    try {
      const res = await api('/api/account/topup', { method: 'POST', body: { amount: Number(btn.dataset.topup) } });
      window.open(res.url, '_blank');
    } catch (err) {
      if (err.status === 451) DeloistSignin.askConsent();
      toast(trServer(err.message));
    } finally {
      btn.disabled = false;
    }
  }));
  // Сайт ответил 451 (баланс Deloist ждёт согласия): показываем документы.
  window.addEventListener('deloist:event', (e) => {
    if (e.detail?.type === 'consent_required') DeloistSignin.askConsent();
  });
  // Подписку выбрали, но подарок на этом компьютере уже был: сразу к пополнению.
  window.addEventListener('deloist:ready', () => {
    if (window.DeloistAccount?.giftDenied) {
      openAccount();
      toast(tr('Подарок на этом компьютере уже получали. Пополни баланс, чтобы начать'), 6000);
    }
  }, { once: true });
  document.getElementById('account-logout').addEventListener('click', async () => {
    try {
      await DeloistSignin.signOut();
    } catch (err) {
      toast(trServer(err.message));
    }
  });
  document.getElementById('account-shortcut').addEventListener('click', async () => {
    try {
      await api('/api/shortcut', { method: 'POST' });
      toast(tr('Ярлык уже на рабочем столе'));
    } catch (err) {
      toast(trServer(err.message));
    }
  });
}

// ---------------------------------------------------------------------------
// Разделённое окно: второй чат рядом или снизу
// ---------------------------------------------------------------------------

// Второй чат — эта же страница во фрейме (?pane=1): сессия, потоки событий и все возможности чата те же,
// второго рендера в одном документе нет. Родитель держит рамку, разделитель и подсказки перетаскивания.
function openPaneView(view = PANE) {
  const room = view.room || (view.kind === 'room' ? view.id : '');
  const bot = view.bot || (view.kind === 'bot' ? view.id : '');
  if (room && roomById(room)) selectRoom(room);
  else if (bot && botById(bot)) selectBot(bot);
  else parent.postMessage({ deloist: 'pane-close' }, location.origin);
}

const Split = (() => {
  const KEY = 'deloist-split';
  const MIME = 'application/x-deloist-view';
  const MIN = { row: 320, col: 240 };
  const ICON = { row: 'split-row', col: 'split-col', close: 'close', flip: 'split-flip' };
  let pane = null; // { kind: 'bot' | 'room', id }
  let orient = 'row';
  let ratio = 0.5;
  let box = null;
  let frame = null;
  let bar = null;
  let zone = null;
  let zoneText = null;
  let menuView = null;
  let frameView = null;
  let dragView = null;
  let zoneOn = false;

  const same = (a, b) => Boolean(a && b && a.kind === b.kind && a.id === b.id);
  const exists = (v) => Boolean(v && (v.kind === 'room' ? roomById(v.id) : botById(v.id)));
  const narrow = () => innerWidth < 760;
  const eff = () => (narrow() ? 'col' : orient);
  const body = () => document.body;
  const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, n));
  // Что сейчас стоит в главной панели; на экране команды или каталога чата нет.
  const mainView = () => (!PANE && !$('#chat').hidden && S.view?.id ? { kind: S.view.kind, id: S.view.id } : null);
  const sideWidth = () => (innerWidth > 820 ? $('#sidebar').getBoundingClientRect().width : 0);

  function save() {
    try {
      localStorage.setItem(KEY, JSON.stringify({ orientation: orient, ratio, view: pane }));
    } catch {}
  }

  function load() {
    try {
      const saved = JSON.parse(localStorage.getItem(KEY) || 'null');
      if (!saved) return null;
      if (saved.orientation === 'row' || saved.orientation === 'col') orient = saved.orientation;
      if (typeof saved.ratio === 'number' && saved.ratio > 0.1 && saved.ratio < 0.9) ratio = saved.ratio;
      const v = saved.view;
      return v && (v.kind === 'bot' || v.kind === 'room') && typeof v.id === 'string' ? { kind: v.kind, id: v.id } : null;
    } catch {
      return null;
    }
  }

  function ensureDom() {
    if (box) return;
    box = document.createElement('div');
    box.id = 'split-pane';
    box.className = 'split-pane';
    box.hidden = true;
    bar = document.createElement('div');
    bar.className = 'split-bar';
    bar.setAttribute('role', 'separator');
    bar.tabIndex = 0;
    bar.title = tr('Потяни, чтобы изменить размер. Двойной щелчок — поровну');
    bar.setAttribute('aria-label', tr('Размер панелей'));
    frame = document.createElement('iframe');
    frame.className = 'split-frame';
    frame.title = tr('Второй чат');
    box.append(bar, frame);
    $('#app').append(box);

    zone = document.createElement('div');
    zone.className = 'split-zone';
    zone.setAttribute('aria-hidden', 'true');
    zoneText = document.createElement('span');
    zone.append(zoneText);
    document.body.append(zone);

    bar.addEventListener('pointerdown', startResize);
    bar.addEventListener('dblclick', () => setRatio(0.5));
    bar.addEventListener('keydown', (e) => {
      const grow = eff() === 'row' ? 'ArrowLeft' : 'ArrowUp';
      const shrink = eff() === 'row' ? 'ArrowRight' : 'ArrowDown';
      if (e.key === grow) setRatio(ratio + 0.02);
      else if (e.key === shrink) setRatio(ratio - 0.02);
      else if (e.key === 'Home') setRatio(0.5);
      else return;
      e.preventDefault();
    });
  }

  function setFrame() {
    if (!pane) return;
    if (same(frameView, pane)) return;
    const live = frameView && frame.contentWindow && frame.dataset.ready === '1';
    frameView = { ...pane };
    if (live) {
      frame.contentWindow.postMessage({ deloist: 'pane-open', kind: pane.kind, id: pane.id }, location.origin);
      return;
    }
    frame.dataset.ready = '';
    frame.src = `/?pane=1&${pane.kind}=${encodeURIComponent(pane.id)}`;
  }

  function paint() {
    const on = Boolean(pane);
    const o = eff();
    const b = body();
    b.classList.toggle('split-on', on);
    b.classList.toggle('split-row', on && o === 'row');
    b.classList.toggle('split-col', on && o === 'col');
    b.style.setProperty('--split-ratio', String(ratio));
    const btn = $('#btn-split');
    if (btn) {
      btn.classList.toggle('on', on);
      const label = on ? tr('Разделённое окно') : tr('Открыть второй чат');
      btn.title = label;
      btn.setAttribute('aria-label', label);
    }
    if (bar) {
      bar.setAttribute('aria-orientation', o === 'row' ? 'vertical' : 'horizontal');
      bar.setAttribute('aria-valuemin', '10');
      bar.setAttribute('aria-valuemax', '90');
      bar.setAttribute('aria-valuenow', String(Math.round(ratio * 100)));
    }
  }

  function open(next, mode) {
    if (!exists(next)) return;
    if (same(mainView(), next)) {
      toast(tr('Этот чат уже открыт в главной панели'));
      return;
    }
    ensureDom();
    const was = Boolean(pane);
    pane = { kind: next.kind, id: next.id };
    if (mode) orient = mode;
    box.hidden = false;
    setFrame();
    paint();
    save();
    decorate();
    if (!was) {
      box.classList.remove('split-in');
      void box.offsetWidth;
      box.classList.add('split-in');
      box.addEventListener('animationend', () => box.classList.remove('split-in'), { once: true });
    }
  }

  function close() {
    if (!pane) return;
    pane = null;
    frameView = null;
    save();
    paint();
    decorate();
    closeMenu();
    if (!box) return;
    box.hidden = true;
    frame.dataset.ready = '';
    frame.src = 'about:blank'; // поток событий и таймеры второй панели останавливаются вместе со страницей
  }

  function setRatio(next) {
    const a = $('#app').getBoundingClientRect();
    const o = eff();
    const total = o === 'row' ? a.width - sideWidth() - (innerWidth > 820 ? 16 : 24) : a.height - 24;
    const lo = Math.min(0.5, MIN[o] / Math.max(total, 1));
    ratio = clamp(next, lo, 1 - lo);
    paint();
    save();
  }

  function startResize(e) {
    if (!pane || e.button > 0) return;
    e.preventDefault();
    bar.setPointerCapture(e.pointerId);
    body().classList.add('split-drag');
    const move = (ev) => {
      const a = $('#app').getBoundingClientRect();
      if (eff() === 'row') {
        const total = a.width - sideWidth() - (innerWidth > 820 ? 16 : 24);
        setRatio((a.right - 8 - (ev.clientX + 4)) / total);
      } else {
        setRatio((a.bottom - 8 - (ev.clientY + 4)) / (a.height - 24));
      }
    };
    const end = () => {
      bar.removeEventListener('pointermove', move);
      bar.removeEventListener('pointerup', end);
      bar.removeEventListener('pointercancel', end);
      body().classList.remove('split-drag');
    };
    bar.addEventListener('pointermove', move);
    bar.addEventListener('pointerup', end);
    bar.addEventListener('pointercancel', end);
  }

  // ---- меню в шапке чата ----

  function closeMenu(refocus = false) {
    if (!menuView) return;
    menuView.destroy();
    menuView = null;
    $('#head-split-pop').hidden = true;
    $('#btn-split').setAttribute('aria-expanded', 'false');
    if (refocus) $('#btn-split').focus({ preventScroll: true });
  }

  function mountMenu(items, label, onpick) {
    const pop = $('#head-split-pop');
    menuView?.destroy();
    pop.hidden = false;
    $('#btn-split').setAttribute('aria-expanded', 'true');
    menuView = window.DeloistUI.mountMenu(pop, { items, label, onpick, onclose: (focus) => closeMenu(focus) });
  }

  function pickList(mode) {
    const main = mainView();
    const items = [{ id: 'n', kind: 'note', label: tr('Какой чат открыть второй панелью') }];
    const bots = S.bots.filter((b) => !same(main, { kind: 'bot', id: b.id })).slice(0, 14);
    for (const b of bots) items.push({ id: `bot:${b.id}`, label: b.name, hint: b.role || '' });
    const rooms = S.rooms.filter((r) => !r.archived && !same(main, { kind: 'room', id: r.id })).slice(0, 6);
    if (rooms.length && bots.length) items.push({ kind: 'sep' });
    for (const r of rooms) items.push({ id: `room:${r.id}`, label: r.name, hint: tr('Команда') });
    if (!bots.length && !rooms.length) items.push({ id: 'none', label: tr('Других чатов пока нет'), disabled: true });
    mountMenu(items, tr('Открыть второй чат'), (id) => {
      const [kind, ...rest] = String(id).split(':');
      closeMenu(true);
      if (kind === 'bot' || kind === 'room') open({ kind, id: rest.join(':') }, mode);
    });
  }

  function openMenu() {
    if (menuView) {
      closeMenu(true);
      return;
    }
    if (PANE) return;
    if (pane) {
      const items = [];
      if (!narrow()) {
        items.push({
          id: 'flip',
          label: orient === 'row' ? tr('Друг под другом') : tr('Рядом'),
          icon: orient === 'row' ? ICON.col : ICON.row,
        });
      }
      items.push({ id: 'close', label: tr('Закрыть разделение'), icon: ICON.close, danger: true });
      mountMenu(items, tr('Разделённое окно'), (id) => {
        closeMenu(true);
        if (id === 'flip') {
          orient = orient === 'row' ? 'col' : 'row';
          paint();
          save();
        } else if (id === 'close') close();
      });
      return;
    }
    const items = [
      { id: 'row', label: tr('Рядом'), icon: ICON.row, hint: tr('слева и справа') },
      { id: 'col', label: tr('Друг под другом'), icon: ICON.col, hint: tr('сверху и снизу') },
    ];
    mountMenu(items, tr('Открыть второй чат'), (id) => {
      if (id === 'row' || id === 'col') pickList(id);
    });
  }

  // ---- перетаскивание из списка слева ----

  const mainRect = () => {
    const el = [$('#chat'), $('#team'), $('#catalog'), $('#workflows')].find((n) => n && !n.hidden);
    return el ? el.getBoundingClientRect() : null;
  };

  function zoneAt(x, y) {
    if (pane) {
      const r = box.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return { mode: 'replace', rect: r };
      return null;
    }
    const r = mainRect();
    if (!r || x < r.left || x > r.right || y < r.top || y > r.bottom) return null;
    const du = (x - r.left) / r.width - 0.5;
    const dv = (y - r.top) / r.height - 0.5;
    if (du <= 0.04 && dv <= 0.04) return null;
    if (!narrow() && du >= dv) return { mode: 'row', rect: new DOMRect(r.left + r.width / 2, r.top, r.width / 2, r.height) };
    if (dv > 0.04) return { mode: 'col', rect: new DOMRect(r.left, r.top + r.height / 2, r.width, r.height / 2) };
    return null;
  }

  function paintZone(hit) {
    if (!zone) return;
    if (!hit) {
      zoneOn = false;
      zone.classList.remove('on');
      return;
    }
    const r = hit.rect;
    // Первый показ встаёт на место сразу, дальше зона плавно переезжает.
    zone.classList.toggle('snap', !zoneOn);
    Object.assign(zone.style, { left: `${r.left + 4}px`, top: `${r.top + 4}px`, width: `${r.width - 8}px`, height: `${r.height - 8}px` });
    zoneText.textContent = hit.mode === 'row' ? tr('Открыть рядом') : hit.mode === 'col' ? tr('Открыть снизу') : tr('Показать здесь');
    zoneOn = true;
    zone.classList.add('on');
  }

  function endDrag() {
    dragView = null;
    body().classList.remove('split-dragging');
    paintZone(null);
  }

  function bindDrag() {
    $('#bot-list').addEventListener('dragstart', (e) => {
      const row = e.target.closest?.('.sb-row[data-id]');
      if (!row || !e.dataTransfer) return;
      dragView = { kind: row.dataset.kind === 'room' ? 'room' : 'bot', id: row.dataset.id };
      e.dataTransfer.setData(MIME, JSON.stringify(dragView));
      e.dataTransfer.effectAllowed = 'copyMove';
      // Рамки зон рисуем уже после старта: правка страницы внутри dragstart прерывает перетаскивание.
      requestAnimationFrame(() => {
        if (dragView) {
          ensureDom();
          body().classList.add('split-dragging');
        }
      });
    });
    document.addEventListener('dragover', (e) => {
      if (!dragView) return;
      const hit = zoneAt(e.clientX, e.clientY);
      paintZone(hit);
      if (!hit) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
    });
    document.addEventListener('drop', (e) => {
      if (!dragView) return;
      const hit = zoneAt(e.clientX, e.clientY);
      const view = dragView;
      endDrag();
      if (!hit) return;
      e.preventDefault();
      open(view, hit.mode === 'replace' ? undefined : hit.mode);
    });
    document.addEventListener('dragend', endDrag);
    // Указатель снова шлёт события — значит, перетаскивание кончилось, а dragend потерялся вместе со строкой.
    document.addEventListener('pointermove', () => { if (dragView) endDrag(); });
  }

  // Подсветка строк в списке и перетаскиваемость.
  function decorate() {
    if (PANE) return;
    for (const row of $$('#bot-list .sb-row[data-id]')) {
      const kind = row.querySelector('.room-icon') ? 'room' : 'bot';
      row.draggable = true;
      row.dataset.kind = kind;
      row.classList.toggle('sb-pane', Boolean(pane && pane.kind === kind && pane.id === row.dataset.id));
    }
  }

  // Бот из списка открывается слева. Если он уже стоит во второй панели, она берёт то, что было слева.
  function beforeSelect(kind, id) {
    if (PANE || !pane || !same(pane, { kind, id })) return;
    const prev = mainView();
    if (prev && !same(prev, { kind, id }) && exists(prev)) {
      pane = prev;
      setFrame();
      save();
      paint();
    } else close();
  }

  function sync() {
    if (PANE) return;
    if (pane && !exists(pane)) close();
  }

  function onMessage(e) {
    if (e.origin !== location.origin) return;
    const d = e.data;
    if (!d || typeof d !== 'object' || typeof d.deloist !== 'string') return;
    if (PANE) {
      if (e.source !== parent) return;
      if (d.deloist === 'pane-open') openPaneView({ kind: d.kind, id: String(d.id || '') });
      return;
    }
    if (!frame || e.source !== frame.contentWindow) return;
    if (d.deloist === 'pane-close') close();
    else if (d.deloist === 'pane-settings') openSettings(String(d.section || 'model'));
    else if (d.deloist === 'pane-ready') {
      frame.dataset.ready = '1';
      // Родитель успел сменить вид, пока страница панели грузилась.
      if (pane && !same(pane, { kind: d.kind, id: d.id })) {
        frameView = null;
        setFrame();
      }
    } else if (d.deloist === 'pane-view' && pane && (d.kind === 'bot' || d.kind === 'room')) {
      // Внутри панели открыли другого бота (например, гостя): запоминаем, что там теперь.
      pane = { kind: d.kind, id: String(d.id || '') };
      frameView = { ...pane };
      save();
      decorate();
    }
  }

  function init() {
    addEventListener('message', onMessage);
    if (PANE) {
      const closeBtn = $('#btn-pane-close');
      closeBtn.title = tr('Закрыть панель');
      closeBtn.setAttribute('aria-label', tr('Закрыть панель'));
      closeBtn.addEventListener('click', () => parent.postMessage({ deloist: 'pane-close' }, location.origin));
      const now = () => (inRoom() ? { kind: 'room', id: S.view.id } : { kind: 'bot', id: S.currentId || '' });
      window.addEventListener('deloist:bot', () => parent.postMessage({ deloist: 'pane-view', ...now() }, location.origin));
      parent.postMessage({ deloist: 'pane-ready', ...now() }, location.origin);
      return;
    }
    const btn = $('#btn-split');
    btn.addEventListener('click', openMenu);
    bindDrag();
    addEventListener('resize', paint);
    const saved = load();
    paint();
    if (saved && exists(saved)) open(saved);
    else if (saved) save();
  }

  return { init, sync, decorate, beforeSelect, open, close };
})();

async function boot() {
  // Без входа в аккаунт программа дальше экрана входа не идёт.
  await DeloistSignin.ensure({ api });
  bindUi();
  bindAccount();
  // Без аккаунта Deloist (публичная сборка) кнопок аккаунта и обращений к серверу Deloist нет.
  if (window.DeloistAccount && !window.DeloistAccount.required) for (const id of ["btn-account", "btn-feedback"]) document.getElementById(id)?.style.setProperty("display", "none");
  try {
    const data = await api('/api/state');
    S.bots = data.bots;
    S.mentionPicker?.update?.({ bots: S.bots });
    S.rooms = data.rooms || [];
    S.skills = data.skills || [];
    S.mcp = data.mcp || [];
    S.routines = data.routines;
    S.goals = data.goals || [];
    S.checkins = data.checkins || [];
    S.settings = data.settings;
    S.platform = data.platform || S.platform || {};
    S.levels = data.levels || S.levels || {};
    S.dataDir = data.dataDir;
    paintSecretsBanner(data.secretsUnavailable === true);
    for (const bot of S.bots) if (bot.busy) S.busy.add(bot.id);
    syncRooms();
    applyFont(S.settings.uiFont);
    // Язык выбран на сервере, а браузер помнит другой: выравниваем и перечитываем страницу один раз.
    if (S.settings.uiLang && S.settings.uiLang !== window.LANG && rememberLang(S.settings.uiLang)) {
      location.reload();
      return;
    }
    // Первый запуск: запоминаем язык системы, сервер заодно переводит стартовых ботов.
    if (!S.settings.uiLang) {
      await saveLook({ uiLang: window.LANG });
      await refreshState();
    }
    // Тема выбрана раньше, чем её стали запоминать в настройках: доку она нужна оттуда.
    let theme = '';
    try {
      theme = localStorage.getItem('crew-theme') || '';
    } catch {}
    if (theme !== (S.settings.uiTheme || '') && !PANE) saveLook({ uiTheme: theme });
    markLook();
    if (PANE) openPaneView();
    else showTeam();
    // После живой перезагрузки тестовой версии возвращаемся в тот же чат.
    if (!PANE) try {
      const back = JSON.parse(sessionStorage.getItem('deloist-dev-view') || 'null');
      sessionStorage.removeItem('deloist-dev-view');
      if (back?.kind === 'bot' && botById(back.id)) selectBot(back.id);
    } catch {}
    // После первого запуска (chooseTeam) открываем сцену оркестратора, если ничего другого не выбрано.
    if (!PANE && teamOpen()) {
      const first = firstRunRoom();
      if (first) selectRoom(first.id);
    }
    syncWaiting();
    refreshNeeds();
    if (!PANE) noteAway();
    // Экраны первого запуска ждут этого, чтобы, например, сразу открыть настройки модели.
    window.dispatchEvent(new Event('deloist:ready'));
    // Окно открылось кликом по питомцу, пока было в трее: сразу в чат этого бота.
    if (!PANE) api('/api/open-request', { method: 'POST' })
      .then(({ botId }) => {
        if (botId === '+new') openModal('modal-new');
        else if (botId && botById(botId)) selectBot(botId);
      })
      .catch(() => {});
  } catch (err) {
    toast(tr('Сервер не отвечает: {error}', { error: err.message }), 8000);
  }
  connectEvents();
  if (!PANE) mountRemote();
  Split.init();
  // Остальное — дело самого окна: вторая панель не обновляет программу и не показывает «Что нового» второй раз.
  if (PANE) {
    // Выбору модели в шапке нужен список провайдеров.
    DeloistProviders.start({ api, toast, onChanged: providersChanged });
    reloadModelPickers();
    return;
  }
  DeloistUpdates.start({ api, toast, openWhatsNew });
  // «Что нового» показываем один раз на новую версию.
  (async () => {
    try {
      const upd = await api('/api/updates');
      const currentVersion = upd?.current || '';
      if (!currentVersion) return;
      const last = S.settings?.lastSeenVersion || '';
      if (last && last !== currentVersion) {
        const wn = await api('/api/whats-new?from=' + encodeURIComponent(last));
        openWhatsNew(wn);
      }
      if (last !== currentVersion) {
        const { settings } = await api('/api/settings', { method: 'PUT', body: { lastSeenVersion: currentVersion } });
        if (settings) S.settings = settings;
      }
    } catch {}
  })();
  DeloistFeedback.start({ api, toast });
  DeloistProviders.start({ api, toast, onChanged: providersChanged });
  reloadModelPickers();
}

boot();


