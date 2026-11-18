// «Команда за работой»: кто из помощников сейчас что делает. Состояние строится из тех же событий, что и остальной
// интерфейс (delegate_*, tool_*, text_delta, turn_*), своего хранилища нет. Данные идут в двух местах: живое дерево
// на экране «Команда» (teamBlock) и шаги помощника в карточке передачи задачи в чате (detail, её рисует app.js).
// app.js вызывает DeloistTeam.init(hooks) и DeloistTeam.onEvent(ev).
(() => {
  const tr = window.tr || ((text) => text);

  // Что нужно от приложения: бот по id, открыть чат, обновить экран «Команда», подпись шага, «у помощника что-то изменилось».
  const hooks = {
    bot: () => null,
    openBot: () => {},
    refreshTeam: () => {},
    toolLabel: (name) => name,
    changed: () => {},
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
  const elapsed = (run) => (run.endedAt || Date.now()) - run.startedAt;

  function phaseText(run) {
    if (run.endedAt) return tr('Готово');
    const w = working(run);
    if (w) return tr('Работают: {n}', { n: w });
    return run.nodes.length && run.nodes.every((n) => n.state !== 'queued') && run.nodes.some((n) => n.endedAt) ? tr('Собирает итог') : tr('Делит задачу…');
  }

  function startRun(leadId) {
    const lead = hooks.bot(leadId);
    const run = {
      id: ++seq, leadId, leadName: lead?.name || '', startedAt: Date.now(), endedAt: 0,
      nodes: [],
    };
    runs.push(run);
    if (runs.length > 12) runs.shift();
    owner.set(leadId, run);
    return run;
  }

  function endRun(run) {
    if (run.endedAt) return;
    run.endedAt = Date.now();
    for (const n of run.nodes) {
      if (n.state === 'working' || n.state === 'queued') {
        finishNode(n, 'stopped');
        changed(n);
      }
    }
    for (const [id, r] of [...owner]) if (r === run) owner.delete(id);
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
          tokens: 0, summary: '', lead: run.leadId,
        };
        run.nodes.push(node);
        owner.set(ev.to, run);
        changed(node);
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
        finishNode(node, ok ? 'done' : 'failed', ev);
        // Помощник мог стать ведущим для своих: его прогон остаётся тем же, но сам он больше не занят.
        if (!run.nodes.some((n) => n.botId === ev.to && (n.state === 'working' || n.state === 'queued'))) owner.delete(ev.to);
        changed(node);
        refreshBlock();
        return;
      }
      case 'turn_start': {
        const src = String(ev.source || '');
        const node = nodeOf(ev.botId);
        if (node && src.startsWith('delegation:')) {
          node.state = 'working';
          node.cur = tr('Думает');
          changed(node);
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
        changed(node);
        refreshBlock();
        return;
      }
      case 'tool_update': {
        const node = nodeOf(ev.botId);
        if (!node) return;
        const step = node.steps.find((s) => s.callId === ev.callId);
        if (step && ev.status !== 'running') step.status = ev.status === 'ok' ? 'ok' : 'bad';
        changed(node);
        return;
      }
      case 'text_delta': {
        const node = nodeOf(ev.botId);
        if (!node || node.cur === tr('Пишет ответ')) return;
        node.cur = tr('Пишет ответ');
        changed(node);
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
  // Кольцо и точка помощника для экрана «Команда»
  // ---------------------------------------------------------------------------

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

  function stepText(n) {
    if (n.state === 'queued') return tr('Ждёт очереди');
    if (n.state === 'working') return n.cur || tr('Думает');
    if (n.state === 'stopped') return tr('Остановлен');
    if (n.state === 'failed') return n.summary || tr('Не вернул ответ');
    return n.summary || tr('Готово');
  }

  // ---------------------------------------------------------------------------
  // Таймеры: раз в секунду, только пока есть идущий прогон и вкладка видна
  // ---------------------------------------------------------------------------

  let timer = 0;

  function tick() {
    if (!document.hidden && blockHost?.isConnected) refreshBlock();
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

  // Шаги и состояние помощника `toId` в последней передаче от `leadId`: для карточки задачи в чате.
  function detail(leadId, toId) {
    for (let i = runs.length - 1; i >= 0; i -= 1) {
      if (runs[i].leadId !== leadId) continue;
      const node = [...runs[i].nodes].reverse().find((n) => n.botId === toId);
      if (node) return { steps: node.steps, total: node.stepTotal, cur: node.cur };
    }
    return null;
  }

  const changed = (n) => hooks.changed(n.lead, n.botId);

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
    detail,
    teamBlock,
    // Для проверок: снимок состояния.
    snapshot: () => runs.map((r) => ({ id: r.id, lead: r.leadId, live: isLive(r), nodes: r.nodes.map((n) => ({ bot: n.botId, state: n.state, steps: n.stepTotal })) })),
  };
})();
