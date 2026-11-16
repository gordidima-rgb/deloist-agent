// Питомец на рабочем столе: один бот в своём прозрачном окне. Только чтение состояния и клики.
(() => {
  const own = window.PET_BOT;
  const root = document.getElementById('pet');
  const CLOSE_ICON = '<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 6l12 12"/><path d="M18 6L6 18"/></svg>';
  const host = document.createElement('div');
  host.className = 'pet-avatar';
  root.append(host);

  let bot = null;
  let avatar = null;
  let busy = false;
  let queued = 0;
  const waiting = new Set(); // id подтверждений, вопросов и просьб к человеку
  // Знак занятия и эмоции от событий хода: правила в avatar-states.js.
  const avatarStates = window.CrewAvatarStates?.createTracker(() => [avatar]);
  let refreshTimer = 0;

  const safeColor = (color) => (/^#[0-9a-f]{6}$/i.test(color || '') ? color : '#7a8595');
  const initial = (who) => (who.name || '?').trim().charAt(0).toUpperCase() || '?';
  const ipc = (message) => {
    try {
      window.ipc?.postMessage(message);
    } catch {}
  };

  async function api(path) {
    const res = await fetch(path, { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.json();
  }

  function send(method, path, body) {
    return fetch(path, {
      method,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    }).catch(() => {});
  }

  const stateNow = () => (waiting.size ? 'wait' : busy ? 'work' : 'idle');

  function openChat() {
    send('POST', `/api/bots/${own}/open`, {});
    ipc('open');
  }

  // Окно сообщает новое место, когда его перестали двигать.
  window.petMoved = (x, y) => send('POST', `/api/bots/${own}/pet-place`, { x, y });

  // Курсор где угодно на экране: программа присылает его сдвиг от середины окна в пикселях экрана.
  window.petCursor = (dx, dy) => {
    const dpr = window.devicePixelRatio || 1;
    const x = innerWidth / 2 + dx / dpr;
    const y = innerHeight / 2 + dy / dpr;
    window.CrewAvatar?.pointAt?.(x, y);
  };

  // Клик без движения открывает чат, движение перетаскивает окно.
  let drag = null;
  root.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || event.target.closest('.pet-close')) return;
    drag = { x: event.clientX, y: event.clientY };
    // Нажатие сминает питомца, как игрушку, и он пружинит обратно.
    host.classList.add('squish');
    try {
      root.setPointerCapture(event.pointerId);
    } catch {}
  });
  root.addEventListener('pointermove', (event) => {
    if (!drag) return;
    if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) > 5) {
      // Дальше окно ведёт система, и отпускание кнопки сюда может не дойти.
      drag = null;
      host.classList.remove('squish');
      ipc('drag');
    }
  });
  root.addEventListener('pointerup', () => {
    host.classList.remove('squish');
    if (!drag) return;
    drag = null;
    openChat();
  });
  root.addEventListener('pointercancel', () => {
    host.classList.remove('squish');
    drag = null;
  });
  // Меню браузера у питомца ни к чему.
  root.addEventListener('contextmenu', (event) => event.preventDefault());

  const closeButton = document.createElement('button');
  closeButton.type = 'button';
  closeButton.className = 'pet-close';
  closeButton.innerHTML = CLOSE_ICON;
  closeButton.title = window.tr('Убрать с рабочего стола');
  closeButton.setAttribute('aria-label', closeButton.title);
  closeButton.addEventListener('click', () => send('PATCH', `/api/bots/${own}`, { pet: false }));
  root.append(closeButton);

  function badges() {
    root.querySelector('.pet-badges')?.remove();
    const failed = Boolean(bot.preview?.error) && !busy;
    if (!queued && !failed) return;

    const box = document.createElement('div');
    box.className = 'pet-badges';
    if (queued) {
      const badge = document.createElement('div');
      badge.className = 'avatar-badge';
      badge.textContent = String(queued);
      badge.title = window.tr('В очереди: {count}', { count: queued });
      box.append(badge);
    }
    if (failed) {
      const badge = document.createElement('div');
      badge.className = 'avatar-badge is-error';
      badge.innerHTML = CLOSE_ICON;
      badge.title = bot.preview.text;
      badge.setAttribute('role', 'img');
      badge.setAttribute('aria-label', window.tr('Последний ход не удался'));
      box.append(badge);
    }
    root.append(box);
  }

  function paint() {
    if (!bot) return;
    root.title = bot.name;
    const look = {
      color: safeColor(bot.color),
      skin: bot.skin ?? 0,
      name: bot.name,
    };

    if (!avatar) {
      if (window.CrewAvatar) {
        host.textContent = '';
        host.style.width = '96px';
        host.style.height = '96px';
        avatar = CrewAvatar.mount(host, { size: 96, ...look });
      }
      if (avatar) {
        host.classList.add('live');
        host.style.background = '';
      } else {
        host.textContent = initial(bot);
        host.style.background = safeColor(bot.color);
      }
    }
    if (avatar) avatar.set({ ...look, state: stateNow() });
    badges();
  }

  let refreshVersion = 0;
  async function refresh() {
    const version = ++refreshVersion;
    try {
      const data = await api('/api/state');
      if (version !== refreshVersion) return;
      // У окна питомцев свой профиль: язык берём из настроек программы и перечитываем страницу один раз.
      const lang = data.settings?.uiLang;
      if (lang && lang !== window.LANG && window.rememberLang?.(lang)) {
        location.reload();
        return;
      }
      bot = (data.bots || []).find((item) => item.id === own) || null;
      if (!bot) return;
      busy = Boolean(bot.busy);
      if (typeof bot.queued === 'number') queued = bot.queued;
      paint();
      waiting.clear();
      if (busy) {
        try {
          const { pending } = await api(`/api/bots/${own}/pending`);
          if (version !== refreshVersion) return;
          for (const item of pending || []) waiting.add(item.id);
        } catch {}
      }
      paint();
    } catch {
      // Сервер мог перезапускаться: окно живо, попробуем в следующий раз.
    }
  }

  function refreshSoon() {
    // Старый ответ не должен перезаписать более свежий профиль, пока ждём следующий запрос.
    refreshVersion++;
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 350);
  }

  function handle(event) {
    if (event.type === 'state_changed') {
      refreshSoon();
      return;
    }
    if (event.botId !== own) return;
    avatarStates?.feed(event);
    switch (event.type) {
      case 'turn_start':
        busy = true;
        waiting.clear();
        paint();
        break;
      case 'turn_end':
        busy = false;
        waiting.clear();
        paint();
        if (event.failed && avatar) {
          const { mood, seconds } = avatarStates ? avatarStates.endMood(own) : { mood: 'error', seconds: 5 };
          avatar.feel(mood, seconds);
        }
        refreshSoon();
        break;
      case 'approval':
      case 'question':
      case 'browser_needs_human':
      case 'specialist_offer':
      case 'team_proposal':
        if (window.CrewAvatarStates?.waitOf(event)) waiting.add(window.CrewAvatarStates.waitOf(event).open);
        paint();
        break;
      case 'approval_closed':
      case 'question_closed':
        waiting.delete(event.id);
        paint();
        break;
      case 'tool_start':
      case 'tool_update':
      case 'delegate_start': {
        // Просьба посмотреть браузер и предложение специалиста не имеют события закрытия: бот пошёл дальше сам.
        const before = waiting.size;
        for (const key of [...waiting]) if (key === 'human' || key.startsWith('offer:')) waiting.delete(key);
        if (before !== waiting.size) paint();
        break;
      }
      case 'queue_updated':
        queued = Array.isArray(event.items) ? event.items.length : 0;
        paint();
        break;
      default:
        break;
    }
  }

  function connect() {
    const { port } = new SharedWorker('/pet-events.js');
    port.addEventListener('message', ({ data }) => {
      // Поток оборвался: он восстановится сам, а пропущенное заберём из состояния.
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

  connect();
  refresh();
})();
