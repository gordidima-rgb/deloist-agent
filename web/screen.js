// Экран бота: живая картинка его браузера, управление человеком, живая оболочка и «Научи бота».
// Отдельный скрипт: app.js только сообщает события (deloist:event), выбор бота (deloist:bot)
// и открывает панель кнопкой «Браузер».

(() => {
  const tr = (text, vars) => (window.tr ? window.tr(text, vars) : text);
  // Строки сервера (ошибки, шаг бота, причина без картинки) в английском интерфейсе переводятся.
  const srv = (text) => (window.trServer ? window.trServer(text) : text);
  const esc = (s) =>
    String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  async function api(path, body) {
    const res = await fetch(path, {
      method: body === undefined ? 'GET' : 'POST',
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
    return data;
  }

  const STYLE = `
.screen-panel{box-sizing:border-box;position:fixed;top:var(--k-space-3,12px);right:var(--k-space-3,12px);width:min(42vw,640px);height:min(40vh,430px);max-width:calc(100vw - 24px);z-index:40;display:flex;flex-direction:column;gap:8px;padding:12px;
  background:var(--k-surface,#fff);color:var(--k-text,#121212);border:1px solid var(--k-line,#e8e8e8);border-radius:var(--k-radius-lg,26px);
  box-shadow:var(--k-shadow-2);font-size:var(--text-sm,13px);overflow:hidden}
.screen-panel[hidden]{display:none}
.screen-panel [hidden]{display:none!important}
.sp-head{position:relative;display:flex;align-items:center;gap:var(--k-space-1,4px)}
.sp-head strong{margin-right:var(--k-space-2,8px)}
.sp-ibtn{flex:none;display:inline-flex;align-items:center;justify-content:center;width:32px;height:32px;padding:0;border:0;border-radius:var(--k-radius-sm,12px);background:transparent;color:var(--k-text-2,#4a4a4a);cursor:pointer;transition:background var(--k-dur-fast,140ms) var(--k-ease-out,ease),transform var(--k-dur-instant,90ms) var(--k-ease-out,ease)}
.sp-ibtn:hover{background:var(--k-surface-2,#f0f0f0);color:var(--k-text,#121212)}
.sp-ibtn:active{transform:scale(.94)}
.sp-ibtn:focus-visible{outline:none;box-shadow:var(--k-focus)}
.sp-ibtn svg{width:16px;height:16px;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round}
.sp-ibtn .ic-restore{display:none}
.screen-panel.is-max .sp-ibtn .ic-max{display:none}
.screen-panel.is-max .sp-ibtn .ic-restore{display:block}
.sp-head strong{font-family:var(--font-heading,inherit);font-size:var(--text-md,15px)}
.sp-step{flex:1;min-width:0;color:var(--color-text-muted,#5f6878);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.sp-bar{display:flex;gap:6px;align-items:center}
.sp-bar form{flex:1;display:flex}
.k-btn.rec.on{background:var(--k-danger-fill);color:#fff}
.sp-bar input,.sp-teach input,.sp-review input,.sp-review textarea{flex:1;min-width:0;border:1px solid var(--color-border-default,#e5e7eb);border-radius:9px;padding:6px 9px;font:inherit;background:var(--color-bg-surface,#fff);color:inherit}
.sp-tabs{display:flex;gap:6px;overflow-x:auto;scrollbar-width:thin}
.sp-tab{max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;border-radius:8px;padding:3px 8px;border:1px solid var(--color-border-default,#e5e7eb);background:none;color:var(--color-text-secondary,#334155);font:inherit;cursor:pointer}
.sp-tab.current{border-color:var(--k-brand);color:var(--k-brand-text)}
.sp-view{position:relative;flex:1;min-height:0;background:var(--k-ink);border-radius:10px;overflow:hidden;outline:none}
.sp-view img{width:100%;height:100%;object-fit:contain;display:block;user-select:none;-webkit-user-drag:none}
.sp-view.held{box-shadow:inset 0 0 0 3px var(--color-accent,#ff6a4d);cursor:crosshair}
.sp-view:focus-visible{box-shadow:inset 0 0 0 3px var(--k-brand)}
.sp-empty{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;padding:24px;text-align:center;color:var(--k-on-ink-2)}
.sp-row{display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.sp-row:has(> [data-note]:empty){display:none}
.sp-note{color:var(--color-text-muted,#5f6878)}
.sp-teach{display:flex;gap:6px;align-items:center}
.sp-steps{display:flex;gap:8px;overflow-x:auto;max-height:150px;scrollbar-width:thin}
.sp-steps:empty{display:none}
.sp-stepcard{flex:0 0 150px;border:1px solid var(--color-border-default,#e5e7eb);border-radius:9px;padding:6px;font-size:var(--text-xs,12px)}
.sp-stepcard img{width:100%;height:70px;object-fit:cover;border-radius:6px;display:block;margin-bottom:4px;background:var(--k-line)}
.sp-stepcard.secret{border-color:var(--color-accent,#ff6a4d)}
.sp-review{display:flex;flex-direction:column;gap:6px;max-height:45%;overflow:auto}
.sp-review[hidden]{display:none}
.sp-review textarea{min-height:140px;font-family:var(--font-mono,monospace);font-size:var(--text-xs,12px)}
.sp-shell{border-radius:10px;background:var(--k-ink);color:var(--k-on-ink);display:flex;flex-direction:column;max-height:30%;min-height:90px}
.sp-shell[hidden]{display:none}
.sp-shell-head{display:flex;gap:8px;align-items:center;padding:6px 10px;border-bottom:1px solid var(--k-ink-line);font-family:var(--font-mono,monospace);font-size:var(--text-xs,12px)}
.sp-shell-head code{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.sp-shell pre{margin:0;padding:8px 10px;overflow:auto;flex:1;font-family:var(--font-mono,monospace);font-size:var(--text-xs,12px);white-space:pre-wrap;word-break:break-word}
.sp-shell pre .err{color:#fca5a5}
@media (max-width:760px){.screen-panel{top:56px;right:8px;width:min(92vw,480px);height:min(38vh,340px)}body.sp-max .screen-panel{left:8px;right:8px;top:56px;bottom:8px;height:auto}}

/* Рядом с чатом: чат уступает место, панель его не перекрывает. */
.sp-splitter{position:fixed;top:var(--k-space-3,12px);bottom:var(--k-space-3,12px);right:calc(var(--sp-w) + var(--k-space-3,12px));width:var(--k-space-3,12px);z-index:41;display:none;cursor:col-resize;touch-action:none;outline:none}
.sp-splitter::before{content:"";position:absolute;inset:0 -6px}
.sp-splitter::after{content:"";position:absolute;left:4px;top:50%;width:4px;height:48px;margin-top:-24px;border-radius:var(--k-radius-pill,999px);background:var(--k-line-strong,#d2d2d2);transition:background var(--k-dur-fast,140ms) var(--k-ease-out,ease),height var(--k-dur-fast,140ms) var(--k-ease-out,ease)}
.sp-splitter:hover::after,.sp-splitter:focus-visible::after,body.sp-drag .sp-splitter::after{background:var(--k-brand);height:72px;margin-top:-36px}
.sp-splitter:focus-visible::after{box-shadow:var(--k-focus)}
body.sp-drag{user-select:none;cursor:col-resize}
@media (min-width:761px){body.sp-split.sp-open:not(.sp-drag) #chat{transition:margin-right var(--k-dur-base,220ms) var(--k-ease-out,ease)}}

/* Развёрнуто: панель на всю область справа от списка ботов, чат становится островком. */
body.sp-max .screen-panel{left:var(--sp-left,308px);right:var(--k-space-3,12px);bottom:var(--k-space-3,12px);height:auto;width:auto;max-width:none}
@media (max-width:760px){body.sp-max .screen-panel{left:8px;right:8px;top:56px;bottom:8px}}
@media (max-height:650px){body.sp-split .screen-panel{height:min(50vh,430px)}}
body.sp-split .screen-panel .sp-teach,body.sp-split .screen-panel .sp-steps,body.sp-split .screen-panel .sp-review,body.sp-split .screen-panel .sp-shell{display:none}
.sp-more summary{cursor:pointer;list-style:none}.sp-more summary::-webkit-details-marker{display:none}.sp-more-menu{position:absolute;right:0;top:100%;z-index:3;box-sizing:border-box;width:min(280px,100%);padding:6px;border-radius:9px;background:var(--k-surface,#fff);box-shadow:var(--k-shadow-2)}.sp-more-menu .sp-btn{white-space:normal;text-align:left;width:100%}
body.sp-min .screen-panel{display:none}
.sp-edgepill,.sp-chatpill{display:none}
@media (min-width:761px){
  body.sp-max.sp-open #chat{position:fixed;z-index:45;width:min(400px,calc(100vw - var(--sp-left,308px) - 48px));height:min(580px,calc(100vh - 100px));margin:0;border-radius:var(--k-radius-island,28px);overflow:hidden;background:var(--k-surface,#fff);box-shadow:var(--k-shadow-island);border:1px solid var(--k-line,#e8e8e8)}
  body.sp-max.sp-open #chat.sp-c-br{right:24px;bottom:24px}
  body.sp-max.sp-open #chat.sp-c-tr{right:24px;top:76px}
  body.sp-max.sp-open #chat.sp-c-bl{left:calc(var(--sp-left,308px) + 12px);bottom:24px}
  body.sp-max.sp-open #chat.sp-c-tl{left:calc(var(--sp-left,308px) + 12px);top:76px}
  body.sp-max.sp-open #chat.sp-chatmin{display:none}
  body.sp-max.sp-open #chat .view-head{background:var(--k-ink,#000);color:var(--k-on-ink,#fff);padding:var(--k-space-2,8px) var(--k-space-3,12px);cursor:grab;touch-action:none;border-bottom:0;min-height:0}
  body.sp-max.sp-open #chat .view-head:focus-visible{outline:none;box-shadow:inset var(--k-focus)}
  body.sp-max.sp-open #chat .head-name{color:var(--k-on-ink,#fff)}
  body.sp-max.sp-open #chat .head-actions,body.sp-max.sp-open #chat .head-role,body.sp-max.sp-open #chat .only-mobile,body.sp-max.sp-open #chat .composer-bar,body.sp-max.sp-open #chat .grants-line,body.sp-max.sp-open #chat .queue,body.sp-max.sp-open #chat .composer .attach{display:none}
  body.sp-max.sp-open #chat .sp-island-min{display:inline-flex;margin-left:auto;color:var(--k-on-ink-2,rgba(255,255,255,.64))}
  body.sp-max.sp-open #chat .sp-island-min:hover{background:var(--k-ink-2,#17181c);color:var(--k-on-ink,#fff)}
  body.sp-max.sp-open.sp-dragchat #chat{transition:none}
  body.sp-max.sp-open.sp-dragchat #chat .view-head{cursor:grabbing}
  body.sp-max.sp-open.sp-chatmin .sp-chatpill{display:flex}
}
#chat .sp-island-min{display:none}
.sp-chatpill{position:fixed;z-index:45;width:56px;height:56px;padding:0;align-items:center;justify-content:center;border:0;border-radius:var(--k-radius-pill,999px);background:var(--k-ink,#000);color:var(--k-on-ink,#fff);box-shadow:var(--k-shadow-island);outline:1px solid var(--k-ink-line,rgba(255,255,255,.09));outline-offset:-1px;cursor:pointer;transition:transform var(--k-dur-fast,140ms) var(--k-ease-spring,ease)}
.sp-chatpill:hover{background:var(--k-ink-2,#17181c);transform:scale(1.05)}
.sp-chatpill:focus-visible{outline:none;box-shadow:var(--k-shadow-island),var(--k-focus)}
.sp-chatpill .avatar{width:40px;height:40px;pointer-events:none;overflow:visible}
.sp-chatpill.sp-c-br{right:24px;bottom:24px}
.sp-chatpill.sp-c-tr{right:24px;top:24px}
.sp-chatpill.sp-c-bl{left:calc(var(--sp-left,308px) + 12px);bottom:24px}
.sp-chatpill.sp-c-tl{left:calc(var(--sp-left,308px) + 12px);top:24px}
.sp-count{position:absolute;top:-4px;right:-4px;min-width:20px;height:20px;padding:0 5px;border-radius:var(--k-radius-pill,999px);background:var(--k-attention,#ff5a3c);color:var(--k-on-brand,#fff);font-size:var(--k-text-micro,11px);font-weight:600;line-height:20px;text-align:center;box-shadow:0 0 0 2px var(--k-ink,#000)}
.sp-count:empty{display:none}

/* Свёрнуто: таблетка с миниатюрой у края окна. */
.sp-edgepill{position:fixed;right:0;bottom:96px;z-index:41;align-items:center;gap:var(--k-space-2,8px);padding:var(--k-space-2,8px) var(--k-space-2,8px) var(--k-space-2,8px) var(--k-space-3,12px);border:0;border-radius:var(--k-radius-island,28px) 0 0 var(--k-radius-island,28px);background:var(--k-ink,#000);color:var(--k-on-ink,#fff);box-shadow:var(--k-shadow-island);outline:1px solid var(--k-ink-line,rgba(255,255,255,.09));outline-offset:-1px;cursor:pointer;font:inherit;font-size:var(--k-text-xs,12px);transition:background var(--k-dur-fast,140ms) var(--k-ease-out,ease),transform var(--k-dur-base,220ms) var(--k-ease-spring,ease)}
body.sp-min.sp-open .sp-edgepill{display:flex}
.sp-edgepill:hover{background:var(--k-ink-2,#17181c);transform:translateX(-4px)}
.sp-edgepill:focus-visible{outline:none;box-shadow:var(--k-shadow-island),var(--k-focus)}
.sp-edgepill img{width:72px;height:46px;object-fit:cover;object-position:top;border-radius:var(--k-radius-xs,6px);background:var(--k-ink-3,#24262c);display:block}
.sp-edgepill img[hidden]{display:none}
@media (prefers-reduced-motion:reduce){
  body.sp-split.sp-open #chat,.sp-edgepill,.sp-chatpill,.sp-ibtn,.sp-splitter::after{transition:none!important}
  .sp-edgepill:hover,.sp-chatpill:hover{transform:none}
}

/* Строка в чате: бот просит посмотреть на экран. */
.screen-note{display:flex;align-items:center;gap:var(--k-space-3,12px);align-self:center;max-width:min(560px,100%);margin:var(--k-space-2,8px) auto;padding:var(--k-space-2,8px) var(--k-space-3,12px);border-radius:var(--k-radius-md,18px);background:var(--k-attention-tint,#ffe8e2);color:var(--k-attention-text,#b4321b);font-size:var(--k-text-sm,13px)}
.screen-note span{min-width:0;overflow-wrap:anywhere}
.screen-note button{flex:none;border:0;border-radius:var(--k-radius-pill,999px);padding:4px 12px;background:var(--k-ink,#000);color:var(--k-on-ink,#fff);font:inherit;cursor:pointer}
.screen-note button:focus-visible{outline:none;box-shadow:var(--k-focus)}
`;

  const P = {
    bot: null,
    es: null,
    held: false,
    meta: null,
    recording: false,
    humanApproval: new Map(), // бот → id карточки browser_human
    queue: Promise.resolve(),
    wheel: null,
    recorded: 0,
    shellBot: null,
    shellSize: 0,
    draftSteps: [],
  };
  let root = null;
  let control = null;
  let holdRequest = null;
  const $ = (sel) => root.querySelector(sel);

  // ---------------------------------------------------------------------------
  // Раскладка: рядом с чатом, развёрнуто (чат — островок), свёрнуто в таблетку
  // ---------------------------------------------------------------------------

  const layoutLib = () => (window.DeloistUI && window.DeloistUI.screenLayout) || null;
  const storage = () => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  };
  const reducedMotion = () => Boolean(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const wide = () => !window.matchMedia || window.matchMedia('(min-width: 761px)').matches;
  const L = {
    width: 560,
    corner: 'br',
    mode: 'split', // split | max | min
    before: 'split', // из какого режима свернули
    chatMin: false, // чат-островок свёрнут в кружок
    unread: 0,
    loaded: false,
  };
  let splitter = null;
  let edgePill = null;
  let chatPill = null;
  let islandBtn = null;

  function loadLayout() {
    if (L.loaded) return;
    L.loaded = true;
    const lib = layoutLib();
    if (!lib) return;
    const saved = lib.loadLayout(storage());
    L.width = saved.width;
    L.corner = saved.corner;
    // Режим не помним: экран всегда открывается рядом с чатом, чтобы не прятать чат без спроса.
  }

  function saveLayout() {
    const lib = layoutLib();
    if (lib) lib.saveLayout(storage(), { width: L.width, corner: L.corner, mode: 'split' });
  }

  function contentLeft() {
    const side = document.getElementById('sidebar');
    const right = side ? Math.round(side.getBoundingClientRect().right) : 0;
    return right > 0 && right <= window.innerWidth - 320 ? right : 0;
  }

  function containerWidth() {
    return Math.max(0, window.innerWidth - contentLeft());
  }

  function setMode(action) {
    const lib = layoutLib();
    if (!lib) return;
    const next = lib.nextMode({ mode: L.mode, before: L.before }, action);
    L.mode = next.mode;
    L.before = next.before;
    if (L.mode !== 'max') L.chatMin = false;
    saveLayout();
    applyLayout();
  }

  function applyLayout() {
    const body = document.body;
    const lib = layoutLib();
    const opened = isOpen();
    const mode = opened && lib ? L.mode : 'split';
    body.classList.toggle('sp-open', opened);
    body.classList.toggle('sp-split', opened && mode === 'split');
    body.classList.toggle('sp-max', opened && mode === 'max');
    body.classList.toggle('sp-min', opened && mode === 'min');
    body.classList.toggle('sp-chatmin', opened && mode === 'max' && L.chatMin);
    const chat = document.getElementById('chat');
    if (chat) {
      for (const c of ['tl', 'tr', 'bl', 'br']) chat.classList.toggle(`sp-c-${c}`, c === L.corner);
      chat.classList.toggle('sp-chatmin', opened && mode === 'max' && L.chatMin);
    }
    if (chatPill) {
      for (const c of ['tl', 'tr', 'bl', 'br']) chatPill.classList.toggle(`sp-c-${c}`, c === L.corner);
    }
    if (!opened || !lib) {
      body.style.removeProperty('--sp-w');
      body.style.removeProperty('--sp-left');
      return;
    }
    L.width = lib.clampPanel(L.width, containerWidth());
    body.style.setProperty('--sp-w', `${L.width}px`);
    body.style.setProperty('--sp-left', `${contentLeft() + 12}px`);
    root.classList.toggle('is-max', mode === 'max');
    const maxBtn = $('[data-max]');
    const maxText = mode === 'max' ? tr('Свернуть до превью') : tr('Развернуть');
    maxBtn.title = maxText;
    maxBtn.setAttribute('aria-label', maxText);
    if (splitter) {
      splitter.setAttribute('aria-valuenow', String(L.width));
      splitter.setAttribute('aria-valuemin', String(lib.MIN_PANEL));
      splitter.setAttribute('aria-valuemax', String(lib.clampPanel(Infinity, containerWidth())));
    }
    if (mode === 'min') syncThumb();
    syncChatPill();
  }

  function syncThumb() {
    if (!edgePill) return;
    const img = $('[data-img]');
    const thumb = edgePill.querySelector('img');
    thumb.hidden = !img || img.hidden || !img.src;
    if (!thumb.hidden) thumb.src = img.src;
  }

  function syncChatPill() {
    if (!chatPill) return;
    const head = document.getElementById('head-avatar');
    const slot = chatPill.querySelector('[data-avatar]');
    slot.innerHTML = '';
    if (head) {
      const copy = head.cloneNode(true);
      copy.removeAttribute('id');
      // Аватар рисуется на canvas: клон пуст, пока не перенесём кадр.
      const from = head.querySelectorAll('canvas');
      copy.querySelectorAll('canvas').forEach((canvas, i) => {
        try {
          canvas.getContext('2d').drawImage(from[i], 0, 0);
        } catch {
          // без кадра остаётся пустой кружок: имя видно в подсказке
        }
      });
      slot.append(copy);
    }
    const name = (document.getElementById('head-name') || {}).textContent || '';
    chatPill.setAttribute('aria-label', tr('Открыть чат: {name}', { name }));
    chatPill.title = tr('Открыть чат: {name}', { name });
    const count = chatPill.querySelector('.sp-count');
    const lib = layoutLib();
    count.textContent = lib ? lib.badge(L.unread) : '';
  }

  function setChatMin(value) {
    L.chatMin = Boolean(value);
    if (!L.chatMin) L.unread = 0;
    applyLayout();
    if (!L.chatMin) {
      const messages = document.getElementById('messages');
      if (messages) messages.scrollTop = messages.scrollHeight;
      const input = document.getElementById('input');
      if (input && !input.disabled) input.focus({ preventScroll: true });
    } else if (chatPill) {
      chatPill.focus({ preventScroll: true });
    }
  }

  function buildLayoutChrome() {
    splitter = document.createElement('div');
    splitter.className = 'sp-splitter';
    splitter.setAttribute('role', 'separator');
    splitter.setAttribute('aria-orientation', 'vertical');
    splitter.setAttribute('aria-label', tr('Ширина экрана бота'));
    splitter.tabIndex = 0;
    document.body.append(splitter);

    edgePill = document.createElement('button');
    edgePill.type = 'button';
    edgePill.className = 'sp-edgepill';
    edgePill.title = tr('Показать экран бота');
    edgePill.setAttribute('aria-label', tr('Показать экран бота'));
    edgePill.innerHTML = `<img alt="" hidden><span>${esc(tr('Экран бота'))}</span>`;
    document.body.append(edgePill);

    chatPill = document.createElement('button');
    chatPill.type = 'button';
    chatPill.className = 'sp-chatpill';
    chatPill.innerHTML = '<span data-avatar></span><span class="sp-count" aria-live="polite"></span>';
    document.body.append(chatPill);

    const head = document.getElementById('chat-head');
    if (head) {
      islandBtn = document.createElement('button');
      islandBtn.type = 'button';
      islandBtn.className = 'sp-ibtn sp-island-min';
      islandBtn.title = tr('Свернуть чат');
      islandBtn.setAttribute('aria-label', tr('Свернуть чат'));
      islandBtn.innerHTML = DeloistUI.icon('minimize');
      head.append(islandBtn);
      head.setAttribute('aria-roledescription', 'draggable');
    }

    $('[data-max]').addEventListener('click', () => setMode('maximize'));
    $('[data-min]').addEventListener('click', () => setMode('minimize'));
    edgePill.addEventListener('click', () => setMode('restore'));
    chatPill.addEventListener('click', () => setChatMin(false));
    if (islandBtn) islandBtn.addEventListener('click', () => setChatMin(true));
    bindSplitter();
    bindIsland();
    window.addEventListener('resize', () => {
      if (isOpen()) applyLayout();
    });
    // Esc возвращает «развёрнуто» в «рядом»; диалоги и меню закрываются своим Esc.
    document.addEventListener(
      'keydown',
      (e) => {
        if (e.key !== 'Escape' || !isOpen() || L.mode !== 'max' || e.defaultPrevented) return;
        if (document.querySelector('dialog[open]') || root.querySelector('.sp-more[open]')) return;
        e.preventDefault();
        e.stopImmediatePropagation();
        setMode('back');
      },
      true
    );
  }

  function bindSplitter() {
    const lib = layoutLib();
    if (!lib) return;
    let dragging = false;
    splitter.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      dragging = true;
      try {
        splitter.setPointerCapture(e.pointerId);
      } catch {
        // синтетическое событие: без захвата указателя тянем по движению над разделителем
      }
      document.body.classList.add('sp-drag');
      e.preventDefault();
    });
    splitter.addEventListener('pointermove', (e) => {
      if (!dragging) return;
      L.width = lib.widthFromPointer(e.clientX, window.innerWidth, containerWidth());
      applyLayout();
    });
    const stop = () => {
      if (!dragging) return;
      dragging = false;
      document.body.classList.remove('sp-drag');
      saveLayout();
    };
    splitter.addEventListener('pointerup', stop);
    splitter.addEventListener('pointercancel', stop);
    splitter.addEventListener('lostpointercapture', stop);
    splitter.addEventListener('keydown', (e) => {
      const next = lib.widthByKey(L.width, e.key, e.shiftKey, containerWidth());
      if (next === L.width && !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
      e.preventDefault();
      L.width = next;
      applyLayout();
      saveLayout();
    });
    splitter.addEventListener('dblclick', () => {
      L.width = lib.clampPanel(lib.DEFAULT_PANEL, containerWidth());
      applyLayout();
      saveLayout();
    });
  }

  // Островок чата: тянется за шапку, при отпускании прилипает к ближайшему углу.
  function bindIsland() {
    const lib = layoutLib();
    const chat = document.getElementById('chat');
    const head = document.getElementById('chat-head');
    if (!lib || !chat || !head) return;
    head.tabIndex = 0;
    let drag = null;
    const bounds = () => ({ left: contentLeft(), top: 0, width: containerWidth(), height: window.innerHeight });
    const moveTo = (corner) => {
      const first = chat.getBoundingClientRect();
      L.corner = lib.validCorner(corner, L.corner);
      applyLayout();
      saveLayout();
      const last = chat.getBoundingClientRect();
      if (!reducedMotion() && chat.animate && (first.left !== last.left || first.top !== last.top)) {
        chat.animate(
          [{ transform: `translate(${first.left - last.left}px, ${first.top - last.top}px)` }, { transform: 'none' }],
          { duration: 260, easing: 'cubic-bezier(0.34, 1.36, 0.5, 1)' }
        );
      }
    };
    head.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || !document.body.classList.contains('sp-max') || e.target.closest('button')) return;
      const rect = chat.getBoundingClientRect();
      drag = { id: e.pointerId, dx: e.clientX - rect.left, dy: e.clientY - rect.top, w: rect.width, h: rect.height, moved: false };
      try {
        head.setPointerCapture(e.pointerId);
      } catch {
        // синтетическое событие
      }
    });
    head.addEventListener('pointermove', (e) => {
      if (!drag || e.pointerId !== drag.id) return;
      if (!drag.moved) {
        drag.moved = true;
        document.body.classList.add('sp-dragchat');
      }
      const b = bounds();
      const left = Math.min(Math.max(e.clientX - drag.dx, b.left), b.left + b.width - drag.w);
      const top = Math.min(Math.max(e.clientY - drag.dy, 0), b.height - drag.h);
      chat.style.setProperty('left', `${left}px`);
      chat.style.setProperty('top', `${top}px`);
      chat.style.setProperty('right', 'auto');
      chat.style.setProperty('bottom', 'auto');
    });
    const release = (e) => {
      if (!drag || e.pointerId !== drag.id) return;
      const moved = drag.moved;
      drag = null;
      document.body.classList.remove('sp-dragchat');
      if (!moved) return;
      const rect = chat.getBoundingClientRect();
      const corner = lib.nearestCorner(rect, bounds());
      chat.style.removeProperty('left');
      chat.style.removeProperty('top');
      chat.style.removeProperty('right');
      chat.style.removeProperty('bottom');
      // Анимация идёт от места, где отпустили, к углу.
      L.corner = lib.validCorner(corner);
      applyLayout();
      saveLayout();
      const last = chat.getBoundingClientRect();
      if (!reducedMotion() && chat.animate) {
        chat.animate(
          [{ transform: `translate(${rect.left - last.left}px, ${rect.top - last.top}px)` }, { transform: 'none' }],
          { duration: 260, easing: 'cubic-bezier(0.34, 1.36, 0.5, 1)' }
        );
      }
    };
    head.addEventListener('pointerup', release);
    head.addEventListener('pointercancel', release);
    // С клавиатуры: стрелки переносят островок в соседний угол.
    head.addEventListener('keydown', (e) => {
      if (!document.body.classList.contains('sp-max') || e.target !== head) return;
      const [v, h] = [L.corner[0], L.corner[1]];
      let next = null;
      if (e.key === 'ArrowLeft') next = `${v}l`;
      else if (e.key === 'ArrowRight') next = `${v}r`;
      else if (e.key === 'ArrowUp') next = `t${h}`;
      else if (e.key === 'ArrowDown') next = `b${h}`;
      if (!next) return;
      e.preventDefault();
      moveTo(next);
    });
  }

  // ---------------------------------------------------------------------------
  // Строка в чате и автооткрытие
  // ---------------------------------------------------------------------------

  let lastLine = { text: '', at: 0 };

  // Короткая строка в ленте чата: «Бот просит посмотреть…» с кнопкой, которая открывает экран.
  function chatLine(bot, text) {
    const messages = document.getElementById('messages');
    if (!messages || window.DeloistCurrentBot !== bot) return;
    const now = Date.now();
    if (lastLine.text === text && now - lastLine.at < 5000) return;
    lastLine = { text, at: now };
    const line = document.createElement('div');
    line.className = 'screen-note';
    line.setAttribute('role', 'status');
    const span = document.createElement('span');
    span.textContent = text;
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = tr('Открыть экран');
    button.addEventListener('click', () => open(bot));
    line.append(span, button);
    const stick = messages.scrollHeight - messages.scrollTop - messages.clientHeight < 180;
    messages.append(line);
    if (stick) messages.scrollTop = messages.scrollHeight;
  }

  function chatVisible(bot) {
    const chat = document.getElementById('chat');
    const button = document.getElementById('btn-browser');
    return Boolean(chat && !chat.hidden && button && !button.hidden && window.DeloistCurrentBot === bot);
  }

  function build() {
    if (root) return;
    const style = document.createElement('style');
    style.textContent = STYLE;
    document.head.append(style);
    root = document.createElement('aside');
    root.className = 'screen-panel';
    root.hidden = true;
    root.setAttribute('aria-label', tr('Экран бота'));
    root.innerHTML = `
      <div class="sp-head">
        <strong>${esc(tr('Экран бота'))}</strong>
        <span class="sp-step" data-step></span>
        <details class="sp-more"><summary class="sp-ibtn" aria-label="${esc(tr('Дополнительно'))}" title="${esc(tr('Дополнительно'))}">⋯</summary><div class="sp-more-menu"><button type="button" class="k-btn k-btn--ghost k-btn--sm" data-window title="${esc(tr('Вывести настоящее окно браузера: например, для капчи, которой нужно окно'))}">${esc(tr('Открыть настоящее окно браузера'))}</button></div></details>
        <button type="button" class="sp-ibtn sp-max-btn" data-max title="${esc(tr('Развернуть'))}" aria-label="${esc(tr('Развернуть'))}">
          ${DeloistUI.icon('expand', { className: 'ic-max' })}
          ${DeloistUI.icon('shrink', { className: 'ic-restore' })}
        </button>
        <button type="button" class="sp-ibtn" data-min title="${esc(tr('Свернуть'))}" aria-label="${esc(tr('Свернуть'))}">
          ${DeloistUI.icon('minimize')}
        </button>
        <button type="button" class="sp-ibtn" data-close title="${esc(tr('Закрыть экран бота'))}" aria-label="${esc(tr('Закрыть экран бота'))}">
          ${DeloistUI.icon('close')}
        </button>
      </div>
      <div class="sp-bar">
        <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-nav="back" title="${esc(tr('Назад'))}">←</button>
        <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-nav="forward" title="${esc(tr('Вперёд'))}">→</button>
        <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-nav="reload" title="${esc(tr('Обновить'))}">⟳</button>
        <form data-address><input name="url" autocomplete="off" spellcheck="false" placeholder="${esc(tr('Адрес страницы'))}" aria-label="${esc(tr('Адрес страницы'))}"></form>
      </div>
      <div class="sp-tabs" data-tabs></div>
      <div class="sp-view" data-view tabindex="0">
        <img data-img alt="${esc(tr('Экран браузера бота'))}" hidden>
        <div class="sp-empty" data-empty>${esc(tr('Подключаюсь к браузеру бота…'))}</div>
        <div data-control></div>
      </div>
      <div class="sp-row">
        <span class="sp-note" data-note></span>
      </div>
      <div class="sp-teach">
        <button type="button" class="k-btn k-btn--ghost k-btn--sm rec" data-rec title="${esc(tr('Научи бота: покажи задачу, а бот запишет её как навык'))}">● ${esc(tr('Запись'))}</button>
        <input data-narration placeholder="${esc(tr('Пояснение к шагу (необязательно)'))}" aria-label="${esc(tr('Пояснение к шагу'))}">
        <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-addnote>${esc(tr('Добавить'))}</button>
      </div>
      <div class="sp-steps" data-steps></div>
      <div class="sp-review" data-review hidden>
        <span class="sp-note" data-warning></span>
        <input data-name aria-label="${esc(tr('Название навыка'))}" placeholder="${esc(tr('Название навыка'))}">
        <input data-desc aria-label="${esc(tr('Когда применять'))}" placeholder="${esc(tr('Когда применять'))}">
        <textarea data-body aria-label="${esc(tr('Шаги навыка'))}"></textarea>
        <div class="sp-row">
          <button type="button" class="k-btn k-btn--primary k-btn--sm" data-save>${esc(tr('Сохранить навык'))}</button>
          <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-try disabled>${esc(tr('Проверить навык'))}</button>
          <button type="button" class="k-btn k-btn--ghost k-btn--sm" data-discard>${esc(tr('Не сохранять'))}</button>
        </div>
      </div>
      <div class="sp-shell" data-shell hidden>
        <div class="sp-shell-head"><code data-cmd></code><span data-code></span><button type="button" class="k-btn k-btn--ghost k-btn--sm" data-shellstop>${esc(tr('Стоп команды'))}</button></div>
        <pre data-out></pre>
      </div>`;
    document.body.append(root);
    control = window.DeloistUI.mountScreenControl($('[data-control]'), { ontoggle: () => hold(!P.held) });
    bind();
    buildLayoutChrome();
  }

  // ---------------------------------------------------------------------------
  // Поток картинки
  // ---------------------------------------------------------------------------

  function connect() {
    disconnect();
    if (!P.bot || document.visibilityState === 'hidden') return;
    const es = new EventSource(`/api/bots/${encodeURIComponent(P.bot)}/screen`);
    P.es = es;
    es.onmessage = (e) => {
      let msg;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg.type === 'frame') showFrame(msg);
      else if (msg.type === 'tabs') showTabs(msg);
      else if (msg.type === 'control') {
        setHeld(msg.held);
        setRecording(msg.recording);
      } else if (msg.type === 'unavailable') showEmpty(msg.message);
      // Команда бота и её вывод приходят только сюда, в поток экрана, а не в общий поток событий.
      else if (msg.type === 'shell_start' || msg.type === 'shell_output' || msg.type === 'shell_end') shell(msg);
    };
  }

  function disconnect() {
    if (P.es) P.es.close();
    P.es = null;
  }

  function showFrame(msg) {
    const img = $('[data-img]');
    img.src = `data:image/jpeg;base64,${msg.data}`;
    img.hidden = false;
    $('[data-empty]').hidden = true;
    P.meta = msg.meta;
    if (L.mode === 'min') syncThumb();
  }

  function showEmpty(text) {
    $('[data-img]').hidden = true;
    const empty = $('[data-empty]');
    empty.hidden = false;
    empty.textContent = window.trServer ? window.trServer(text) : text;
  }

  function showTabs(msg) {
    const input = $('[data-address] input');
    if (document.activeElement !== input) input.value = msg.url || '';
    const box = $('[data-tabs]');
    box.innerHTML = '';
    (msg.tabs || []).forEach((tab, i) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = `sp-tab${tab.current ? ' current' : ''}`;
      b.textContent = tab.title || tab.url || tr('Пустая вкладка');
      b.title = tab.url || '';
      b.addEventListener('click', () => send('nav', { tab: i + 1 }));
      box.append(b);
    });
    box.hidden = (msg.tabs || []).length < 2;
  }

  // ---------------------------------------------------------------------------
  // Управление
  // ---------------------------------------------------------------------------

  function note(text) {
    $('[data-note]').textContent = text || '';
  }

  function setHeld(held) {
    const changed = P.held !== Boolean(held);
    P.held = Boolean(held);
    control?.update({ held: P.held });
    $('[data-view]').classList.toggle('held', P.held);
    // Подпись меняется только при смене состояния: иначе повторное событие затрёт ошибку.
    if (changed && !P.held && !P.recording) note('');
    if (changed && P.held && !P.recording) note(tr('Управление у тебя: бот ждёт. Кликай и печатай прямо в картинке.'));
  }

  function setRecording(on) {
    P.recording = Boolean(on);
    const btn = $('[data-rec]');
    btn.classList.toggle('on', P.recording);
    btn.textContent = P.recording ? `■ ${tr('Стоп')}` : `● ${tr('Запись')}`;
    if (P.recording) note(tr('Идёт запись: делай задачу как обычно. Пароли не записываются.'));
  }

  async function hold(on) {
    if (holdRequest) return;
    const request = { bot: P.bot };
    holdRequest = request;
    control?.update({ busy: true, error: '' });
    try {
      await api(`/api/bots/${request.bot}/screen/control`, { hold: on });
      if (holdRequest !== request || P.bot !== request.bot || !isOpen()) return;
      setHeld(on);
      if (on) $('[data-view]').focus();
      // Бот звал человека: «Вернуть боту» значит «Готово».
      const card = P.humanApproval.get(request.bot);
      if (!on && card) {
        P.humanApproval.delete(request.bot);
        await api(`/api/approvals/${card}`, { answer: 'once' }).catch(() => {});
      }
    } catch (err) {
      if (holdRequest !== request || P.bot !== request.bot || !isOpen()) return;
      note(srv(err.message));
      control?.update({ error: srv(err.message) });
    } finally {
      if (holdRequest === request) {
        holdRequest = null;
        control?.update({ busy: false });
      }
    }
  }

  // Ввод идёт строго по порядку: иначе буквы перепутаются.
  function send(path, body) {
    const bot = P.bot;
    P.queue = P.queue.then(async () => {
      try {
        const res = await api(`/api/bots/${bot}/screen/${path}`, body);
        if (typeof res.recorded === 'number' && res.recorded !== P.recorded) {
          P.recorded = res.recorded;
          refreshSteps();
        }
      } catch (err) {
        note(srv(err.message));
      }
    });
    return P.queue;
  }

  function point(e) {
    const img = $('[data-img]');
    const rect = img.getBoundingClientRect();
    const nw = img.naturalWidth || rect.width;
    const nh = img.naturalHeight || rect.height;
    const scale = Math.min(rect.width / nw, rect.height / nh);
    const w = nw * scale;
    const h = nh * scale;
    const left = rect.left + (rect.width - w) / 2;
    const top = rect.top + (rect.height - h) / 2;
    return { x: (e.clientX - left) / w, y: (e.clientY - top) / h, iw: nw, ih: nh };
  }

  function modifiers(e) {
    return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
  }

  function bind() {
    $('[data-close]').addEventListener('click', close);
    $('[data-window]').addEventListener('click', async () => {
      root.querySelector('.sp-more').open = false;
      try {
        await api(`/api/bots/${P.bot}/browser`, {});
        note(tr('Окно браузера открыто поверх всех. Закончишь — просто закрой его.'));
      } catch (err) {
        note(srv(err.message));
      }
    });
    document.addEventListener('click', (e) => {
      const menu = root.querySelector('.sp-more');
      if (menu.open && !menu.contains(e.target)) menu.open = false;
    });
    document.addEventListener('keydown', (e) => {
      const menu = root.querySelector('.sp-more');
      if (e.key === 'Escape' && menu.open) {
        e.preventDefault();
        e.stopImmediatePropagation();
        menu.open = false;
        menu.querySelector('summary').focus();
      }
    });
    root.querySelectorAll('[data-nav]').forEach((b) =>
      b.addEventListener('click', () => (P.held ? send('nav', { action: b.dataset.nav }) : note(tr('Сначала возьми управление: кнопка «Взять управление».'))))
    );
    $('[data-address]').addEventListener('submit', (e) => {
      e.preventDefault();
      const url = e.target.url.value.trim();
      if (!url) return;
      if (!P.held) return note(tr('Сначала возьми управление: кнопка «Взять управление».'));
      send('nav', { url });
      $('[data-view]').focus();
    });

    const view = $('[data-view]');
    const img = $('[data-img]');
    img.addEventListener('mousedown', (e) => {
      e.preventDefault();
      view.focus();
      if (!P.held) return note(tr('Сначала возьми управление: кнопка «Взять управление».'));
      const button = e.button === 2 ? 'right' : e.button === 1 ? 'middle' : 'left';
      send('input', { kind: 'click', ...point(e), button, count: Math.min(e.detail || 1, 3), modifiers: modifiers(e) });
    });
    img.addEventListener('contextmenu', (e) => e.preventDefault());
    view.addEventListener(
      'wheel',
      (e) => {
        if (e.target.closest('[data-control]')) return;
        if (!P.held) return;
        e.preventDefault();
        const at = point(e);
        if (!P.wheel) {
          P.wheel = { ...at, dx: 0, dy: 0 };
          setTimeout(() => {
            const w = P.wheel;
            P.wheel = null;
            send('input', { kind: 'wheel', ...w });
          }, 80);
        }
        P.wheel.dx += e.deltaX;
        P.wheel.dy += e.deltaY;
      },
      { passive: false }
    );
    view.addEventListener('keydown', (e) => {
      if (e.target.closest('[data-control]')) return;
      if (!P.held || e.isComposing) return;
      if (['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'].includes(e.key)) return;
      e.preventDefault();
      send('input', { kind: 'key', key: e.key, code: e.code, keyCode: e.keyCode, modifiers: modifiers(e) });
    });

    $('[data-rec]').addEventListener('click', toggleRecording);
    $('[data-addnote]').addEventListener('click', addNote);
    $('[data-narration]').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        addNote();
      }
    });
    $('[data-save]').addEventListener('click', saveSkill);
    $('[data-try]').addEventListener('click', trySkill);
    $('[data-discard]').addEventListener('click', () => {
      $('[data-review]').hidden = true;
      $('[data-steps]').innerHTML = '';
    });
    $('[data-shellstop]').addEventListener('click', async () => {
      try {
        await api(`/api/bots/${P.shellBot || P.bot}/screen/shell/stop`, {});
      } catch (err) {
        note(srv(err.message));
      }
    });
  }

  // ---------------------------------------------------------------------------
  // «Научи бота»
  // ---------------------------------------------------------------------------

  function stepText(step) {
    const t = step.target || {};
    const what = t.label ? `«${t.label}»` : t.selector || '';
    switch (step.kind) {
      case 'open':
        return tr('Открыть {url}', { url: step.url });
      case 'click':
        return tr('Нажать {what}', { what });
      case 'type':
        return tr('Ввести «{text}» в {what}', { text: step.value, what });
      case 'press':
        return tr('Нажать {key}', { key: step.value });
      case 'secret':
        if (t.secret === 'пароль') return tr('Человек ввёл пароль');
        if (t.secret === 'данные карты') return tr('Человек ввёл данные карты');
        if (t.secret === 'код') return tr('Человек ввёл код подтверждения');
        return tr('Человек ввёл данные, значение не записано');
      default:
        return step.note || '';
    }
  }

  function renderSteps(steps) {
    const box = $('[data-steps]');
    box.innerHTML = '';
    steps.forEach((step, i) => {
      const card = document.createElement('div');
      card.className = `sp-stepcard${step.kind === 'secret' ? ' secret' : ''}`;
      const thumb = step.thumb ? `<img alt="" src="data:image/jpeg;base64,${step.thumb}">` : '';
      const after = step.after ? `<div class="sp-note">→ ${esc(step.after)}</div>` : '';
      const said = step.note ? `<div class="sp-note">${esc(step.note)}</div>` : '';
      card.innerHTML = `${thumb}<div>${i + 1}. ${esc(stepText(step))}</div>${after}${said}`;
      box.append(card);
    });
    box.scrollLeft = box.scrollWidth;
  }

  async function refreshSteps() {
    try {
      const data = await api(`/api/bots/${P.bot}/screen/teach`);
      if (data.recording) renderSteps(data.steps);
    } catch {
      // запись уже закончилась
    }
  }

  async function toggleRecording() {
    if (P.recording) return stopRecording();
    try {
      if (!P.held) await hold(true);
      const data = await api(`/api/bots/${P.bot}/screen/teach/start`, {});
      $('[data-review]').hidden = true;
      P.recorded = data.steps.length;
      renderSteps(data.steps);
      setRecording(true);
      $('[data-view]').focus();
    } catch (err) {
      note(srv(err.message));
    }
  }

  async function addNote() {
    const input = $('[data-narration]');
    const text = input.value.trim();
    if (!text || !P.recording) return;
    try {
      const data = await api(`/api/bots/${P.bot}/screen/teach/note`, { text });
      input.value = '';
      renderSteps(data.steps);
    } catch (err) {
      note(srv(err.message));
    }
  }

  async function stopRecording() {
    const btn = $('[data-rec]');
    btn.disabled = true;
    note(tr('Пишу навык по записи…'));
    try {
      await P.queue;
      const data = await api(`/api/bots/${P.bot}/screen/teach/stop`, {});
      setRecording(false);
      renderSteps(data.steps);
      P.draftSteps = data.steps;
      $('[data-review]').hidden = false;
      $('[data-warning]').textContent = data.warning ? tr('Модель не помогла: {why}', { why: data.warning }) : tr('Проверь и поправь навык, потом сохрани.');
      $('[data-name]').value = data.draft.name;
      $('[data-desc]').value = data.draft.description;
      $('[data-body]').value = data.draft.body;
      $('[data-try]').disabled = true;
      note('');
    } catch (err) {
      note(srv(err.message));
      setRecording(false);
    } finally {
      btn.disabled = false;
    }
  }

  async function saveSkill() {
    try {
      const data = await api(`/api/bots/${P.bot}/screen/teach/save`, {
        name: $('[data-name]').value,
        description: $('[data-desc]').value,
        body: $('[data-body]').value,
      });
      $('[data-try]').disabled = false;
      $('[data-try]').dataset.name = data.skill.name;
      $('[data-warning]').textContent = data.created ? tr('Навык сохранён и подключён боту.') : tr('Навык обновлён. Прошлую версию можно вернуть.');
    } catch (err) {
      note(srv(err.message));
    }
  }

  async function trySkill() {
    const name = $('[data-try]').dataset.name;
    if (!name) return;
    try {
      if (P.held) await hold(false);
      await api(`/api/bots/${P.bot}/messages`, { text: tr('Выполни навык «{name}» на открытой странице и проверь каждый шаг.', { name }) });
      note(tr('Бот проверяет навык. Смотри на экране.'));
    } catch (err) {
      note(srv(err.message));
    }
  }

  // ---------------------------------------------------------------------------
  // Живая оболочка
  // ---------------------------------------------------------------------------

  function shell(ev) {
    if (!root) return;
    const box = $('[data-shell]');
    const out = $('[data-out]');
    if (ev.type === 'shell_start') {
      P.shellBot = P.bot;
      P.shellSize = 0;
      box.hidden = false;
      out.textContent = '';
      $('[data-cmd]').textContent = `$ ${ev.command}`;
      $('[data-code]').textContent = '';
      $('[data-shellstop]').hidden = false;
    } else if (ev.type === 'shell_output') {
      box.hidden = false;
      P.shellSize += ev.text.length;
      if (P.shellSize > 400000) return;
      const span = document.createElement('span');
      if (ev.stream === 'err') span.className = 'err';
      span.textContent = ev.text;
      const stick = out.scrollTop + out.clientHeight >= out.scrollHeight - 8;
      out.append(span);
      if (stick) out.scrollTop = out.scrollHeight;
    } else if (ev.type === 'shell_end') {
      $('[data-shellstop]').hidden = true;
      $('[data-code]').textContent = ev.stopped ? tr('остановлена') : tr('код {code}', { code: ev.code });
    }
  }

  // ---------------------------------------------------------------------------
  // Открыть и закрыть
  // ---------------------------------------------------------------------------

  function open(bot, message) {
    build();
    if (P.bot !== bot) {
      holdRequest = null;
      control?.update({ busy: false, error: '' });
      note('');
      $('[data-steps]').innerHTML = '';
      $('[data-review]').hidden = true;
      $('[data-shell]').hidden = true;
      $('[data-step]').textContent = '';
      setRecording(false);
      setHeld(false);
      showEmpty(tr('Подключаюсь к браузеру бота…'));
    }
    P.bot = bot;
    root.hidden = false;
    loadLayout();
    // Бот зовёт — таблетка разворачивается. Фокус в поле ввода не трогаем.
    if (L.mode === 'min') {
      const next = layoutLib() ? layoutLib().nextMode({ mode: L.mode, before: L.before }, 'restore') : { mode: 'split', before: 'split' };
      L.mode = next.mode;
      L.before = next.before;
    }
    applyLayout();
    connect();
    if (message) note(tr('Бот просит: {message}', { message }));
  }

  function close() {
    if (!root) return;
    holdRequest = null;
    control?.update({ busy: false, error: '' });
    note('');
    root.querySelector('.sp-more').open = false;
    root.hidden = true;
    disconnect();
    L.chatMin = false;
    L.unread = 0;
    applyLayout();
  }

  function isOpen() {
    return Boolean(root && !root.hidden);
  }

  window.DeloistScreen = {
    open,
    close,
    toggle(bot) {
      if (isOpen() && P.bot === bot && L.mode === 'min') setMode('restore');
      else if (isOpen() && P.bot === bot) close();
      else open(bot);
    },
  };

  window.addEventListener('deloist:bot', (e) => {
    if (isOpen() && e.detail !== P.bot) close();
  });

  window.addEventListener('deloist:event', (e) => {
    const ev = e.detail || {};
    switch (ev.type) {
      case 'approval':
        if (ev.tool === 'browser_human') P.humanApproval.set(ev.botId, ev.id);
        break;
      case 'approval_closed':
        if (P.humanApproval.get(ev.botId) === ev.id) P.humanApproval.delete(ev.botId);
        break;
      case 'screen_open': {
        // Бот зовёт человека: открываем экран, если открыт его чат.
        if (chatVisible(ev.botId)) open(ev.botId, ev.message);
        break;
      }
      case 'browser_needs_human': {
        // Страница входа или просьба бота посмотреть: панель открывается сама, ничего не нажимая на странице.
        if (!chatVisible(ev.botId)) break;
        const reason = srv(String(ev.reason || ''));
        chatLine(
          ev.botId,
          ev.asked ? tr('Бот просит посмотреть: {reason}', { reason }) : tr('{reason}. Открыл экран бота.', { reason })
        );
        open(ev.botId);
        break;
      }
      case 'message':
        if (ev.message && ev.message.role !== 'user' && L.chatMin && ev.botId === P.bot && isOpen()) {
          L.unread += 1;
          syncChatPill();
        }
        break;
      case 'screen_control':
        if (root && ev.botId === P.bot) setHeld(ev.held);
        break;
      case 'screen_record':
        if (root && ev.botId === P.bot && !ev.recording && P.recording) setRecording(false);
        break;
      case 'screen_step':
        if (root && ev.botId === P.bot) $('[data-step]').textContent = window.trServer ? window.trServer(ev.text) : ev.text;
        break;
      case 'turn_end':
        if (root && ev.botId === P.bot) $('[data-step]').textContent = '';
        break;
      default:
    }
  });

  window.addEventListener('deloist:bot', (e) => {
    window.DeloistCurrentBot = e.detail;
  });

  // Окно ушло в трей или свернулось: картинку не гоняем.
  document.addEventListener('visibilitychange', () => {
    if (!isOpen()) return;
    if (document.visibilityState === 'hidden') disconnect();
    else connect();
  });

  // Кнопка «Браузер» спрятана — значит, открыт не чат бота: экран закрываем.
  window.addEventListener('deloist:ready', () => {
    const button = document.getElementById('btn-browser');
    if (!button) return;
    new MutationObserver(() => {
      if (button.hidden) close();
    }).observe(button, { attributes: true, attributeFilter: ['hidden'] });
  });
})();
