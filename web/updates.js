// Карточки обновлений: новая версия Deloist и подключённых программ. Ставится только по нажатию.
window.DeloistUpdates = (() => {
  const KEY = 'deloist-update-dismissed';
  const CLOSE_ICON = DeloistUI.icon('close', { className: 'i' });

  let api = null;
  let toast = () => {};
  let openWhatsNew = null;
  let plaque = null;
  let snapshot = null;
  let checked = false;
  let checking = false;
  const providerCards = new Map();
  const providerSeen = (() => {
    try { return JSON.parse(localStorage.getItem('deloist-provider-update-seen')) || {}; }
    catch { return {}; }
  })();

  function providers(list) {
    for (const p of list) {
      if (!p?.id) continue;
      const id = `provider:${p.id}`;
      if (!('update' in p) && !['installing', 'verifying', 'unpacking', 'node', 'node_verify', 'npm'].includes(p.status)) continue;
      if (!p.update || !p.latest || p.custom || !p.installed || ['installing', 'verifying', 'unpacking', 'node', 'node_verify', 'npm'].includes(p.status)) {
        providerCards.delete(id);
        continue;
      }
      if (providerCards.get(id)?.version === p.latest || providerSeen[p.id] === p.latest) continue;
      providerSeen[p.id] = p.latest;
      try { localStorage.setItem('deloist-provider-update-seen', JSON.stringify(providerSeen)); } catch {}
      providerCards.set(id, {
        id, version: p.latest, title: tr('Вышло обновление {name}', { name: p.name }),
        meta: p.latest, action: tr('Обновить'),
        run: () => { providerCards.delete(id); renderCards(); window.DeloistProviders.update(p.id); },
        close: p.latest,
      });
    }
    renderCards();
  }

  // Хранилище браузера может быть недоступно: тогда просто забудем, что карточку закрывали.
  function dismissed() {
    try {
      return JSON.parse(localStorage.getItem(KEY)) || {};
    } catch {
      return {};
    }
  }

  function dismiss(id, version) {
    providerCards.delete(id);
    const all = dismissed();
    all[id] = version;
    try {
      localStorage.setItem(KEY, JSON.stringify(all));
    } catch {
      // без памяти карточка вернётся при следующем запуске
    }
    render();
  }

  const closable = (id, version) => dismissed()[id] !== version;

  // В карточку идёт только короткая строка: полные заметки (раздел журнала) показывает окно «Что нового».
  const shortNotes = (text) => (text && !text.trim().includes('\n') && text.length <= 200 ? text.trim() : '');

  function appCard(app, current) {
    const version = app.latest || '';
    const range = version ? `${current} → ${version}` : '';
    switch (app.state) {
      case 'available':
        if (!closable('app', version)) return null;
        return { id: 'app', title: tr('Вышла новая версия Deloist'), meta: range, notes: shortNotes(app.notes), note: tr('Программа перезапустится сама.'), action: tr('Обновить'), run: installApp, close: version };
      case 'ready':
        // Версия уже на месте: вместо карточки справа внизу боковой панели стоит плашка с «Перезапустить».
        return null;
      case 'waiting':
        return { id: 'app', title: tr('Новая версия Deloist на подходе'), meta: range, note: tr('Поставлю, как только боты закончат работу.') };
      case 'downloading':
        return { id: 'app', title: tr('Скачиваю версию {version}', { version }), meta: `${app.percent || 0}%`, percent: app.percent || 0 };
      case 'installing':
        return { id: 'app', title: tr('Перезапускаюсь на версии {version}', { version }), busy: true };
      case 'error':
        if (!closable('app', `error:${app.error}`)) return null;
        return { id: 'app', title: tr('Обновление не поставилось'), note: trServer(app.error), action: version ? tr('Ещё раз') : tr('Проверить снова'), run: version ? installApp : check, close: `error:${app.error}` };
      default:
        return null;
    }
  }

  function toolCard(tool) {
    const id = `tool:${tool.id}`;
    const range = `${tool.current} → ${tool.latest}`;
    switch (tool.state) {
      case 'available':
        if (!closable(id, tool.latest)) return null;
        return { id, title: tr('Вышло обновление {name}', { name: tool.name }), meta: range, action: tr('Обновить'), run: () => updateTool(tool.id), close: tool.latest };
      case 'updating':
        return { id, title: tr('Обновляю {name}', { name: tool.name }), meta: range, busy: true };
      case 'error':
        if (!closable(id, `error:${tool.error}`)) return null;
        return { id, title: tr('{name} не обновился', { name: tool.name }), note: trServer(tool.error), action: tr('Ещё раз'), run: () => updateTool(tool.id), close: `error:${tool.error}` };
      default:
        return null;
    }
  }

  function fillCard(el, card) {
    el.replaceChildren();
    const head = document.createElement('div');
    head.className = 'update-head';
    const title = document.createElement('strong');
    title.textContent = card.title;
    head.append(title);
    if (card.close !== undefined) {
      const close = document.createElement('button');
      close.type = 'button';
      close.className = 'icon-btn';
      close.innerHTML = CLOSE_ICON;
      close.setAttribute('aria-label', tr('Скрыть'));
      close.addEventListener('click', () => dismiss(card.id, card.close));
      head.append(close);
    }
    el.append(head);
    if (card.meta) {
      const meta = document.createElement('p');
      meta.className = 'update-meta';
      meta.textContent = card.meta;
      el.append(meta);
    }
    if (card.notes) {
      const notes = document.createElement('p');
      notes.className = 'update-note';
      notes.textContent = card.notes;
      el.append(notes);
    }
    if (card.note) {
      const note = document.createElement('p');
      note.className = 'update-note';
      note.textContent = card.note;
      el.append(note);
    }
    if (card.percent !== undefined || card.busy) {
      const bar = document.createElement('div');
      bar.className = card.busy ? 'update-bar busy' : 'update-bar';
      const fill = document.createElement('span');
      if (!card.busy) fill.style.width = `${card.percent}%`;
      bar.append(fill);
      el.append(bar);
    }
    if (card.action) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'k-btn k-btn--primary';
      button.textContent = card.action;
      button.addEventListener('click', card.run);
      el.append(button);
    }
  }

  function renderCards() {
    const host = document.getElementById('updates');
    if (!host) return;
    const cards = [...(snapshot ? [appCard(snapshot.app, snapshot.current), ...snapshot.tools.map(toolCard)] : []), ...providerCards.values()].filter(Boolean);
    const keep = new Set(cards.map((c) => c.id));
    [...host.children].forEach((el) => { if (!keep.has(el.dataset.id)) el.remove(); });
    for (const card of cards) {
      let el = [...host.children].find((c) => c.dataset.id === card.id);
      if (!el) {
        el = document.createElement('section');
        el.className = 'update-card';
        el.dataset.id = card.id;
        host.append(el);
      }
      fillCard(el, card);
    }
  }

  // Плашка «Deloist X готов»: свою версию программа уже поставила, осталось перезапустить.
  function renderPlaque() {
    const host = document.getElementById('update-plaque');
    if (!host) return;
    const app = snapshot?.app;
    if (app?.state !== 'ready' || !app.latest || !window.DeloistUI?.mountUpdatePlaque) {
      if (plaque) {
        try { plaque.destroy(); } catch {}
        plaque = null;
      }
      host.replaceChildren();
      return;
    }
    const props = { version: app.latest, pending: Boolean(app.restartPending), error: app.error ? trServer(app.error) : '' };
    if (plaque) {
      plaque.update(props);
      return;
    }
    plaque = window.DeloistUI.mountUpdatePlaque(host, { ...props, onrestart: restartApp, onwhatsnew: showNotes });
  }

  // Заметки поставленной версии лежат в подписанном описании: до перезапуска их показывает это окно.
  function showNotes() {
    const app = snapshot?.app;
    if (!app || !openWhatsNew) return;
    const sections = window.DeloistUI.parseNotes(app.notes, app.latest, window.LANG);
    openWhatsNew({ current: app.latest, from: snapshot.current, sections });
  }

  function appLine() {
    const { current, app } = snapshot;
    if (app.state === 'ready') return tr('Версия {latest} установлена, осталось перезапустить', { latest: app.latest });
    if (app.latest && app.state !== 'error') return tr('Версия {current}, вышла {latest}', { current, latest: app.latest });
    if (app.state === 'error') return trServer(app.error);
    return checked ? tr('Версия {current}, это последняя', { current }) : tr('Версия {current}', { current });
  }

  function renderSettings() {
    const line = document.getElementById('upd-app-line');
    if (!line || !snapshot) return;
    line.textContent = appLine();
    const summaryVersion = document.getElementById('upd-summary-version');
    if (summaryVersion) summaryVersion.textContent = snapshot.current;
    const foot = document.getElementById('upd-settings-foot');
    if (foot) foot.textContent = tr('Deloist Боты {version}', { version: snapshot.current });
    const restart = document.getElementById('upd-restart');
    if (restart) {
      restart.hidden = snapshot.app.state !== 'ready';
      restart.disabled = Boolean(snapshot.app.restartPending);
    }
    const button = document.getElementById('upd-check');
    button.disabled = checking;
    button.textContent = checking ? tr('Проверяю…') : tr('Проверить');

    const list = document.getElementById('upd-tools');
    list.replaceChildren();
    if (!snapshot.tools.length) {
      const empty = document.createElement('p');
      empty.className = 'note';
      empty.textContent = tr('Если подключишь программу вроде Claude или OpenCode, её версия появится здесь после проверки.');
      list.append(empty);
      return;
    }
    for (const tool of snapshot.tools) {
      const row = document.createElement('div');
      row.className = 'update-row';
      const text = document.createElement('div');
      const name = document.createElement('strong');
      name.textContent = tool.name;
      const detail = document.createElement('span');
      detail.className = 'update-meta';
      detail.textContent = tool.state === 'available'
        ? tr('Версия {current}, вышла {latest}', { current: tool.current, latest: tool.latest })
        : tool.state === 'error' ? trServer(tool.error) : tr('Версия {current}', { current: tool.current });
      text.append(name, detail);
      row.append(text);
      if (tool.state === 'available' || tool.state === 'error') {
        const update = document.createElement('button');
        update.type = 'button';
        update.className = 'k-btn k-btn--ghost';
        update.textContent = tr('Обновить');
        update.addEventListener('click', () => updateTool(tool.id));
        row.append(update);
      }
      list.append(row);
    }
  }

  function render() {
    renderCards();
    renderPlaque();
    renderSettings();
  }

  function apply(next) {
    // Программа обновилась: карточка уходит, короткое подтверждение остаётся.
    if (snapshot) {
      for (const tool of next.tools) {
        const before = snapshot.tools.find((t) => t.id === tool.id);
        if (before?.state === 'updating' && tool.state === 'current') {
          toast(tr('{name} обновлён до {version}', { name: tool.name, version: tool.current }));
        }
      }
    }
    snapshot = next;
    render();
  }

  async function call(path) {
    try {
      apply(await api(path, { method: 'POST' }));
    } catch (err) {
      toast(err.message, 5000);
    }
  }

  const installApp = () => call('/api/updates/app');
  const restartApp = () => call('/api/updates/restart');
  const updateTool = (id) => call(`/api/updates/tools/${encodeURIComponent(id)}`);

  async function check() {
    if (checking) return;
    checking = true;
    renderSettings();
    await call('/api/updates/check');
    checking = false;
    checked = true;
    renderSettings();
  }

  async function start(deps) {
    api = deps.api;
    toast = deps.toast;
    openWhatsNew = deps.openWhatsNew || null;
    document.getElementById('upd-check').addEventListener('click', check);
    document.getElementById('upd-restart')?.addEventListener('click', restartApp);
    try {
      apply(await api('/api/updates'));
    } catch {
      // без ответа сервера показывать нечего, придёт следующим событием
    }
  }

  return { start, apply, providers };
})();
