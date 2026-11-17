// Настройки → «Мой Chrome»: подключить расширение Deloist к своему Chrome или Edge,
// выбрать, какие боты работают там, и остановить всё. Состояние живое, пока раздел открыт.
(function () {
  const tr = (text, vars) => (window.tr ? window.tr(text, vars) : text);
  let timer = 0;
  let state = null;
  let busy = false;
  let pairingCode = null;

  async function call(path, method = 'GET', body) {
    const res = await fetch(path, {
      method,
      headers: body ? { 'content-type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
    return data;
  }

  function el(tag, attrs = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs)) {
      if (key === 'class') node.className = value;
      else if (key === 'text') node.textContent = value;
      else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
      else if (value !== false && value !== undefined) node.setAttribute(key, value === true ? '' : value);
    }
    for (const child of children) if (child) node.append(child);
    return node;
  }

  function say(message) {
    const line = document.getElementById('user-chrome-msg');
    if (line) line.textContent = message || '';
  }

  async function act(path, method, body) {
    if (busy) return;
    busy = true;
    try {
      state = await call(path, method, body);
      say('');
    } catch (e) {
      say(tr('Не получилось: {error}', { error: window.trServer ? window.trServer(e.message) : e.message }));
    } finally {
      busy = false;
      render();
    }
  }

  function statusLine(st) {
    if (st.connected) {
      const link = st.links[st.links.length - 1] || {};
      const name = link.browser === 'edge' ? 'Edge' : 'Chrome';
      return tr('Подключено: {browser}, расширение {version}', { browser: name, version: link.version || '?' });
    }
    if (st.links.some((l) => l.peerUnpaired)) return tr('Не связано: расширение не связано с приложением');
    if (st.waitingForPairing) return tr('Расширение пришло и ждёт связи: введи код в его окне');
    if (st.registered) return tr('Программа готова, ждём расширение в браузере');
    return tr('Не подключено');
  }

  function steps(st) {
    const list = el('ol', { class: 'user-chrome-steps' });
    const browser = st.browsers.find((b) => b.id === 'chrome') || st.browsers[0];
    const page = browser && browser.id === 'edge' ? 'edge://extensions' : 'chrome://extensions';
    list.append(
      el('li', { text: tr('Открой в браузере адрес {page} (скопируй его в адресную строку).', { page }) }),
      el('li', { text: tr('Включи «Режим разработчика» — переключатель вверху справа (в Edge — слева внизу).') }),
      el('li', { text: tr('Нажми «Загрузить распакованное расширение» и выбери папку:') }),
    );
    const folder = el('div', { class: 'update-row' },
      el('code', { class: 'user-chrome-path', text: st.extensionDir }),
      el('button', { type: 'button', class: 'k-btn k-btn--ghost', text: tr('Открыть папку'), onclick: () => act('/api/user-chrome/folder', 'POST') }),
    );
    list.append(folder);
    list.append(el('li', { text: tr('Закрепи значок Deloist на панели браузера: там видно, кто работает, и есть «Остановить всё».') }));
    return list;
  }

  function botRow(bot) {
    const mode = el('select', {
      class: 's-select',
      onchange: (event) => {
        const value = event.target.value;
        if (value === 'mine' && !confirm(tr('Бот {name} будет работать в твоём Chrome с твоими входами', { name: bot.name }))) {
          event.target.value = bot.mode || '';
          return;
        }
        act(`/api/user-chrome/bots/${encodeURIComponent(bot.id)}`, 'PUT', { mode: value });
      },
    },
      el('option', { value: '', text: tr('Как по умолчанию') }),
      el('option', { value: 'own', text: tr('Свой отдельный браузер') }),
      el('option', { value: 'mine', text: tr('Мой основной Chrome') }),
    );
    mode.value = bot.mode || '';
    const unattended = el('input', {
      type: 'checkbox',
      onchange: (event) => {
        if (event.target.checked && bot.mine && !confirm(tr('Бот {name} будет работать в твоём Chrome без тебя, с твоими входами', { name: bot.name }))) {
          event.target.checked = false;
          return;
        }
        act(`/api/user-chrome/bots/${encodeURIComponent(bot.id)}`, 'PUT', { unattended: event.target.checked });
      },
    });
    unattended.checked = !!bot.unattended;
    unattended.disabled = !bot.mine;
    const sites = bot.sites && bot.sites.length ? tr('Сайты «всегда»: {sites}', { sites: bot.sites.join(', ') }) : '';
    return el('div', { class: 'user-chrome-bot' },
      el('div', { class: 'update-row' },
        el('div', {},
          el('strong', { text: bot.name }),
          el('span', { class: 'update-meta', text: bot.active ? tr('работает в твоём Chrome') : bot.mine ? tr('в твоём Chrome') : tr('в своём браузере') }),
        ),
        mode,
      ),
      el('label', { class: 'quiet-check' }, unattended, el('span', { text: tr('Можно без меня: рутины и ночная смена, только на сайтах «всегда»') })),
      sites ? el('p', { class: 'note', text: sites }) : null,
    );
  }

  function render() {
    const body = document.getElementById('user-chrome-body');
    if (!body) return;
    const st = state;
    body.replaceChildren();
    body.append(el('p', { class: 'note', text: tr('Бот работает в твоём настоящем Chrome или Edge: с твоими входами, cookies и адресом. Сайты вроде seller.ozon.ru видят обычный браузер, потому что это он и есть. Бот открывает только свои вкладки в группе «Deloist · имя бота», каждый сайт — с твоего разрешения. Пароли и данные карт вводишь ты.') }));
    if (!st) {
      body.append(el('p', { class: 'note', text: tr('Загружаю…') }));
      return;
    }
    body.prepend(el('p', { class: 'note', text: tr('Для кабинетов это не нужно: браузер бота — и так настоящий Chrome со своим профилем, где ты один раз входишь сам. Этот режим — чтобы бот работал прямо в твоём повседневном профиле.') }));
    const head = el('div', { class: 'update-row' },
      el('div', {}, el('strong', { text: tr('Расширение Deloist') }), el('span', { class: 'update-meta', text: statusLine(st) })),
      st.registered
        ? el('button', { type: 'button', class: 'k-btn k-btn--ghost', text: tr('Отключить'), onclick: () => act('/api/user-chrome/disconnect', 'POST') })
        : el('button', { type: 'button', class: 'k-btn k-btn--primary', text: tr('Подключить'), disabled: !st.browsers.length, onclick: () => act('/api/user-chrome/connect', 'POST') }),
    );
    body.append(head);
    const pairBlock = el('div', { class: 'update-row' },
      st.paired
        ? el('button', { type: 'button', class: 'k-btn k-btn--ghost', text: tr('Отвязать'), onclick: async () => { pairingCode = null; await act('/api/user-chrome/unpair', 'POST'); } })
        : el('button', { type: 'button', class: 'k-btn k-btn--primary', text: tr('Связать с расширением'), onclick: async () => {
            if (busy) return;
            busy = true;
            try { const data = await call('/api/user-chrome/pairing', 'POST'); pairingCode = data.code; say(''); }
            catch (e) { say(tr('Не получилось: {error}', { error: window.trServer ? window.trServer(e.message) : e.message })); }
            finally { busy = false; render(); }
          } }),
    );
    body.append(pairBlock);
    if (pairingCode) {
      body.append(el('p', { class: 'note', text: tr('Код для расширения (скопируй один раз):') }));
      body.append(el('div', { class: 'update-row' },
        el('code', { class: 'user-chrome-path', text: pairingCode }),
        el('button', { type: 'button', class: 'k-btn k-btn--ghost', text: tr('Скопировать'), onclick: async () => { try { await navigator.clipboard.writeText(pairingCode); } catch {} } }),
      ));
    }
    if (!st.browsers.length) body.append(el('p', { class: 'note', text: tr('Не нашёл Chrome или Edge на этом компьютере.') }));
    if (st.registered && !st.connected) body.append(steps(st));
    body.append(el('p', { class: 'note', id: 'user-chrome-msg' }));

    const fallback = el('select', { onchange: (event) => act('/api/user-chrome/settings', 'PUT', { default: event.target.value }) },
      el('option', { value: 'own', text: tr('Свой отдельный браузер') }),
    );
    fallback.value = st.default;
    body.append(el('label', { class: 'field mid' }, el('span', { text: tr('Где боты работают по умолчанию') }), fallback));
    for (const bot of st.bots) body.append(botRow(bot));
    body.append(el('button', { type: 'button', class: 'k-btn k-btn--danger', text: tr('Остановить всё'), onclick: () => act('/api/user-chrome/stop', 'POST') }));
    body.append(el('p', { class: 'note', text: tr('«Остановить всё» сразу отключает ботов от всех вкладок твоего браузера. Та же кнопка есть в окне расширения, а Chrome показывает полосу «Deloist отлаживает этот браузер», пока бот подключён.') }));
  }

  async function refresh() {
    clearTimeout(timer);
    const group = document.getElementById('set-user-chrome');
    if (!group || !group.open) return;
    if (!busy) {
      try {
        state = await call('/api/user-chrome');
        render();
      } catch (e) {
        say(e.message);
      }
    }
    timer = setTimeout(refresh, 2000);
  }

  document.addEventListener('DOMContentLoaded', () => {
    const group = document.getElementById('set-user-chrome');
    if (!group) return;
    render();
    group.addEventListener('toggle', refresh);
  });
})();
