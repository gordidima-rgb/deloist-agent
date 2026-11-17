// Док ботов у края экрана: аватары в кольцах лимита, лоток (голос, настройки, «+»), мини-чат и карточки.
// Островок, лоток, меню дока и голос рисует Svelte-компонент Dock (ui/src/components/Dock.svelte, кит —
// docs/design/kit.md); этот файл ведёт окно: область мыши, место у края, перенос, чат, карточки, аватары.
// Окно ловит мышь только в прямоугольниках, которые мы ему присылаем (region), поэтому после любой
// перемены на экране область отправляется заново.
(() => {
  const MARGIN = 8; // отступ дока от верха и низа рабочей области
  const DRAG_START = 5; // столько пикселей нужно сдвинуть ручку, чтобы это стало перетаскиванием
  const PEEL = 64; // настолько оттянуть от стены, чтобы док отлип
  const TUCK_DELAY = 900; // док уходит за край, если им не пользуются столько миллисекунд
  const CARD_LIFE = 8000; // столько карточка готовой работы висит, пока на неё не смотрят
  const CARD_STEP = 100;
  const LOOK_GRACE = 600; // время перевести указатель с дока на карточки, не теряя таймер
  const CARDS_SHOWN = 3;
  const SHADOW = 18; // запас области под тень
  const CHAT_HEIGHT = 560;
  const THREAD_LIMIT = 60;

  // Значки статичной разметки dock.html (svg с data-icon) заполняются сразу, до первой отрисовки.
  DeloistUI.paintIcons();
  const CLOSE_ICON = DeloistUI.icon('close', { className: 'i' });
  const tr = (key, params) => window.tr(key, params);
  const $ = (id) => document.getElementById(id);
  const wrap = $('wrap');
  let grip = null; // ручку отдаёт компонент дока
  const label = $('label');
  const shelf = $('shelf');
  const strip = $('strip');
  const chat = $('chat');
  const chatLog = $('chat-log');
  const chatInput = $('chat-input');
  const chatSend = $('chat-send');
  const chatPop = $('chat-pop');
  const chatMenu = $('chat-menu');

  let settings = {};
  let bots = [];
  const avatars = new Map(); // слот → { avatar, botId }
  // Знак занятия и эмоции от событий хода (правила в avatar-states.js): все аватары бота в доке и в мини-чате.
  const avatarsOf = (id) => [...avatars.values()].filter((held) => held.botId === id).map((held) => held.avatar);
  const avatarStates = window.CrewAvatarStates?.createTracker(avatarsOf);
  const faces = new Map(); // id бота → ячейка лица в кольце (её создаёт компонент дока)
  // Голос: состояние приходит событием voice_state (горячие клавиши) или из поддельного источника в отладке.
  let voice = { state: 'idle', level: 0, progress: 0, botId: '', error: '' };
  let voiceTimer = 0;
  let lastTalked = ''; // последний, с кем говорили: ему уходит голос, если бот не выбран
  let fakeVoice = false;
  const busy = new Set();
  const waiting = new Map(); // id бота → множество id подтверждений и вопросов
  const queued = new Map();
  const unread = new Set();
  const lastAt = new Map(); // время последнего ответа бота: новая карточка только на новый ответ
  const finished = new Set(); // кто закончил ход после последнего чтения состояния
  let cards = []; // { botId, name, color, text, left, el }
  let mainActive = false;
  let hovering = false;
  let lastHover = 0;
  let press = null; // ручка зажата: { x, y, top, grabY, active, carried }
  let tucked = false;
  let tuckTimer = 0;
  let refreshTimer = 0;
  let regionFrame = 0;
  // Открытый мини-чат.
  let openId = null;
  let openedAt = 0;
  let live = null; // пишущийся ответ: { turnId, text, el }
  let closing = null;

  const safeColor = (color) => (/^#[0-9a-f]{6}$/i.test(color || '') ? color : getComputedStyle(document.documentElement).getPropertyValue('--k-text-3').trim());
  const initial = (who) => (who.name || '?').trim().charAt(0).toUpperCase() || '?';
  const right = () => settings.dockEdge !== 'left';
  const autohide = () => Boolean(settings.dockAutohide);
  const botById = (id) => bots.find((bot) => bot.id === id);
  const seatOf = (id) => wrap.querySelector(`[data-seat="${CSS.escape(id)}"]`);
  const escapeHtml = (text) =>
    String(text ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

  const ipc = (message) => {
    try {
      window.ipc?.postMessage(message);
    } catch {}
  };

  async function api(path, options = {}) {
    const res = await fetch(path, {
      method: options.method || 'GET',
      cache: 'no-store',
      headers: options.body === undefined ? undefined : { 'content-type': 'application/json' },
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
    return data;
  }

  const send = (method, path, body) => api(path, { method, body }).catch(() => {});

  // --- Область окна -------------------------------------------------------

  function rectOf(el, pad) {
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    return [r.left - pad, r.top - pad, r.width + pad * 2, r.height + pad * 2];
  }

  function sendRegion() {
    regionFrame = 0;
    const rects = [];
    if (tucked) {
      if (autohide()) rects.push(rectOf(strip, 0));
    } else {
      // Островок целиком, лоток — только видимая часть: спрятанный лоток не должен ловить мышь под доком.
      const edge = wrap.querySelector('.k-edge');
      rects.push(edge ? rectOf(edge, SHADOW) : rectOf(wrap, SHADOW));
      const tray = wrap.querySelector('.k-tray-clip');
      if (tray) rects.push(trayOut() ? rectOf(tray, SHADOW) : grip && rectOf(grip, 6));
      for (const el of wrap.querySelectorAll('[data-region]')) rects.push(rectOf(el, SHADOW));
      for (const pill of pills.values()) if (pill.host.style.display !== 'none') rects.push(rectOf(pill.host.querySelector('.pill'), SHADOW));
      if (label.classList.contains('on')) rects.push(rectOf(label, 4));
    }
    // Чат берём по раскладке, а не по экрану: в начале анимации он крошечный, а область нужна целиком.
    if (!chat.hidden) rects.push([chat.offsetLeft - SHADOW, chat.offsetTop - SHADOW, chat.offsetWidth + SHADOW * 2, chat.offsetHeight + SHADOW * 2]);
    else for (const el of shelf.children) rects.push(rectOf(el, SHADOW));
    const text = rects
      .filter(Boolean)
      .map((r) => r.map((n) => Math.round(n * 10) / 10).join(','))
      .join(';');
    ipc(`region:${text}`);
  }

  function regionSoon() {
    if (!regionFrame) regionFrame = requestAnimationFrame(sendRegion);
  }

  // Лоток снаружи: указатель на доке, фокус внутри, открыто меню или идёт голос.
  const trayOut = () => hovering || wrap.matches(':focus-within') || Boolean(wrap.querySelector('.k-gear[aria-expanded="true"]')) || voice.state !== 'idle';
  // Лоток уезжает через 380 мс и едет 260 мс: область сжимаем, когда он уже спрятался.
  let trayTimer = 0;
  function trayRegionLater() {
    clearTimeout(trayTimer);
    trayTimer = setTimeout(regionSoon, 700);
  }

  // --- Место вдоль края -------------------------------------------------------

  const room = () => Math.max(0, innerHeight - MARGIN * 2 - wrap.offsetHeight);

  function placeDock(top) {
    const clamped = Math.min(Math.max(top, MARGIN), MARGIN + room());
    wrap.style.top = `${clamped}px`;
    placeShelf();
    return clamped;
  }

  function placeFromOffset() {
    const offset = Number.isFinite(settings.dockOffset) ? settings.dockOffset : 0.4;
    placeDock(MARGIN + room() * offset);
  }

  // Карточки начинаются вровень с доком, но не вылезают за низ экрана.
  function placeShelf() {
    const top = wrap.offsetTop + 18;
    shelf.style.top = `${Math.max(MARGIN, Math.min(top, innerHeight - MARGIN - shelf.offsetHeight))}px`;
  }

  function saveOffset() {
    const offset = room() ? (wrap.offsetTop - MARGIN) / room() : 0.5;
    settings.dockOffset = offset;
    return Math.round(offset * 1000) / 1000;
  }

  // --- Уход за край -------------------------------------------------------

  const inUse = () => hovering || Boolean(press) || cards.length > 0 || Boolean(openId);

  function setTucked(next) {
    if (tucked === next) return;
    tucked = next;
    wrap.classList.toggle('tucked', next);
    document.body.classList.toggle('tucked', next);
    hideLabel();
    // Уходящий док держит свою область до конца движения, иначе его обрежет на полпути.
    if (!next) sendRegion();
  }

  function scheduleTuck() {
    clearTimeout(tuckTimer);
    if (!autohide() || inUse()) {
      setTucked(false);
      return;
    }
    tuckTimer = setTimeout(() => {
      if (autohide() && !inUse()) setTucked(true);
    }, TUCK_DELAY);
  }

  wrap.addEventListener('transitionend', (event) => {
    if (event.target === wrap && event.propertyName === 'transform') sendRegion();
  });

  // Указатель в окне — значит, на доке, чате, карточках или полоске у края: окно ловит его только там.
  document.addEventListener('pointerover', () => {
    if (!hovering) regionSoon();
    hovering = true;
    lastHover = Date.now();
    scheduleTuck();
  });
  document.documentElement.addEventListener('pointerleave', () => {
    hovering = false;
    lastHover = Date.now();
    hideLabel();
    scheduleTuck();
    trayRegionLater();
  });
  strip.addEventListener('pointerenter', () => setTucked(false));

  // --- Подпись при наведении -------------------------------------------------------

  function noteOf(bot) {
    if (waiting.get(bot.id)?.size) return tr('Ждёт тебя');
    if (busy.has(bot.id)) return tr('Работает');
    const count = queued.get(bot.id) || 0;
    if (count) return tr('В очереди: {count}', { count });
    return bot.role || '';
  }

  function showLabel(seat) {
    if (press || tucked || openId) return;
    const bot = botById(seat.dataset.seat);
    if (!bot) return;
    label.querySelector('.label-name').textContent = bot.name;
    label.querySelector('.label-note').textContent = noteOf(bot);
    const limit = label.querySelector('.label-limit');
    limit.textContent = limitText(bot.limit);
    limit.classList.toggle('is-low', Boolean(bot.limit) && bot.limit.left < 0.15);
    const r = seat.getBoundingClientRect();
    label.style.top = `${Math.max(4, r.top + r.height / 2 - label.offsetHeight / 2)}px`;
    label.classList.add('on');
    regionSoon();
  }

  // Сколько осталось и какой предел сработал (правило — src/limit.rs). Число только здесь, под кольцом его нет.
  // «Дневной предел» приходит, только когда на нём остановилась рутина: живой разговор он не держит.
  function limitText(limit) {
    if (!limit) return '';
    const percent = Math.round(Math.min(1, Math.max(0, limit.left)) * 100);
    if (limit.source === 'wallet') {
      const rub = Number.isFinite(limit.moneyLeftKop) ? (limit.moneyLeftKop / 100).toLocaleString(window.LOCALE, { style: 'currency', currency: 'RUB' }) : '';
      return rub ? tr('Осталось {percent} %: {money} · кошелёк Deloist', { percent, money: rub }) : tr('Осталось {percent} % · кошелёк Deloist', { percent });
    }
    if (limit.source === 'plan') {
      const reset = DeloistUI.resetLine(limit.resetsAt);
      const left = tr('Осталось {percent} % подписки агента', { percent });
      return reset ? `${left} · ${reset}` : left;
    }
    if (limit.source === 'month') return tr('Осталось {percent} % месячного лимита бота · сброс 1-го числа', { percent });
    return tr('Дневной предел токенов исчерпан: рутины остановлены');
  }

  function hideLabel() {
    if (!label.classList.contains('on')) return;
    label.classList.remove('on');
    regionSoon();
  }

  // --- Аватары -------------------------------------------------------

  const stateOf = (id) => (waiting.get(id)?.size ? 'wait' : busy.has(id) ? 'work' : 'idle');

  function lookOf(bot) {
    return {
      color: safeColor(bot.color),
      skin: bot.skin ?? 0,
      name: bot.name || '',
    };
  }

  function paintFace(face, bot, slot, size) {
    const look = lookOf(bot);
    let held = avatars.get(slot);
    // Ячейку заняли другим ботом: аватар проще собрать заново.
    if (held && held.botId !== bot.id) {
      held.avatar.destroy();
      avatars.delete(slot);
      held = null;
    }
    if (!held) {
      face.textContent = '';
      // В доке аватар в покое стоит неподвижно (как маленькие аватары в окне программы) и оживает при наведении
      // и смене состояния: восемь дышащих шаров держали GPU-процесс на ~20 % ядра. Работу показывает плашка шагов.
      const avatar = window.CrewAvatar ? CrewAvatar.mount(face, { size, ...look, still: slot.startsWith('seat:') }) : null;
      if (avatar) {
        held = { avatar, botId: bot.id };
        avatars.set(slot, held);
      }
    } else if (held.avatar.canvas && held.avatar.canvas.parentElement !== face) {
      face.textContent = '';
      held.avatar.attach(face);
    }
    if (held) {
      held.avatar.set({ ...look, state: stateOf(bot.id), quiet: true });
      face.classList.add('live');
      face.style.background = '';
    } else {
      face.textContent = initial(bot);
      face.style.background = safeColor(bot.color);
    }
  }

  const docked = () => bots.filter((bot) => bot.dock !== false);

  function seatProps(bot) {
    return {
      id: bot.id,
      name: bot.name,
      color: safeColor(bot.color),
      limit: bot.limit || null,
      queued: queued.get(bot.id) || 0,
      failed: Boolean(bot.preview?.error) && !busy.has(bot.id),
      working: busy.has(bot.id),
      needs: Boolean(waiting.get(bot.id)?.size),
      unread: unread.has(bot.id),
    };
  }

  function voiceTargetId() {
    const chosen = settings.voiceTarget;
    if (chosen && botById(chosen)) return chosen;
    return lastTalked && botById(lastTalked) ? lastTalked : openId || docked()[0]?.id || '';
  }
  window.dockVoiceTarget = voiceTargetId;

  function dockProps() {
    return {
      bots: docked().map(seatProps),
      side: right() ? 'right' : 'left',
      voice: { ...voice },
      voiceAvailable: Boolean(window.DeloistVoice?.toggle),
      voiceTargetId: voiceTargetId(),
      settings: { autohide: autohide(), layer: settings.dockLayer || 'top', voiceTarget: settings.voiceTarget || '' },
      hiddenBots: bots.filter((bot) => bot.dock === false).map((bot) => ({ id: bot.id, name: bot.name, color: safeColor(bot.color) })),
    };
  }

  function paint() {
    const list = docked();
    const wanted = new Set(list.map((bot) => `seat:${bot.id}`));
    for (const [slot, held] of avatars) {
      if (slot.startsWith('seat:') && !wanted.has(slot)) {
        held.avatar.destroy();
        avatars.delete(slot);
      }
    }
    dock.update(dockProps());
    // Новые ячейки лица появляются при отрисовке компонента и сами зовут onface; старые красим заново.
    for (const bot of list) {
      const face = faces.get(bot.id);
      if (face?.isConnected) paintFace(face, bot, `seat:${bot.id}`, 44);
    }
    // Бота убрали из дока, пока его чат был открыт.
    if (openId && !list.some((bot) => bot.id === openId)) closeChat(true);
    for (const id of [...pills.keys()]) {
      if (!activityOn() || !list.some((bot) => bot.id === id)) endSteps(id);
    }
    if (!press) placeFromOffset();
    if (openId) placeChat();
    for (const id of pills.keys()) placePill(id);
    regionSoon();
  }

  function repaintBot(id) {
    const bot = botById(id);
    const face = faces.get(id);
    if (bot && face?.isConnected) {
      paintFace(face, bot, `seat:${id}`, 44);
      dock.update({ bots: docked().map(seatProps) });
    }
    if (bot && id === openId) {
      paintFace($('chat-face'), bot, 'chat', 32);
      $('chat-state').textContent = busy.has(id) || waiting.get(id)?.size ? noteOf(bot) : '';
    }
  }

  // --- Мини-чат -------------------------------------------------------

  // Ссылка на файл в ответе: настоящий href только для «Копировать ссылку», переходить по file: окно не должно.
  for (const type of ['click', 'auxclick']) {
    chatLog.addEventListener(type, (e) => {
      if (e.target.closest?.('a.md-file')) e.preventDefault();
    });
  }

  // Одна разметка с главным чатом: ui/src/lib/markdown.js (таблицы, списки, ссылки со значком сервиса).
  function md(text) {
    return window.DeloistUI.markdown(text);
  }

  const nearBottom = () => chatLog.scrollHeight - chatLog.scrollTop - chatLog.clientHeight < 60;
  const toBottom = () => {
    chatLog.scrollTop = chatLog.scrollHeight;
  };

  function messageNode(msg) {
    const el = document.createElement('div');
    if (msg.role === 'user') {
      el.className = 'm-user';
      el.textContent = msg.text;
    } else {
      el.className = `m-bot${msg.error ? ' error' : ''}`;
      if (msg.error || !msg.text) {
        el.innerHTML = md(msg.text);
      } else {
        // Готовый ответ: текст отдельно, под ним значки «копировать», «озвучить» и время.
        const body = document.createElement('div');
        body.className = 'm-body';
        body.innerHTML = md(msg.text);
        el.append(body, answerBar(msg, body));
      }
    }
    return el;
  }

  const ACT_ICONS = { copy: 'copy', check: 'check', speak: 'speak', stop: 'stop-small' };
  const setActIcon = (btn, kind) => {
    btn.innerHTML = DeloistUI.icon(ACT_ICONS[kind], { className: 'i' });
  };
  function actBtn(kind, label) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'm-act';
    btn.title = label;
    btn.setAttribute('aria-label', label);
    setActIcon(btn, kind);
    return btn;
  }
  function setActLabel(btn, label) {
    btn.title = label;
    btn.setAttribute('aria-label', label);
  }

  // «1 час назад»; старше недели — дата.
  function agoText(ts) {
    const n = Number(ts);
    if (!Number.isFinite(n) || n <= 0) return '';
    const sec = Math.round((Date.now() - n) / 1000);
    if (sec < 45) return tr('только что');
    if (sec >= 7 * 86400) return new Date(n).toLocaleDateString(window.LOCALE, { day: 'numeric', month: 'short' });
    const fmt = new Intl.RelativeTimeFormat(window.LOCALE, { numeric: 'always' });
    if (sec < 3600) return fmt.format(-Math.max(1, Math.round(sec / 60)), 'minute');
    if (sec < 86400) return fmt.format(-Math.round(sec / 3600), 'hour');
    return fmt.format(-Math.round(sec / 86400), 'day');
  }

  async function copyAnswer(text) {
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

  let speakingBtn = null;
  function stopAnswerSpeech() {
    if (!speakingBtn) return;
    const btn = speakingBtn;
    speakingBtn = null;
    try { speechSynthesis.cancel(); } catch {}
    btn.classList.remove('on');
    setActLabel(btn, tr('Озвучить'));
    setActIcon(btn, 'speak');
  }
  function speakAnswer(btn, text) {
    const wasThis = speakingBtn === btn;
    stopAnswerSpeech();
    if (wasThis || !text.trim()) return;
    const utter = new SpeechSynthesisUtterance(text);
    const want = window.LANG === 'en' ? 'en' : 'ru';
    let name = '';
    try { name = localStorage.getItem('vc.voice') || ''; } catch {}
    const voices = speechSynthesis.getVoices() || [];
    const voice = voices.find((v) => v.name === name) || voices.find((v) => (v.lang || '').toLowerCase().startsWith(want));
    if (voice) utter.voice = voice;
    speakingBtn = btn;
    btn.classList.add('on');
    setActLabel(btn, tr('Остановить'));
    setActIcon(btn, 'stop');
    const end = () => { if (speakingBtn === btn) stopAnswerSpeech(); };
    utter.onend = end;
    utter.onerror = end;
    try { speechSynthesis.speak(utter); } catch { end(); }
  }

  function answerBar(msg, body) {
    const bar = document.createElement('div');
    bar.className = 'm-actions';
    const copy = actBtn('copy', tr('Копировать'));
    copy.addEventListener('click', async () => {
      const ok = await copyAnswer(body.innerText.trim());
      setActLabel(copy, ok ? tr('Скопировано') : tr('Не получилось скопировать'));
      if (ok) setActIcon(copy, 'check');
      copy.classList.add('on');
      clearTimeout(copy._t);
      copy._t = setTimeout(() => {
        copy.classList.remove('on');
        setActLabel(copy, tr('Копировать'));
        setActIcon(copy, 'copy');
      }, 1600);
    });
    bar.append(copy);
    if ('speechSynthesis' in window) {
      const speak = actBtn('speak', tr('Озвучить'));
      speak.addEventListener('click', () => speakAnswer(speak, body.innerText));
      bar.append(speak);
    }
    const when = agoText(msg.at);
    if (when) {
      const time = document.createElement('span');
      time.className = 'm-when';
      time.textContent = when;
      bar.append(time);
    }
    return bar;
  }

  // У последнего ответа значки видны всегда, у остальных — при наведении.
  function markLastAnswer() {
    chatLog.querySelector('.m-bot.last-answer')?.classList.remove('last-answer');
    const all = chatLog.querySelectorAll('.m-bot:has(.m-actions)');
    all[all.length - 1]?.classList.add('last-answer');
  }

  function removeTyping() {
    chatLog.querySelector('.typing')?.remove();
  }

  function showTyping() {
    if (chatLog.querySelector('.typing') || live) return;
    const dots = document.createElement('div');
    dots.className = 'typing';
    dots.setAttribute('aria-label', tr('Работает'));
    dots.innerHTML = `<span class="k-pix k-pix--paper k-pix--think" role="img">${'<i></i>'.repeat(9)}</span>`;
    chatLog.append(dots);
  }

  function showWaitingNote() {
    chatLog.querySelector('.m-note')?.remove();
    if (!waiting.get(openId)?.size) return;
    const note = document.createElement('div');
    note.className = 'm-note';
    const text = document.createElement('span');
    text.textContent = tr('Бот ждёт твоего решения');
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'k-btn k-btn--sm';
    button.textContent = tr('Открыть в Deloist');
    button.addEventListener('click', () => openFull(openId));
    note.append(text, button);
    chatLog.append(note);
    toBottom();
  }

  async function loadThread(id) {
    chatLog.textContent = '';
    live = null;
    try {
      const { messages } = await api(`/api/bots/${encodeURIComponent(id)}/thread?limit=${THREAD_LIMIT}`);
      if (openId !== id) return;
      const shown = (messages || []).filter((msg) => msg.text && (msg.role === 'user' || msg.role === 'assistant'));
      chatLog.textContent = '';
      if (!shown.length) {
        const empty = document.createElement('div');
        empty.className = 'chat-empty';
        empty.textContent = tr('Напиши, что сделать: бот ответит здесь');
        chatLog.append(empty);
      }
      for (const msg of shown) chatLog.append(messageNode(msg));
      markLastAnswer();
      if (busy.has(id)) showTyping();
      showWaitingNote();
      toBottom();
    } catch {
      // Сервер мог перезапускаться: переписка подтянется при следующем открытии.
    }
  }

  // Чат стоит рядом с аватаром: по высоте вокруг него, но целиком на экране.
  function placeChat() {
    const height = Math.min(CHAT_HEIGHT, innerHeight - MARGIN * 2);
    chat.style.height = `${height}px`;
    const seat = seatOf(openId);
    const r = seat ? seat.getBoundingClientRect() : wrap.getBoundingClientRect();
    const center = r.top + r.height / 2;
    const top = Math.max(MARGIN, Math.min(center - height * 0.6, innerHeight - MARGIN - height));
    chat.style.top = `${top}px`;
  }

  // Откуда расти и куда сворачиваться: середина аватара в координатах чата.
  function seatOrigin(id) {
    const seat = seatOf(id);
    const c = chat.getBoundingClientRect();
    if (!seat) return { x: right() ? c.width : 0, y: c.height / 2 };
    const s = seat.getBoundingClientRect();
    return { x: s.left + s.width / 2 - c.left, y: s.top + s.height / 2 - c.top };
  }

  function toggleChat(id) {
    if (openId === id && !closing) closeChat();
    else openChat(id);
  }

  function openChat(id) {
    const bot = botById(id);
    if (!bot) return;
    lastTalked = id;
    closing?.cancel();
    closing = null;
    const switching = Boolean(openId) && !chat.hidden;
    openId = id;
    openedAt = Date.now();
    unread.delete(id);
    cards = cards.filter((card) => card.botId !== id);
    renderCards();
    hideLabel();
    closeChatMenu();
    document.body.classList.add('chatting');
    $('chat-name').textContent = bot.name;
    chatInput.placeholder = tr('Сообщение: {name}', { name: bot.name });
    repaintBot(id);
    chat.hidden = false;
    placeChat();
    for (const pillId of pills.keys()) placePill(pillId);
    loadThread(id);
    if (!switching && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
      const o = seatOrigin(id);
      chat.style.transformOrigin = `${o.x}px ${o.y}px`;
      chat.animate(
        [
          { transform: 'scale(0.08)', opacity: 0, borderRadius: '50%' },
          { transform: 'scale(1.015)', opacity: 1, borderRadius: '24px', offset: 0.7 },
          { transform: 'scale(1)', opacity: 1, borderRadius: '24px' },
        ],
        { duration: 380, easing: 'cubic-bezier(0.25, 0.9, 0.35, 1)' },
      );
      seatOf(id)?.animate([{ transform: 'scale(1)' }, { transform: 'scale(0.86)' }, { transform: 'scale(1)' }], { duration: 300, easing: 'ease-out' });
    }
    // Поле ввода ждёт текста: окно берёт фокус, пока чат открыт.
    ipc('focus:1');
    setTimeout(() => chatInput.focus(), 60);
    sendRegion();
    scheduleTuck();
  }

  // Сворачивается обратно в аватар бота, и аватар чуть пружинит, «проглотив» окно.
  function closeChat(instant = false) {
    if (!openId || closing) return;
    const id = openId;
    closeChatMenu();
    const done = () => {
      closing = null;
      chat.hidden = true;
      chat.style.transform = '';
      openId = null;
      live = null;
      document.body.classList.remove('chatting');
      for (const pillId of pills.keys()) placePill(pillId);
      const held = avatars.get('chat');
      if (held) {
        held.avatar.destroy();
        avatars.delete('chat');
      }
      ipc('focus:0');
      sendRegion();
      scheduleTuck();
    };
    if (instant || chat.hidden || matchMedia('(prefers-reduced-motion: reduce)').matches) {
      done();
      return;
    }
    const o = seatOrigin(id);
    chat.style.transformOrigin = `${o.x}px ${o.y}px`;
    closing = chat.animate(
      [
        { transform: 'scale(1)', opacity: 1, borderRadius: '24px' },
        { transform: 'scale(0.08)', opacity: 0, borderRadius: '50%' },
      ],
      { duration: 240, easing: 'cubic-bezier(0.55, 0, 0.75, 0.2)', fill: 'forwards' },
    );
    closing.onfinish = () => {
      closing?.cancel();
      done();
      seatOf(id)?.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.14)' }, { transform: 'scale(1)' }], { duration: 320, easing: 'cubic-bezier(0.3, 1.6, 0.5, 1)' });
    };
  }

  function openFull(id) {
    closeChat(true);
    send('POST', `/api/bots/${encodeURIComponent(id)}/open`, {});
    ipc('open');
  }

  async function sendMessage() {
    const text = chatInput.value.trim();
    // Снимок экрана с горячей клавиши (voice.js) едет вложением.
    const attachments = window.dockBeforeSend?.() ?? [];
    if ((!text && !attachments.length) || !openId) return;
    const id = openId;
    chatInput.value = '';
    fitInput();
    chatLog.querySelector('.chat-empty')?.remove();
    const mine = messageNode({ role: 'user', text });
    mine.classList.add('pending');
    mine.dataset.pending = text;
    chatLog.append(mine);
    toBottom();
    try {
      await api(`/api/bots/${encodeURIComponent(id)}/messages`, { method: 'POST', body: attachments.length ? { text, attachments } : { text } });
    } catch (err) {
      mine.remove();
      window.dockAfterFail?.(id, attachments);
      if (openId === id) {
        chatInput.value = text;
        fitInput();
        const note = messageNode({ role: 'assistant', text: err.message, error: true });
        chatLog.append(note);
        toBottom();
      }
    }
  }

  function fitInput() {
    chatInput.style.height = 'auto';
    chatInput.style.height = `${Math.min(chatInput.scrollHeight, 120)}px`;
    chatSend.disabled = !chatInput.value.trim();
  }

  $('chat-form').addEventListener('submit', (event) => {
    event.preventDefault();
    sendMessage();
  });
  chatInput.addEventListener('input', fitInput);
  chatInput.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      sendMessage();
    }
  });
  $('chat-close').addEventListener('click', () => closeChat());
  // --- Меню «…» в шапке мини-чата (Svelte Menu из кита) ----------------------------
  const ICON = { main: 'window-bar', window: 'window-out', speak: 'speak-alt', mic: 'mic', free: 'shield-free', compact: 'compact-lines', clear: 'trash-can', undock: 'eye-off' };
  let chatMenuView = null;
  let confirmClear = false;

  function chatMenuItems() {
    const bot = botById(openId);
    if (!bot) return [];
    if (confirmClear) {
      return [
        { kind: 'note', label: tr('Стереть всю переписку с {name}? Вернуть её не получится.', { name: bot.name }) },
        { id: 'clear-yes', label: tr('Да, очистить'), icon: ICON.clear, danger: true },
        { id: 'clear-no', label: tr('Отмена'), autofocus: true },
      ];
    }
    const target = settings.voiceTarget === bot.id;
    return [
      { id: 'main', label: tr('Открыть в главном окне'), icon: ICON.main },
      // Отдельное окно чата требует нового нативного окна (dock.rs): делает другая ветка.
      { id: 'window', label: tr('Открыть в отдельном окне'), icon: ICON.window, disabled: true, hint: tr('скоро'), tip: tr('Отдельное окно чата появится в следующей версии') },
      { kind: 'sep' },
      { id: 'speak', kind: 'check', label: tr('Озвучивать ответы'), icon: ICON.speak, checked: Boolean(bot.readAloud) },
      { id: 'target', kind: 'check', label: tr('Голос уходит сюда'), icon: ICON.mic, checked: target },
      { id: 'free', kind: 'check', label: tr('Свобода действий'), icon: ICON.free, checked: bot.approval === 'auto' },
      { kind: 'sep' },
      { id: 'compact', label: tr('Сжать историю'), icon: ICON.compact },
      { id: 'clear', label: tr('Очистить чат'), icon: ICON.clear },
      { kind: 'sep' },
      { id: 'undock', label: tr('Убрать из дока'), icon: ICON.undock, danger: true },
    ];
  }

  function openChatMenu() {
    if (chatMenuView) {
      closeChatMenu(true);
      return;
    }
    confirmClear = false;
    chatPop.hidden = false;
    chatMenu.setAttribute('aria-expanded', 'true');
    chatMenuView = window.DeloistUI.mountMenu(chatPop, {
      items: chatMenuItems(),
      label: tr('Ещё'),
      onpick: pickChatMenu,
      onclose: (refocus) => closeChatMenu(refocus),
    });
  }

  function closeChatMenu(refocus = false) {
    if (!chatMenuView) return;
    chatMenuView.destroy();
    chatMenuView = null;
    confirmClear = false;
    chatPop.hidden = true;
    chatMenu.setAttribute('aria-expanded', 'false');
    if (refocus) chatMenu.focus({ preventScroll: true });
  }

  function repaintChatMenu() {
    chatMenuView?.update({ items: chatMenuItems() });
  }

  async function pickChatMenu(act) {
    const id = openId;
    const bot = botById(id);
    if (!bot) return;
    const path = `/api/bots/${encodeURIComponent(id)}`;
    if (act === 'main') {
      closeChatMenu();
      openFull(id);
    } else if (act === 'speak') {
      bot.readAloud = !bot.readAloud;
      repaintChatMenu();
      send('PATCH', path, { readAloud: bot.readAloud }).then(refreshSoon);
    } else if (act === 'target') {
      const voiceTarget = settings.voiceTarget === id ? 'last' : id;
      settings.voiceTarget = voiceTarget === 'last' ? '' : id;
      repaintChatMenu();
      dock.update(dockProps());
      send('PUT', '/api/settings', { voiceTarget }).then(refreshSoon);
    } else if (act === 'free') {
      // Как переключатель под полем ввода в главном окне: «Спрашивать» включает и строгий режим из профиля.
      bot.approval = bot.approval === 'auto' ? 'ask' : 'auto';
      repaintChatMenu();
      send('PATCH', path, { approval: bot.approval }).then(refreshSoon);
    } else if (act === 'compact') {
      closeChatMenu(true);
      await send('POST', `${path}/compact`, {});
      if (openId === id) loadThread(id);
    } else if (act === 'clear') {
      confirmClear = true;
      repaintChatMenu();
    } else if (act === 'clear-no') {
      confirmClear = false;
      repaintChatMenu();
    } else if (act === 'clear-yes') {
      closeChatMenu(true);
      await send('POST', `${path}/clear`, {});
      if (openId === id) loadThread(id);
    } else if (act === 'undock') {
      closeChatMenu();
      closeChat();
      send('PATCH', path, { dock: false }).then(refreshSoon);
    }
  }

  chatMenu.addEventListener('click', openChatMenu);
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape' || !openId) return;
    if (chatMenuView) closeChatMenu(true);
    else closeChat();
  });
  // Человек ушёл в другую программу: чат сворачивается в бота, как всплывающее окошко.
  addEventListener('blur', () => {
    if (openId && Date.now() - openedAt > 400) closeChat();
  });

  // --- Чем занят бот -------------------------------------------------------

  const pills = new Map(); // id бота → { host, ui, now, was, seq }
  const PILL_MAX = 300;
  const fileOf = (path) => String(path || '').split(/[\\/]/).filter(Boolean).pop() || '';
  const hostOf = (url) => {
    try {
      return new URL(url).host;
    } catch {
      return '';
    }
  };
  const activityOn = () => settings.dockActivity !== false;

  // Агенты ACP зовут наш браузер через MCP: «mcp.deloist_browser.browser_page» → «browser_page».
  function plainTool(name) {
    return String(name || '').replace(/^mcp(?:\.|__)[a-z0-9_-]+?(?:\.|__)(?=(?:browser_|web_|delegate$|list_bots$|remember$|recall$))/i, '');
  }
  const looksLikeTool = (text) => /^mcp(?:\.|__)|^(?:(?:browser|web)_[a-z_]+|delegate|list_bots|remember|recall)$/i.test(String(text || '').trim());

  // Шаг словами: что делает сейчас и как это звучит, когда сделано.
  function describe(rawName, args = {}) {
    const name = plainTool(rawName);
    const file = fileOf(args.path);
    switch (name) {
      case 'browser_nav': {
        const host = hostOf(args.url);
        return host
          ? { now: tr('Открывает {host}', { host }), past: tr('Открыл {host}', { host }) }
          : { now: tr('Листает вкладки'), past: tr('Переключил вкладку') };
      }
      case 'browser_page':
        return { now: tr('Читает страницу'), past: tr('Прочитал страницу') };
      case 'browser_mouse':
        return { now: tr('Нажимает на странице'), past: tr('Нажал на странице') };
      case 'browser_keyboard':
        return { now: tr('Печатает на странице'), past: tr('Напечатал на странице') };
      case 'browser_wait':
        return { now: tr('Ждёт страницу'), past: tr('Дождался страницы'), kind: 'wait' };
      case 'browser_human':
      case 'browser_show_to_human':
        return { now: tr('Показывает тебе браузер'), past: tr('Показал тебе браузер'), kind: 'wait' };
      case 'web_search': {
        const query = String(args.query || '').slice(0, 40);
        return { now: tr('Ищет в интернете'), past: tr('Поискал в интернете'), meta: query };
      }
      case 'read_file':
        return { now: tr('Читает {file}', { file }), past: tr('Прочитал {file}', { file }) };
      case 'list_files':
        return file
          ? { now: tr('Смотрит папку {file}', { file }), past: tr('Посмотрел папку {file}', { file }) }
          : { now: tr('Смотрит свою папку'), past: tr('Посмотрел свою папку') };
      case 'write_file':
        return { now: tr('Пишет {file}', { file }), past: tr('Записал {file}', { file }) };
      case 'run_command': {
        const tool = String(args.command || '').trim().split(/\s+/)[0] || '';
        return { now: tr('Запускает команду'), past: tr('Выполнил команду'), meta: fileOf(tool) };
      }
      case 'web_fetch': {
        const host = hostOf(args.url);
        return { now: tr('Открывает {host}', { host }), past: tr('Прочитал {host}', { host }) };
      }
      case 'generate_image':
        return { now: tr('Рисует картинку'), past: tr('Нарисовал картинку') };
      case 'remember':
        return { now: tr('Запоминает'), past: tr('Запомнил') };
      case 'delegate':
        return { now: tr('Передаёт задачу: {bot}', { bot: args.bot || '' }), past: tr('Передал задачу: {bot}', { bot: args.bot || '' }) };
      case 'ask_user':
        return { now: tr('Спрашивает тебя'), past: tr('Спросил тебя') };
      default:
        if (String(name).startsWith('browser_')) return { now: tr('Работает в браузере'), past: tr('Сделал шаг в браузере') };
        if (String(name).startsWith('mcp__')) {
          const tool = String(name).split('__').pop();
          return { now: tr('Вызывает {tool}', { tool }), past: tr('Вызвал {tool}', { tool }) };
        }
        return { now: tr('Работает'), past: tr('Сделал шаг') };
    }
  }

  // Вид шага для девяти пикселей: думает, читает, действует, пишет, ждёт (docs/design/kit.md).
  function kindOf(step) {
    if (step.kind) return step.kind;
    if (step.think) return step.write ? 'write' : 'think';
    const name = String(step.name || '');
    if (/^(read_file|list_files|web_fetch|web_search|search|recall|memory_)/.test(name)) return 'read';
    return 'act';
  }

  const lineOf = (step, key) => ({ key, kind: kindOf(step), text: step.now, meta: step.meta || '' });
  const wasOf = (step, key) => ({ key, text: step.done || step.now, meta: step.doneMeta ?? step.meta ?? '' });

  function placePill(id) {
    const pill = pills.get(id);
    const seat = seatOf(id);
    if (!pill) return;
    if (!seat || openId === id) {
      pill.host.style.display = 'none';
      return;
    }
    pill.host.style.display = '';
    const s = seat.getBoundingClientRect();
    const w = wrap.getBoundingClientRect();
    pill.host.style.top = `${Math.round(s.top - w.top + s.height / 2 - 26)}px`;
  }

  function step(id, next) {
    if (!activityOn() || !botById(id) || botById(id).dock === false) return;
    let pill = pills.get(id);
    if (!pill) {
      const host = document.createElement('div');
      host.className = 'pill-host';
      host.dataset.bot = id;
      wrap.append(host);
      pill = { host, ui: null, now: null, was: null, seq: 0 };
      pills.set(id, pill);
    }
    // Тот же шаг ещё раз (например, ответ пишется дальше) — ничего не двигаем.
    if (pill.now && pill.now.step.now === next.now && !next.callId) return;
    if (pill.now) pill.was = { step: pill.now.step, key: pill.now.key };
    pill.now = { step: next, key: ++pill.seq };
    renderPill(id);
    placePill(id);
    regionSoon();
  }

  function renderPill(id) {
    const pill = pills.get(id);
    if (!pill) return;
    const props = {
      now: lineOf(pill.now.step, pill.now.key),
      was: pill.was ? wasOf(pill.was.step, pill.was.key) : null,
      side: right() ? 'right' : 'left',
      onresize: regionSoon,
    };
    if (pill.ui) pill.ui.update(props);
    else pill.ui = window.DeloistUI.mountStepPill(pill.host, props);
  }

  // Шаг закончился: запоминаем, как он прозвучит строкой «было».
  function stepDone(id, callId, status, result) {
    const pill = pills.get(id);
    const held = [pill?.now, pill?.was].find((item) => item?.step.callId === callId);
    if (!held) return;
    const done = held.step;
    if (status !== 'ok') {
      done.done = tr('Не вышло: {what}', { what: done.now });
      done.doneMeta = '';
    } else {
      done.done = done.past;
      if (done.name === 'read_file' && typeof result === 'string') done.doneMeta = tr('строк: {count}', { count: result.split('\n').length });
    }
    if (pill.was === held) renderPill(id);
  }

  // Ход окончен: плашка втягивается в док.
  function endSteps(id) {
    const pill = pills.get(id);
    if (!pill) return;
    pills.delete(id);
    pill.ui?.update({ leaving: true });
    setTimeout(() => {
      pill.ui?.destroy();
      pill.host.remove();
      regionSoon();
    }, 420);
  }

  function stepEvent(event) {
    const id = event.botId;
    switch (event.type) {
      case 'turn_start':
        step(id, { now: tr('Думает'), think: true });
        break;
      case 'tool_start': {
        const words = describe(event.name, event.args || {});
        step(id, { ...words, callId: event.callId, name: event.name });
        break;
      }
      case 'tool_update':
        stepDone(id, event.callId, event.status, event.result);
        break;
      case 'activity': {
        const title = String(event.call?.title || event.call?.name || '').trim();
        // Заголовок шага у агента бывает просто именем инструмента: показываем его словами.
        const words = looksLikeTool(title) ? describe(title, event.call?.rawInput || event.call?.args || {}) : { now: title, past: title };
        if (title) step(id, { ...words, callId: event.call?.id || title });
        break;
      }
      case 'text_delta':
        step(id, { now: tr('Пишет ответ'), past: tr('Ответил'), kind: 'write' });
        break;
      case 'approval':
      case 'question':
        step(id, { now: tr('Ждёт тебя'), past: tr('Дождался тебя'), kind: 'wait' });
        break;
      case 'turn_end':
        endSteps(id);
        break;
      default:
        break;
    }
  }

  // --- Готовая работа -------------------------------------------------------

  function renderCards() {
    shelf.textContent = '';
    const shown = cards.slice(-CARDS_SHOWN).reverse();
    for (const card of shown) {
      if (!card.el) card.el = makeCard(card);
      shelf.append(card.el);
    }
    const hidden = cards.length - shown.length;
    if (hidden > 0) {
      const more = document.createElement('button');
      more.type = 'button';
      more.className = 'more';
      more.textContent = tr('Ещё {count}', { count: hidden });
      more.addEventListener('click', () => ipc('open'));
      shelf.append(more);
    }
    placeShelf();
    regionSoon();
    scheduleTuck();
  }

  function makeCard(card) {
    const el = document.createElement('div');
    el.className = 'card';
    el.setAttribute('role', 'button');
    const head = document.createElement('div');
    head.className = 'card-head';
    const dot = document.createElement('span');
    dot.className = 'card-dot';
    dot.style.background = safeColor(card.color);
    const name = document.createElement('span');
    name.className = 'card-name';
    name.textContent = card.name;
    head.append(dot, name);
    const text = document.createElement('div');
    text.className = 'card-text';
    text.textContent = card.text;
    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'card-close';
    close.innerHTML = CLOSE_ICON;
    close.title = tr('Закрыть');
    close.setAttribute('aria-label', close.title);
    close.addEventListener('click', (event) => {
      event.stopPropagation();
      dropCard(card);
    });
    el.append(head, text, close);
    el.addEventListener('click', () => openChat(card.botId));
    return el;
  }

  function dropCard(card) {
    if (!cards.includes(card)) return;
    cards = cards.filter((item) => item !== card);
    if (card.el?.isConnected) {
      card.el.classList.add('leaving');
      setTimeout(renderCards, 300);
    } else {
      renderCards();
    }
  }

  function addCard(bot) {
    const text = String(bot.preview?.text || '').replace(/\s+/g, ' ').trim();
    if (!text) return;
    // Новый ответ того же бота заменяет старую карточку: важен последний.
    cards = cards.filter((item) => item.botId !== bot.id);
    cards.push({ botId: bot.id, name: bot.name, color: bot.color, text, left: CARD_LIFE });
    unread.add(bot.id);
    renderCards();
  }

  // Часы карточек стоят, пока на док или карточки смотрят, и ещё немного после.
  setInterval(() => {
    if (!cards.length) return;
    if (hovering || press || Date.now() - lastHover < LOOK_GRACE) return;
    for (const card of cards.slice(-CARDS_SHOWN)) {
      card.left -= CARD_STEP;
      if (card.left <= 0) dropCard(card);
    }
  }, CARD_STEP);

  // --- «+» -------------------------------------------------------

  function addBot() {
    closeChat(true);
    send('POST', '/api/dock/new-bot');
    ipc('open');
  }

  // --- Ручка: вдоль края сдвинуть, от стены отлепить и перенести ----------------------------

  function wireGrip(el) {
    if (grip === el) return;
    grip = el;
    grip.addEventListener('pointerdown', gripDown);
    grip.addEventListener('pointermove', gripMove);
    grip.addEventListener('pointerup', release);
    grip.addEventListener('pointercancel', release);
    // Отпускание, которое не дошло до ручки: Windows могла снять захват, а док не должен остаться «в руке».
    grip.addEventListener('lostpointercapture', release);
  }

  function gripDown(event) {
    if (event.button !== 0) return;
    const r = wrap.getBoundingClientRect();
    press = { x: event.clientX, y: event.clientY, top: wrap.offsetTop, grabX: event.clientX - r.left, grabY: event.clientY - wrap.offsetTop, active: false, carried: false };
    try {
      grip.setPointerCapture(event.pointerId);
    } catch {}
  }

  function gripMove(event) {
    if (!press) return;
    press.sx = event.screenX;
    press.sy = event.screenY;
    if (press.carried) {
      // Окно ведёт программа: она ставит его так, чтобы точка захвата шла за указателем.
      ipc(`move:${event.screenX},${event.screenY}`);
      return;
    }
    const dy = event.clientY - press.y;
    const away = right() ? press.x - event.clientX : event.clientX - press.x;
    if (!press.active) {
      if (Math.abs(dy) < DRAG_START && away < DRAG_START) return;
      press.active = true;
      closeChat(true);
      hideLabel();
      wrap.classList.add('pulling');
    }
    placeDock(press.top + dy);
    // Тянется за указателем всё туже, как будто держится за стену.
    const pull = Math.max(0, away);
    wrap.style.setProperty('--pull', `${(PEEL * 0.5 * (1 - Math.exp(-pull / PEEL))).toFixed(1)}px`);
    if (pull > PEEL) {
      press.carried = true;
      wrap.classList.remove('pulling');
      wrap.classList.add('loose', 'carried');
      wrap.style.removeProperty('--pull');
      ipc(`carry:${event.clientX},${event.clientY}`);
    }
    regionSoon();
  }

  function release(event) {
    const held = press;
    if (!held) return;
    if (held.carried) {
      if (held.dropping) return;
      held.dropping = true;
      // Программа решит, к какой стене ближе, и вернёт место через dockDropped.
      // Захват могли потерять без отпускания: тогда кладём там, где указатель был последним.
      const sx = event.type === 'lostpointercapture' ? held.sx ?? event.screenX : event.screenX;
      const sy = event.type === 'lostpointercapture' ? held.sy ?? event.screenY : event.screenY;
      ipc(`drop:${sx},${sy}`);
      return;
    }
    press = null;
    if (!held.active) return;
    wrap.classList.remove('pulling');
    wrap.style.removeProperty('--pull');
    send('PUT', '/api/settings', { dockOffset: saveOffset() });
    regionSoon();
    scheduleTuck();
  }

  document.addEventListener('pointerup', (event) => {
    if (press?.carried) release(event);
  });

  // Док отпустили: окно уже у нужной стены, а док прилетает к ней из точки, где его бросили.
  window.dockDropped = (edge, x, y) => {
    const held = press;
    press = null;
    settings.dockEdge = edge;
    document.body.classList.toggle('left', edge === 'left');
    dock.update({ side: edge === 'left' ? 'left' : 'right' });
    wrap.classList.remove('carried');
    const grabX = held?.grabX ?? 30;
    const grabY = held?.grabY ?? 30;
    placeDock(y - grabY);
    const rest = wrap.getBoundingClientRect().left;
    const from = x - grabX - rest;
    wrap.classList.remove('loose');
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
      wrap.animate([{ translate: `${from}px 0` }, { translate: '0 0' }], { duration: 560, easing: 'cubic-bezier(0.34, 1.45, 0.5, 1)' }).onfinish = sendRegion;
    }
    send('PUT', '/api/settings', { dockEdge: edge, dockOffset: saveOffset() });
    sendRegion();
    scheduleTuck();
  };

  document.addEventListener('contextmenu', (event) => {
    if (!event.target.closest('.chat-log, .chat-input')) event.preventDefault();
  });

  // --- Состояние -------------------------------------------------------

  function applyLook() {
    const theme = settings.uiTheme;
    const dark = theme ? theme === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    if (settings.uiFont && settings.uiFont !== 'system') document.documentElement.dataset.font = settings.uiFont;
    else delete document.documentElement.dataset.font;
    if (!press) document.body.classList.toggle('left', !right());
    $('chat-close').title = tr('Свернуть');
    $('chat-close').setAttribute('aria-label', $('chat-close').title);
    chatMenu.title = tr('Ещё');
    chatMenu.setAttribute('aria-label', chatMenu.title);
    chatSend.title = tr('Отправить');
    chatSend.setAttribute('aria-label', chatSend.title);
  }

  let refreshVersion = 0;
  async function refresh() {
    const version = ++refreshVersion;
    try {
      const data = await api('/api/state');
      if (version !== refreshVersion) return;
      // У окна дока свой профиль WebView: язык берём из настроек программы и перечитываем страницу один раз.
      const lang = data.settings?.uiLang;
      if (lang && lang !== window.LANG && window.rememberLang?.(lang)) {
        location.reload();
        return;
      }
      settings = data.settings || {};
      if (data.debug && !fakeVoice && !window.DeloistVoice) loadFakeVoice();
      bots = data.bots || [];
      ipc(`config:${JSON.stringify({ layer: settings.dockLayer || 'top', edge: settings.dockEdge || 'right', display: settings.dockDisplay || '' })}`);
      applyLook();
      busy.clear();
      for (const bot of bots) {
        if (bot.busy) busy.add(bot.id);
        if (typeof bot.queued === 'number') queued.set(bot.id, bot.queued);
        const at = bot.preview?.at || 0;
        const seen = lastAt.get(bot.id);
        const answered = seen !== undefined && at > seen && bot.preview.role !== 'user' && !bot.preview.error;
        const wanted = settings.dockResults !== false && bot.dock !== false && !mainActive && openId !== bot.id;
        if (answered && finished.has(bot.id) && wanted) addCard(bot);
        lastAt.set(bot.id, at);
      }
      finished.clear();
      await seedWaiting();
      if (settings.dockResults === false && cards.length) {
        cards = [];
        renderCards();
      }
      paint();
      scheduleTuck();
    } catch {
      // Сервер мог перезапускаться: окно живо, попробуем в следующий раз.
    }
  }

  // «Нужен ты»: вопросы ждут и после конца хода и перезапуска, событий о них док мог не видеть.
  async function seedWaiting() {
    try {
      const { items } = await api('/api/needs');
      waiting.clear();
      for (const item of items || []) {
        if (!waiting.has(item.botId)) waiting.set(item.botId, new Set());
        waiting.get(item.botId).add(item.id);
      }
    } catch {
      // старый сервер без списка
    }
  }

  function refreshSoon() {
    // Старый ответ не должен перезаписать более свежий профиль, пока ждём следующий запрос.
    refreshVersion++;
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 300);
  }

  function chatEvent(event) {
    const stick = nearBottom();
    switch (event.type) {
      case 'turn_start':
        showTyping();
        break;
      case 'text_delta': {
        removeTyping();
        chatLog.querySelector('.chat-empty')?.remove();
        if (!live || live.turnId !== event.turnId) {
          live = { turnId: event.turnId, text: '', el: document.createElement('div') };
          live.el.className = 'm-bot';
          chatLog.append(live.el);
        }
        live.text += event.text || '';
        live.el.innerHTML = md(live.text);
        break;
      }
      case 'message': {
        const msg = event.message || {};
        if (msg.role === 'user') {
          // Своё сообщение уже на экране: снимаем с него отметку «отправляется».
          const pending = [...chatLog.querySelectorAll('.m-user.pending')].find((el) => el.dataset.pending === String(msg.text || '').trim());
          if (pending) {
            pending.classList.remove('pending');
            delete pending.dataset.pending;
          } else if (msg.text) {
            chatLog.append(messageNode(msg));
          }
          break;
        }
        if (msg.role !== 'assistant' || !msg.text) break;
        removeTyping();
        chatLog.querySelector('.chat-empty')?.remove();
        const node = messageNode(msg);
        if (live && live.turnId === event.turnId) live.el.replaceWith(node);
        else chatLog.append(node);
        markLastAnswer();
        live = null;
        break;
      }
      case 'turn_end':
        removeTyping();
        live = null;
        break;
      default:
        break;
    }
    if (stick) toBottom();
  }

  // --- Голос -------------------------------------------------------
  // Договор с горячими клавишами: событие { type: 'voice_state', state, botId, level, progress, error, message }.
  // state: idle | downloading | loading | recording | transcribing | sending | done | error.
  // Нажатие на микрофон зовёт window.DeloistVoice.toggle('dock'), Esc — cancel().
  const VOICE_STATES = new Set(['idle', 'downloading', 'loading', 'recording', 'transcribing', 'sending', 'done', 'error']);
  // Чтение ответа вслух: { state: 'speaking', botId, level } около 20 раз в секунду, конец — { state: 'spoken', botId }.
  // Кольцо бота «звучит» по уровню. Источник может не прислать конец: без событий 700 мс кольцо замирает.
  // Остановить чтение: window.DeloistVoice.stopSpeaking(botId).
  let speaking = { botId: '', level: 0 };
  let speakTimer = 0;
  function setSpeaking(next) {
    speaking = next;
    clearTimeout(speakTimer);
    if (next.botId) speakTimer = setTimeout(() => setSpeaking({ botId: '', level: 0 }), 700);
    dock.update({ speaking: { ...speaking } });
  }
  function stopSpeaking() {
    const id = speaking.botId;
    if (!id) return false;
    try {
      window.DeloistVoice?.stopSpeaking?.(id);
    } catch {}
    setSpeaking({ botId: '', level: 0 });
    return true;
  }
  window.dockVoice = (event) => {
    if (event?.state === 'speaking' && event.botId) {
      setSpeaking({ botId: String(event.botId), level: Math.min(1, Math.max(0, Number(event.level) || 0)) });
      return;
    }
    if (event?.state === 'spoken') {
      if (!event.botId || event.botId === speaking.botId) setSpeaking({ botId: '', level: 0 });
      return;
    }
    if (!event || !VOICE_STATES.has(event.state)) return;
    const was = voice.state;
    voice = {
      state: event.state,
      level: Number(event.level) || 0,
      progress: Number(event.progress) || 0,
      botId: event.botId || voice.botId || '',
      error: event.error || '',
      message: event.message || '',
    };
    clearTimeout(voiceTimer);
    // Источник может не прислать «покой»: галочка и ошибка уходят сами.
    if (voice.state === 'done') voiceTimer = setTimeout(() => window.dockVoice({ state: 'idle' }), 900);
    if (voice.state === 'error') voiceTimer = setTimeout(() => window.dockVoice({ state: 'idle' }), 4500);
    if (voice.state === 'sending' && voice.botId) lastTalked = voice.botId;
    dock.update({ voice: { ...voice }, voiceTargetId: voiceTargetId() });
    if (was === 'idle' && voice.state !== 'idle') {
      if (tucked) setTucked(false);
      regionSoon();
      scheduleTuck();
    }
    if (voice.state === 'idle' && was !== 'idle') {
      trayRegionLater();
      scheduleTuck();
    }
  };

  function pressMic() {
    // Бот читает вслух: микрофон сначала останавливает чтение.
    if (stopSpeaking()) return;
    try {
      window.DeloistVoice?.toggle?.('dock');
    } catch {}
  }

  function voiceAction(action) {
    if (action === 'cancel') {
      try {
        window.DeloistVoice?.cancel?.();
      } catch {}
      window.dockVoice({ state: 'idle' });
      return;
    }
    // Пополнение и настройки микрофона живут в окне программы.
    window.dockVoice({ state: 'idle' });
    ipc('open');
  }

  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (['downloading', 'loading', 'recording', 'transcribing', 'error'].includes(voice.state)) {
      event.stopImmediatePropagation();
      voiceAction('cancel');
    }
  }, true);

  function loadFakeVoice() {
    fakeVoice = true;
    const script = document.createElement('script');
    script.src = '/dock-fake-voice.js';
    script.onload = () => dock.update({ voiceAvailable: Boolean(window.DeloistVoice?.toggle) });
    document.head.append(script);
  }

  function handle(event) {
    if (event.type === 'voice_state') {
      window.dockVoice(event);
      return;
    }
    window.DeloistVoice?.dockEvent?.(event);
    if (event.type === 'state_changed') {
      refreshSoon();
      return;
    }
    const id = event.botId;
    if (!id) return;
    if (id === openId) chatEvent(event);
    stepEvent(event);
    avatarStates?.feed(event);
    const wait = window.CrewAvatarStates?.waitOf(event);
    switch (event.type) {
      case 'turn_start':
        busy.add(id);
        waiting.delete(id);
        repaintBot(id);
        break;
      case 'turn_end':
        busy.delete(id);
        waiting.delete(id);
        if (!event.failed) finished.add(id);
        repaintBot(id);
        if (event.failed && avatarStates) {
          const { mood, seconds } = avatarStates.endMood(id);
          for (const avatar of avatarsOf(id)) avatar.feel(mood, seconds);
        } else if (event.failed) {
          for (const avatar of avatarsOf(id)) avatar.feel('error', 5);
        }
        if (id === openId) showWaitingNote();
        refreshSoon();
        break;
      case 'approval':
      case 'question':
      case 'browser_needs_human':
      case 'specialist_offer':
      case 'team_proposal':
        if (!wait?.open) break;
        if (!waiting.has(id)) waiting.set(id, new Set());
        waiting.get(id).add(wait.open);
        repaintBot(id);
        if (id === openId) showWaitingNote();
        break;
      case 'approval_closed':
      case 'question_closed':
        waiting.get(id)?.delete(event.id);
        repaintBot(id);
        if (id === openId) showWaitingNote();
        break;
      case 'tool_start':
      case 'tool_update':
      case 'delegate_start': {
        // Просьба посмотреть браузер и предложение специалиста не имеют события закрытия: бот пошёл дальше сам.
        const set = waiting.get(id);
        const before = set?.size || 0;
        for (const key of [...(set || [])]) if (key === 'human' || key.startsWith('offer:')) set.delete(key);
        if (before !== (set?.size || 0)) repaintBot(id);
        break;
      }
      case 'queue_updated':
        queued.set(id, Array.isArray(event.items) ? event.items.length : 0);
        repaintBot(id);
        break;
      default:
        break;
    }
  }

  function connect() {
    // Поток событий общий с питомцами: профиль один, а соединений к серверу у браузера не больше шести.
    const { port } = new SharedWorker('/pet-events.js');
    port.addEventListener('message', ({ data }) => {
      if (data === null) {
        refreshSoon();
        return;
      }
      try {
        handle(JSON.parse(data));
      } catch {
        // мусорное событие — пропускаем
      }
    });
    port.start();
  }

  // Программа сообщает, в фокусе ли её главное окно: тогда ответы и так видны, карточки не нужны.
  // --- Компонент дока -------------------------------------------------------

  const dock = window.DeloistUI.mountDock(wrap, {
    ...dockProps(),
    onseat: (id) => toggleChat(id),
    onstopspeak: () => stopSpeaking(),
    onshowbot: (id) => send('PATCH', `/api/bots/${encodeURIComponent(id)}`, { dock: true }).then(refreshSoon),
    onseatenter: (id, el) => showLabel(el),
    onseatleave: () => hideLabel(),
    onface: (id, el) => {
      faces.set(id, el);
      const bot = botById(id);
      if (bot) paintFace(el, bot, `seat:${id}`, 44);
    },
    onfaceremove: (id, el) => {
      if (faces.get(id) === el) faces.delete(id);
    },
    onadd: addBot,
    onmic: pressMic,
    onsetting: (patch) => {
      Object.assign(settings, patch.voiceTarget === 'last' ? { voiceTarget: '' } : patch);
      if ('dockEdge' in patch) document.body.classList.toggle('left', patch.dockEdge === 'left');
      dock.update(dockProps());
      send('PUT', '/api/settings', patch).then(refreshSoon);
      scheduleTuck();
      regionSoon();
    },
    onhide: () => send('PUT', '/api/settings', { dockEnabled: false }),
    onvoiceaction: voiceAction,
    ongrip: wireGrip,
    onregion: regionSoon,
  });

  // Событие напрямую, мимо потока: поддельный голос и проверки дока зовут его так же, как поток.
  window.dockEvent = (event) => handle(event);

  window.dockMainActive = (active) => {
    mainActive = Boolean(active);
    if (mainActive) {
      unread.clear();
      for (const bot of docked()) repaintBot(bot.id);
    }
  };

  // Окно переехало на другой монитор или сменило размер.
  window.dockResized = () => {
    if (!press) placeFromOffset();
    if (openId) placeChat();
    for (const id of pills.keys()) placePill(id);
    regionSoon();
  };
  addEventListener('resize', () => window.dockResized());
  // Горячие клавиши голоса и снимка открывают мини-чат через voice.js.
  // Место на доке под кнопки других скриптов (микрофон из voice.js, потом — Svelte MicButton):
  // над «+». Кнопка с тем же именем заменяет прежнюю; вернувшаяся функция её убирает.
  const mounted = new Map();
  function mount(name, el) {
    mounted.get(name)?.remove();
    mounted.set(name, el);
    addButton.before(el);
    regionSoon();
    return () => {
      if (mounted.get(name) !== el) return;
      mounted.delete(name);
      el.remove();
      regionSoon();
    };
  }
  window.dockHooks = { openChat, send: sendMessage, openId: () => openId, mount };
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyLook);

  applyLook();
  connect();
  refresh();
  // Страховка от пропущенных событий: поток мог оборваться, пока программа перезапускалась.
  setInterval(refresh, 10000);
})();
