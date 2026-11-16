// «Команда за работой»: когда ведущий бот передаёт задачи другим, в его чате растёт дерево —
// ведущий сверху, помощники ветками. У каждого живое кольцо, текущий шаг и таймер, строка раскрывается
// в задачу, шаги и отчёт, остановить можно каждого. Готовый отчёт «поднимается» по ветке к ведущему.
// Состояние строится из тех же событий, что и остальной интерфейс (delegate_*, tool_*, text_delta, turn_*),
// своего хранилища нет. app.js вызывает DeloistTeam.init(hooks) и DeloistTeam.onEvent(ev).
(() => {
  const tr = window.tr || ((text) => text);
  const reduceMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  // Что нужно от приложения: бот по id, «этот чат — чат ведущего?», куда класть карточку и т.д.
  const hooks = {
    bot: () => null,
    isHere: () => false,
    place: () => {},
    stop: async () => {},
    openBot: () => {},
    refreshTeam: () => {},
    toolLabel: (name) => name,
  };

  const runs = []; // все прогоны этой сессии, старые обрезаются
  const owner = new Map(); // botId → идущий прогон, в котором он ведущий или помощник
  let seq = 0;

  const el = (tag, cls, text) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  };

  const plu = (n, one, few, many) => {
    if (window.LANG === 'en') return tr(n === 1 ? one : many);
    const m10 = n % 10;
    const m100 = n % 100;
    if (m10 === 1 && m100 !== 11) return one;
    if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
    return many;
  };

  const clock = (ms) => {
    const total = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
  };

  const colorOf = (id) => {
    const c = hooks.bot(id)?.color;
    return /^#[0-9a-f]{6}$/i.test(c || '') ? c : '';
  };

  // ---------------------------------------------------------------------------
  // Шаги: подпись инструмента и короткий аргумент (адрес или имя файла)
  // ---------------------------------------------------------------------------

  function parseArgs(args) {
    if (args && typeof args === 'object') return args;
    try {
      const value = JSON.parse(String(args || ''));
      return value && typeof value === 'object' ? value : {};
    } catch {
      return {};
    }
  }

  function shortArg(args) {
    const a = parseArgs(args);
    for (const key of ['url', 'path', 'file', 'file_path', 'filename', 'command', 'query', 'q', 'name', 'target']) {
      const v = a[key];
      if (typeof v !== 'string' || !v.trim()) continue;
      let text = v.trim();
      if (/^https?:\/\//i.test(text)) {
        try {
          const u = new URL(text);
          text = u.host + (u.pathname.length > 1 ? u.pathname : '');
        } catch { /* оставляем как есть */ }
      } else if (key !== 'command' && key !== 'query' && key !== 'q' && /[\\/]/.test(text)) {
        text = text.split(/[\\/]/).filter(Boolean).pop() || text;
      }
      text = text.replace(/\s+/g, ' ');
      return text.length > 48 ? `${text.slice(0, 47)}…` : text;
    }
    return '';
  }

  // ---------------------------------------------------------------------------
  // Состояние
  // ---------------------------------------------------------------------------

  const nodeOf = (botId) => {
    const run = owner.get(botId);
    if (!run) return null;
    for (let i = run.nodes.length - 1; i >= 0; i -= 1) {
      const n = run.nodes[i];
      if (n.botId === botId && (n.state === 'working' || n.state === 'queued')) return n;
    }
    return null;
  };

  const isLive = (run) => !run.endedAt;
  const working = (run) => run.nodes.filter((n) => n.state === 'working' || n.state === 'queued').length;
  const stepCount = (run) => run.nodes.reduce((sum, n) => sum + n.stepTotal, 0);
  const elapsed = (run) => (run.endedAt || Date.now()) - run.startedAt;

  function phaseText(run) {
    if (run.endedAt) return tr('Готово');
    const w = working(run);
    if (w) return tr('Работают: {n}', { n: w });
    return run.nodes.length && run.nodes.every((n) => n.state !== 'queued') && run.nodes.some((n) => n.endedAt) ? tr('Собирает итог') : tr('Делит задачу…');
  }

  function summaryText(run) {
    const n = run.nodes.length;
    return `${n} ${plu(n, 'помощник', 'помощника', 'помощников')} · ${clock(elapsed(run))} · ${tr('готово')}`;
  }

  function footText(run) {
    const n = run.nodes.length;
    const s = stepCount(run);
    return `${n} ${plu(n, 'помощник', 'помощника', 'помощников')} · ${s} ${plu(s, 'шаг', 'шага', 'шагов')} · ${clock(elapsed(run))}`;
  }

  function startRun(leadId) {
    const lead = hooks.bot(leadId);
    const run = {
      id: ++seq, leadId, leadName: lead?.name || '', startedAt: Date.now(), endedAt: 0,
      nodes: [], collapsed: false, userToggled: false, visible: true, card: null,
    };
    runs.push(run);
    while (runs.length > 12) {
      const old = runs.shift();
      old.card?.remove();
    }
    owner.set(leadId, run);
    return run;
  }

  function endRun(run) {
    if (run.endedAt) return;
    run.endedAt = Date.now();
    for (const n of run.nodes) {
      if (n.state === 'working' || n.state === 'queued') finishNode(n, 'stopped');
    }
    for (const [id, r] of [...owner]) if (r === run) owner.delete(id);
    if (!run.userToggled) run.collapsed = true;
    paint(run);
    syncTick();
    hooks.refreshTeam();
  }

  function finishNode(n, state, ev = {}) {
    n.state = state;
    n.endedAt = Date.now();
    n.cur = '';
    if (typeof ev.summary === 'string') n.summary = ev.summary;
    if (Number(ev.ms) > 0) n.ms = Number(ev.ms);
    if (Number(ev.tokens) > 0) n.tokens = Number(ev.tokens);
  }

  // ---------------------------------------------------------------------------
  // События
  // ---------------------------------------------------------------------------

  function onEvent(ev) {
    if (!ev || typeof ev !== 'object') return;
    switch (ev.type) {
      case 'delegate_start': {
        let run = owner.get(ev.botId);
        const fresh = !run;
        if (!run) run = startRun(ev.botId);
        const parent = run.nodes.find((n) => n.botId === ev.botId && (n.state === 'working' || n.state === 'queued')) || null;
        const node = {
          id: `${run.id}.${run.nodes.length + 1}`, botId: ev.to, name: ev.target || '', parent,
          depth: parent ? parent.depth + 1 : 1, state: 'queued', task: String(ev.task || ''),
          cur: '', steps: [], stepTotal: 0, startedAt: Number(ev.startedAt) || Date.now(), endedAt: 0, ms: 0,
          tokens: 0, summary: '', stopRequested: false, open: false, row: null,
        };
        run.nodes.push(node);
        owner.set(ev.to, run);
        if (run.collapsed && !run.userToggled) run.collapsed = false;
        paint(run);
        syncTick();
        if (fresh) hooks.refreshTeam();
        else refreshBlock();
        return;
      }
      case 'delegate_end': {
        const run = owner.get(ev.botId);
        if (!run) return;
        let node = null;
        for (let i = run.nodes.length - 1; i >= 0; i -= 1) {
          const n = run.nodes[i];
          if (n.botId === ev.to && (n.state === 'working' || n.state === 'queued')) { node = n; break; }
        }
        if (!node) return;
        const ok = ev.ok !== false && !node.failedTurn;
        finishNode(node, ok ? 'done' : node.stopRequested ? 'stopped' : 'failed', ev);
        // Помощник мог стать ведущим для своих: его прогон остаётся тем же, но сам он больше не занят.
        if (!run.nodes.some((n) => n.botId === ev.to && (n.state === 'working' || n.state === 'queued'))) owner.delete(ev.to);
        paint(run);
        if (ok) travel(run, node);
        refreshBlock();
        return;
      }
      case 'turn_start': {
        const src = String(ev.source || '');
        const node = nodeOf(ev.botId);
        if (node && src.startsWith('delegation:')) {
          node.state = 'working';
          node.cur = tr('Думает');
          paint(owner.get(ev.botId));
          refreshBlock();
          return;
        }
        // Новый ход ведущего, а прежний прогон так и не закрылся: считаем его завершённым.
        const stale = owner.get(ev.botId);
        if (stale && stale.leadId === ev.botId && !src.startsWith('delegation:')) endRun(stale);
        return;
      }
      case 'tool_start': {
        const node = nodeOf(ev.botId);
        if (!node) return;
        const label = hooks.toolLabel(ev.name);
        const arg = shortArg(ev.args);
        node.steps.push({ callId: ev.callId, label, arg, status: 'run' });
        if (node.steps.length > 30) node.steps.shift();
        node.stepTotal += 1;
        node.state = 'working';
        node.cur = arg ? `${label} · ${arg}` : label;
        paint(owner.get(ev.botId));
        refreshBlock();
        return;
      }
      case 'tool_update': {
        const node = nodeOf(ev.botId);
        if (!node) return;
        const step = node.steps.find((s) => s.callId === ev.callId);
        if (step && ev.status !== 'running') step.status = ev.status === 'ok' ? 'ok' : 'bad';
        paint(owner.get(ev.botId));
        return;
      }
      case 'text_delta': {
        const node = nodeOf(ev.botId);
        if (!node || node.cur === tr('Пишет ответ')) return;
        node.cur = tr('Пишет ответ');
        paint(owner.get(ev.botId));
        refreshBlock();
        return;
      }
      case 'turn_end': {
        const node = nodeOf(ev.botId);
        if (node) {
          if (ev.failed) node.failedTurn = true;
          const u = ev.usage || {};
          const total = Number(u.prompt || 0) + Number(u.completion || 0);
          if (total > 0) node.tokens = total;
          return;
        }
        const run = owner.get(ev.botId);
        if (run && run.leadId === ev.botId) endRun(run);
        return;
      }
      default:
    }
  }

  // ---------------------------------------------------------------------------
  // Карточка в чате ведущего
  // ---------------------------------------------------------------------------

  const ICON_TEAM = DeloistUI.icon('org-chart', { className: 'tm-ico' });
  const ICON_CHEV = DeloistUI.icon('chevron-right', { className: 'tm-chev' });
  const MARKS = {
    done: 'M5.2 9.2l2.5 2.5 5.1-5.4',
    failed: 'M6.3 6.3l5.4 5.4M11.7 6.3l-5.4 5.4',
    stopped: 'M6.6 6.6h4.8v4.8H6.6z',
  };

  function ring(state, color) {
    const wrap = el('span', 'tm-ringwrap');
    if (color) wrap.style.setProperty('--tm-c', color);
    wrap.innerHTML = '<svg class="tm-ring" viewBox="0 0 18 18" aria-hidden="true"><circle class="tm-ring-track" cx="9" cy="9" r="7"/><circle class="tm-ring-arc" cx="9" cy="9" r="7"/><circle class="tm-ring-fill" cx="9" cy="9" r="8.4"/><path class="tm-ring-mark"/></svg>';
    wrap.querySelector('.tm-ring-mark').setAttribute('d', MARKS[state] || '');
    return wrap;
  }

  function avatarDot(botId, name) {
    const dot = el('span', 'tm-av', (name || hooks.bot(botId)?.name || '?').trim().charAt(0).toUpperCase() || '?');
    const c = colorOf(botId);
    if (c) dot.style.background = c;
    return dot;
  }

  function ensureCard(run) {
    if (run.card?.isConnected) return run.card;
    if (!hooks.isHere(run.leadId)) return null;
    if (!run.card) run.card = buildCard(run);
    hooks.place(run.card);
    return run.card;
  }

  function buildCard(run) {
    const card = el('div', 'tm-card');
    card.dataset.run = String(run.id);
    const head = el('button', 'tm-head');
    head.type = 'button';
    head.innerHTML = `${ICON_TEAM}<span class="tm-title"></span><span class="tm-status" aria-live="polite"></span>${ICON_CHEV}`;
    head.querySelector('.tm-title').textContent = tr('Команда');
    head.addEventListener('click', () => {
      run.collapsed = !run.collapsed;
      run.userToggled = true;
      paint(run);
    });
    const collapse = el('div', 'tm-collapse');
    const inner = el('div', 'tm-inner');
    const tree = el('div', 'tm-tree');
    const lead = el('div', 'tm-lead');
    lead.append(avatarDot(run.leadId, run.leadName), el('span', 'tm-name', run.leadName || hooks.bot(run.leadId)?.name || ''));
    const kids = el('div', 'tm-kids');
    tree.append(lead, kids);
    const foot = el('div', 'tm-foot');
    inner.append(tree, foot);
    collapse.append(inner);
    card.append(head, collapse);
    // Секунды тикают, только пока карточку видно.
    if ('IntersectionObserver' in window) {
      new IntersectionObserver((entries) => {
        run.visible = entries.some((e) => e.isIntersecting);
        if (run.visible) tick();
      }).observe(card);
    }
    return card;
  }

  // Строка помощника создаётся один раз и дальше только обновляется: раскрытая строка не мигает.
  function buildRow(run, n) {
    const node = el('div', 'tm-node');
    const row = el('div', 'tm-row');
    const main = el('button', 'tm-main');
    main.type = 'button';
    main.setAttribute('aria-expanded', 'false');
    const name = el('span', 'tm-name', n.name);
    const step = el('span', 'tm-step');
    const time = el('span', 'tm-time');
    main.append(avatarDot(n.botId, n.name), name, el('span', 'tm-ringslot'), step, time);
    main.addEventListener('click', () => {
      n.open = !n.open;
      paint(run);
    });
    const stop = el('button', 'tm-stop', '×');
    stop.type = 'button';
    stop.title = tr('Остановить');
    stop.setAttribute('aria-label', `${tr('Остановить')}: ${n.name}`);
    stop.addEventListener('click', () => {
      if (n.stopRequested) return;
      n.stopRequested = true;
      paint(run);
      Promise.resolve(hooks.stop(n.botId)).catch(() => {
        n.stopRequested = false;
        paint(run);
      });
    });
    row.append(main, stop);
    const detail = el('div', 'tm-collapse tm-detailwrap');
    detail.append(el('div', 'tm-inner'));
    const kids = el('div', 'tm-kids');
    node.append(row, detail, kids);
    n.row = { node, main, name, step, time, stop, detail, kids, ringState: '', ringColor: '', detailKey: '' };
    return n.row;
  }

  function stepText(n) {
    if (n.state === 'queued') return tr('Ждёт очереди');
    if (n.state === 'working') return n.cur || tr('Думает');
    if (n.state === 'stopped') return tr('Остановлен');
    if (n.state === 'failed') return n.summary || tr('Не вернул ответ');
    return n.summary || tr('Готово');
  }

  function fillDetail(n) {
    const r = n.row;
    const key = JSON.stringify([n.open, n.task, n.summary, n.state, n.steps.map((s) => [s.label, s.arg, s.status])]);
    if (r.detailKey === key) return;
    r.detailKey = key;
    const inner = r.detail.firstChild;
    inner.replaceChildren();
    const body = el('div', 'tm-detail');
    if (n.task) {
      body.append(el('div', 'tm-cap', tr('Задача')), el('p', 'tm-task', n.task));
    }
    body.append(el('div', 'tm-cap', tr('Шаги')));
    if (n.steps.length) {
      const list = el('ul', 'tm-steps');
      for (const s of n.steps) {
        const li = el('li', `tm-s tm-s-${s.status}`);
        li.append(el('i', 'tm-sdot'), el('span', 'tm-slabel', s.label));
        if (s.arg) li.append(el('span', 'tm-sarg', s.arg));
        list.append(li);
      }
      body.append(list);
    } else {
      body.append(el('p', 'tm-none', tr('Шагов пока нет')));
    }
    if (n.summary) {
      body.append(el('div', 'tm-cap', tr('Отчёт')), el('p', 'tm-report', n.summary));
    }
    inner.append(body);
  }

  function paint(run) {
    const card = ensureCard(run);
    if (!card) return;
    const head = card.querySelector('.tm-head');
    head.setAttribute('aria-expanded', String(!run.collapsed));
    card.dataset.live = isLive(run) ? '1' : '0';
    card.classList.toggle('is-collapsed', run.collapsed);
    card.querySelector(':scope > .tm-collapse').classList.toggle('is-closed', run.collapsed);
    card.querySelector('.tm-status').textContent = run.endedAt ? summaryText(run) : phaseText(run);
    // Ведущий «собирает итог», пока все помощники закончили, а он ещё пишет.
    card.querySelector('.tm-lead').dataset.busy = run.endedAt ? '0' : '1';
    card.querySelector('.tm-foot').textContent = footText(run);

    // Строки: ставим каждого под своего родителя; порядок — порядок передачи.
    const rootKids = card.querySelector('.tm-tree > .tm-kids');
    for (const n of run.nodes) {
      const r = n.row || buildRow(run, n);
      const host = n.parent ? n.parent.row.kids : rootKids;
      if (r.node.parentNode !== host) host.append(r.node);
      r.node.dataset.state = n.state;
      r.name.textContent = n.name;
      r.step.textContent = stepText(n);
      r.step.title = r.step.textContent;
      const color = colorOf(n.botId);
      if (r.ringState !== n.state || r.ringColor !== color) {
        r.ringState = n.state;
        r.ringColor = color;
        r.main.querySelector('.tm-ringslot').replaceChildren(ring(n.state, color));
      }
      r.main.setAttribute('aria-expanded', String(n.open));
      r.node.classList.toggle('is-open', n.open);
      r.detail.classList.toggle('is-closed', !n.open);
      r.stop.hidden = !(n.state === 'working' || n.state === 'queued');
      r.stop.disabled = n.stopRequested;
      r.stop.classList.toggle('is-wait', n.stopRequested);
      fillDetail(n);
      r.time.textContent = clock((n.endedAt || Date.now()) - n.startedAt);
    }
  }

  // Точка идёт от помощника вверх по ветке к тому, кто ему поручил: готовый отчёт «возвращается».
  function travel(run, n) {
    if (reduceMotion() || !run.card?.isConnected || run.collapsed) return;
    const tree = run.card.querySelector('.tm-tree');
    const from = n.row?.node.querySelector('.tm-av');
    const to = (n.parent ? n.parent.row.node : tree.querySelector('.tm-lead'))?.querySelector('.tm-av');
    if (!tree || !from || !to) return;
    requestAnimationFrame(() => {
      const base = tree.getBoundingClientRect();
      const a = from.getBoundingClientRect();
      const b = to.getBoundingClientRect();
      if (!a.height || !b.height) return;
      const p = (rect) => [rect.left - base.left + rect.width / 2 - 4, rect.top - base.top + rect.height / 2 - 4];
      const [ax, ay] = p(a);
      const [bx, by] = p(b);
      const dot = el('i', 'tm-dot');
      const c = colorOf(n.botId);
      if (c) dot.style.background = c;
      tree.append(dot);
      const pos = (x, y, extra = {}) => ({ transform: `translate(${x}px, ${y}px)`, ...extra });
      const anim = dot.animate(
        [pos(ax, ay, { opacity: 0.2 }), pos(bx, ay, { offset: 0.3, opacity: 1 }), pos(bx, by, { opacity: 1 })],
        { duration: 600, easing: 'cubic-bezier(0.34, 1.36, 0.64, 1)', fill: 'forwards' },
      );
      const done = () => {
        dot.remove();
        const target = to.closest('.tm-node, .tm-lead');
        target?.classList.add('tm-got');
        setTimeout(() => target?.classList.remove('tm-got'), 500);
      };
      anim.onfinish = done;
      anim.oncancel = () => dot.remove();
    });
  }

  // ---------------------------------------------------------------------------
  // Таймеры: раз в секунду, только пока есть идущий прогон и вкладка видна
  // ---------------------------------------------------------------------------

  let timer = 0;

  function tick() {
    if (document.hidden) return;
    for (const run of runs) {
      if (!isLive(run) || !run.card?.isConnected || !run.visible) continue;
      run.card.querySelector('.tm-foot').textContent = footText(run);
      for (const n of run.nodes) {
        if (n.row && (n.state === 'working' || n.state === 'queued')) n.row.time.textContent = clock(Date.now() - n.startedAt);
      }
    }
    if (blockHost?.isConnected) refreshBlock();
  }

  function syncTick() {
    const live = runs.some(isLive);
    if (live && !timer) timer = setInterval(tick, 1000);
    else if (!live && timer) {
      clearInterval(timer);
      timer = 0;
    }
  }
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) tick();
  });

  // Вернуть живую карточку в чат после перерисовки ленты (переключили бота, перечитали историю).
  function restore() {
    for (const run of runs) {
      if (isLive(run)) paint(run);
    }
  }

  // ---------------------------------------------------------------------------
  // Экран «Команда»: компактное дерево, пока кто-то работает
  // ---------------------------------------------------------------------------

  let blockHost = null;
  let blockQueued = 0;

  function fillBlock() {
    if (!blockHost) return;
    const live = runs.filter(isLive);
    blockHost.hidden = !live.length;
    const parts = [];
    for (const run of live) {
      const card = el('button', 'tm-livecard');
      card.type = 'button';
      card.title = tr('Открыть чат');
      const lead = el('div', 'tm-livelead');
      lead.append(avatarDot(run.leadId, run.leadName), el('span', 'tm-name', run.leadName), el('span', 'tm-livestat', phaseText(run)), el('span', 'tm-time', clock(elapsed(run))));
      card.append(lead);
      const kids = el('div', 'tm-kids');
      for (const n of run.nodes) {
        const node = el('div', 'tm-node');
        node.dataset.state = n.state;
        node.style.setProperty('--tm-depth', String(n.depth - 1));
        const row = el('div', 'tm-row');
        const main = el('span', 'tm-main');
        const slot = el('span', 'tm-ringslot');
        slot.append(ring(n.state, colorOf(n.botId)));
        main.append(avatarDot(n.botId, n.name), el('span', 'tm-name', n.name), slot, el('span', 'tm-step', stepText(n)));
        row.append(main);
        node.append(row);
        kids.append(node);
      }
      card.append(kids);
      card.addEventListener('click', () => hooks.openBot(run.leadId));
      parts.push(card);
    }
    const title = blockHost.querySelector('.tm-livetitle');
    blockHost.replaceChildren(title, ...parts);
  }

  function refreshBlock() {
    if (!blockHost?.isConnected || blockQueued) return;
    blockQueued = setTimeout(() => {
      blockQueued = 0;
      fillBlock();
    }, 200);
  }

  // Блок для renderTeam: null, если команда сейчас не работает.
  function teamBlock() {
    if (!runs.some(isLive)) return null;
    blockHost = el('section', 'tm-live');
    blockHost.append(el('h2', 'tm-livetitle', tr('Сейчас работает команда')));
    fillBlock();
    return blockHost;
  }

  window.DeloistTeam = {
    init(h) { Object.assign(hooks, h); },
    onEvent,
    restore,
    teamBlock,
    // Для проверок: снимок состояния.
    snapshot: () => runs.map((r) => ({ id: r.id, lead: r.leadId, live: isLive(r), nodes: r.nodes.map((n) => ({ bot: n.botId, state: n.state, steps: n.stepTotal })) })),
  };
})();
