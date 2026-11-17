// Окно «Подключить свою ИИ»: агенты из реестра ACP (ChatGPT, Claude, Gemini…) и провайдеры по ключу.
// Слева список со статусами, справа карточка выбранного: вход, программа, модели, использование.
window.DeloistProviders = (() => {
  let api = null;
  let toast = () => {};
  let onChanged = () => {};
  let items = [];
  let selected = '';
  let query = '';
  let searchTimer = 0;
  let modelFilter = '';
  let listView = null;
  const ADD = '__add';

  const el = (id) => document.getElementById(id);
  const esc = (text) => String(text ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

  const STATUS = {
    not_installed: 'Не загружено',
    installing: 'Загрузка',
    checking: 'Настройка',
    connecting: 'Вход',
    verifying: 'Проверка подписи',
    unpacking: 'Распаковка',
    node: 'Скачиваю Node.js (один раз)…',
    node_verify: 'Проверяю подпись…',
    npm: 'Ставлю агента…',
    not_connected: 'Не подключено',
    connected: 'Подключено',
    error: 'Ошибка',
  };

  function badge(p) {
    if (p.status === 'installing' && p.progress?.percent != null) return `${p.progress.percent} %`;
    return tr(STATUS[p.status] || p.status);
  }

  // Значок агента из реестра (через наш адрес) в цвет темы; нет значка — первая буква.
  function iconColor() {
    const css = getComputedStyle(document.documentElement).getPropertyValue('--color-text-primary').trim();
    const hex = /^#([0-9a-f]{6})$/i.exec(css);
    return hex ? hex[1] : '151414';
  }

  function monogram(p) {
    const letter = (p.name || p.id || '?').trim().charAt(0).toUpperCase();
    const svg = window.DeloistUI?.providerIconSvg?.(p);
    if (svg) return `<span class="prov-icon" aria-hidden="true">${svg}</span>`;
    if (p.kind === 'acp' && p.icon) {
      return `<span class="prov-icon" aria-hidden="true"><img src="/api/providers/${encodeURIComponent(p.id)}/icon?color=${iconColor()}" alt="" data-letter="${esc(letter)}"></span>`;
    }
    return `<span class="prov-icon" aria-hidden="true">${esc(letter)}</span>`;
  }

  function usedBy(p) {
    const n = Number(p.usedBy || 0);
    return n > 0 ? tr('Используют ботов: {n}', { n }) : tr('Пока ни один бот не использует');
  }

  // -------------------------------------------------------------------------
  // Список
  // -------------------------------------------------------------------------

  async function load() {
    if (!api) return;
    try {
      const data = await api(`/api/providers${query ? `?q=${encodeURIComponent(query)}` : ''}`);
      // Записи «Gemini · аккаунт» нужны только выбору модели бота, в списке слева их нет.
      items = (data.providers || []).filter((p) => !p.pickerOnly);
      window.DeloistUpdates?.providers(items);
      el('prov-total').textContent = query ? tr('Найдено: {n}', { n: items.length }) : tr('В реестре агентов: {n}', { n: data.total || 0 });
    } catch (err) {
      items = [];
      listView?.update({ groups: [{ key: 'error', note: trServer(err.message), rows: [] }], selected });
      return;
    }
    if (selected !== ADD && (!selected || !items.some((p) => p.id === selected))) selected = items[0]?.id || '';
    renderList();
    renderDetail();
    viewTitle();
  }

  // На этом компьютере: найденные серверы моделей и скачанные агенты. Остальное — ниже.
  function isFound(p) {
    return p.kind === 'acp' ? Boolean(p.installed) : Boolean(p.local && (p.found || p.status === 'connected'));
  }

  function tone(p) {
    if (p.status === 'connected') return 'brand';
    if (p.status === 'not_installed' || p.status === 'error' || p.update) return 'attention';
    return 'neutral';
  }

  // Строка списка для компонента ProviderList: логотип кита, иначе значок из реестра, иначе буква.
  function row(p) {
    const letter = (p.name || p.id || '?').trim().charAt(0).toUpperCase();
    const svg = window.DeloistUI?.providerIconSvg?.(p) || '';
    const img = !svg && p.kind === 'acp' && p.icon ? `/api/providers/${encodeURIComponent(p.id)}/icon?color=${iconColor()}` : '';
    return { id: p.id, name: p.name, badge: badge(p), tone: tone(p), svg, img, letter };
  }

  function listGroups() {
    const add = { id: ADD, name: tr('Добавить своё'), badge: '', tone: 'neutral', svg: '', img: '', letter: '+', plus: true };
    if (!items.length) return [{ key: 'none', note: tr('Ничего не нашлось'), rows: [add] }];
    if (query) return [{ key: 'found', rows: [...items.map(row), add] }];
    const found = items.filter(isFound);
    const rest = items.filter((p) => !isFound(p));
    return [
      {
        key: 'local',
        title: tr('Найдено на этом компьютере'),
        action: { name: 'scan', label: tr('Сканировать снова') },
        note: found.length ? '' : tr('Пока ничего: скачай агента или запусти Ollama или LM Studio'),
        rows: found.map(row),
      },
      { key: 'other', title: tr('Другие провайдеры'), rows: [...rest.map(row), add] },
    ];
  }

  function renderList() {
    listView?.update({ groups: listGroups(), selected });
  }

  // Раздел «Модели и подключения» показывает либо список, либо карточку выбранного (как подэкран с кнопкой «Назад»).
  let screen = 'list';

  function viewTitle() {
    const title = el('settings-title');
    if (!title || screen !== 'detail') return;
    if (selected === ADD) title.textContent = tr('Добавить своё');
    else title.textContent = items.find((item) => item.id === selected)?.name || tr('Модели и подключения');
  }

  function setView(next) {
    screen = next;
    const section = document.querySelector('[data-settings-section="providers"]');
    if (section) section.dataset.view = next;
    const back = el('settings-back');
    if (back) back.hidden = next !== 'detail';
    if (next === 'detail') viewTitle();
    else if (el('settings-title')) el('settings-title').textContent = tr('Модели и подключения');
    const body = el('settings-body');
    if (body) body.scrollTop = 0;
  }

  function pickRow(id) {
    selected = id;
    modelFilter = '';
    renderList();
    renderDetail();
    setView('detail');
  }

  // -------------------------------------------------------------------------
  // Карточка
  // -------------------------------------------------------------------------

  function section(title, body) {
    return `<section class="prov-section"><h4>${title}</h4><div class="prov-box">${body}</div></section>`;
  }

  function line(title, sub, action = '') {
    return `<div class="prov-line"><div><div class="prov-line-title">${title}</div>${sub ? `<div class="prov-line-sub">${sub}</div>` : ''}</div>${action}</div>`;
  }

  function button(action, label, kind = 'ghost', disabled = false) {
    return `<button type="button" class="k-btn k-btn--${kind}" data-act="${action}"${disabled ? ' disabled' : ''}>${label}</button>`;
  }

  function isFreeModel(m) { return m?.free === true || /:free$/i.test(String(m?.id || '')); }
  function models(p) {
    const all = (p.models || []).slice().sort((a, b) => Number(isFreeModel(b)) - Number(isFreeModel(a)));
    if (!all.length) return '';
    const hidden = new Set(p.hiddenModels || []);
    const visible = all.filter((m) => !hidden.has(m.id)).length;
    const shown = modelFilter ? all.filter((m) => `${m.name} ${m.id}`.toLowerCase().includes(modelFilter)) : all;
    const rows = shown.map((m) => `
      <label class="prov-model">
        <span><b>${esc(m.name)}</b> <code>${esc(m.id)}</code>${isFreeModel(m) ? ` <span class="prov-badge" data-status="free">${tr('Бесплатно')}</span>` : ''}</span>
        <span class="k-switch-box"><input type="checkbox" data-model="${esc(m.id)}"${hidden.has(m.id) ? '' : ' checked'} /><span></span></span>
      </label>`).join('');
    const allVisible = visible === all.length;
    return `<section class="prov-section">
      <div class="prov-head-row"><h4>${tr('Модели')}</h4><button type="button" class="link" data-act="${allVisible ? 'hide-all' : 'show-all'}">${allVisible ? tr('Скрыть все') : tr('Показать все')}</button></div>
      <p class="prov-line-sub">${tr('{n} из {m} моделей видно в выборе модели.', { n: visible, m: all.length })}</p>
      <input type="search" class="prov-model-search" id="prov-model-search" placeholder="${tr('Поиск моделей')}" value="${esc(modelFilter)}" />
      <div class="prov-box">${rows || `<p class="note">${tr('Ничего не нашлось')}</p>`}</div>
    </section>`;
  }

  function use(p) {
    const ready = p.status === 'connected';
    const name = esc(p.name);
    const defaultSub = p.isDefault ? tr('Новые боты работают через {name}.', { name }) : (ready ? tr('Новые боты будут работать через {name}.', { name }) : tr('Сначала подключи {name}.', { name }));
    return section(tr('Использование'),
      line(tr('По умолчанию для новых ботов'), defaultSub, p.isDefault ? `<span class="prov-done">${tr('Основной')}</span>` : button('default', tr('Сделать основным'), 'ghost', !ready))
      + `<label class="prov-line"><div><div class="prov-line-title">${tr('Использовать {name}', { name })}</div><div class="prov-line-sub">${tr('Показывать в выборе модели у ботов')}</div></div><span class="k-switch-box"><input type="checkbox" data-act="enabled"${p.enabled ? ' checked' : ''} /><span></span></span></label>`);
  }

  function addressStatus(p) {
    if (p.status === 'connected') return line(tr('Подключено'), p.local ? tr('Сервер отвечает, модели видны') : tr('Ключ подошёл'), button('verify', tr('Проверить снова')));
    if (p.status === 'error') return line(tr('Ошибка'), `<span class="prov-warn">${esc(trServer(p.error || ''))}</span>`, button('verify', tr('Проверить снова')));
    if (p.local) return line(tr('Не найдено'), tr('Запусти {name} на этом компьютере и нажми «Сканировать снова».', { name: esc(p.name) }), button('scan', tr('Сканировать снова')));
    return line(tr('Не подключено'), tr('Впиши ключ ниже и нажми «Сохранить».'));
  }

  /// «Начать бесплатно»: вход в OpenRouter в браузере вместо ручного ключа. Ключ приходит сразу в хранилище программы.
  function freeStart(p) {
    if (p.signingIn) {
      return `<div class="prov-free"><p class="prov-line-sub">${tr('Жду входа в браузере…')}</p>${button('free-cancel', tr('Отмена'))}</div>`;
    }
    return `<div class="prov-free"><button type="button" class="k-btn k-btn--primary" data-act="free-start">${tr('Начать бесплатно')}</button>`
      + `<p class="prov-line-sub">${tr('Откроется браузер: войдите в OpenRouter, ключ придёт сам. Бесплатные модели ограничены: до 20 запросов в минуту и 50 в день, пока нет пополнений; условия обработки запросов у каждой модели свои, секретное не отправляйте.')}</p></div>`;
  }

  function addressCard(p) {
    const fields = [];
    if (p.custom || (p.local && p.found === false)) {
      fields.push(`<label class="field"><span>${tr('Адрес API')}</span><input id="prov-url" value="${esc(p.baseUrl || '')}" placeholder="http://127.0.0.1:8000/v1" /></label>`);
    }
    if (p.needsKey) {
      fields.push(`<label class="field"><span>${esc(tr(p.keyLabel || 'Ключ API'))}</span><input type="password" id="prov-key" placeholder="${p.hasKey ? tr('ключ сохранён') : ''}" autocomplete="off" /></label>`);
    }
    if (p.needsFolder) {
      fields.push(`<label class="field"><span>${tr('Идентификатор каталога Yandex Cloud')}</span><input id="prov-folder" value="${esc(p.folder || '')}" placeholder="b1g…" /></label>`);
    }
    const options = (p.models || []).map((m) => `<option value="${esc(m.id)}"></option>`).join('');
    fields.push(`<label class="field"><span>${tr('Модель по умолчанию')}</span><input id="prov-key-model" list="prov-model-list" value="${esc(p.model || p.defaultModel || '')}" /><datalist id="prov-model-list">${options}</datalist></label>`);
    if (p.keyUrl) {
      const link = `<a href="${esc(p.keyUrl)}" target="_blank" rel="noopener">${tr('Где взять ключ')}</a>`;
      fields.unshift(`<div class="prov-key-guide"><p class="prov-line-sub">${link}</p><ol class="prov-guide"><li>${tr('Войдите на сайт {name} и авторизуйтесь.', { name: esc(p.name) })}</li><li>${tr('Создайте ключ API.')}</li><li>${tr('Вставьте его сюда и нажмите «Сохранить».')}</li></ol></div>`);
    }
    const free = p.id === 'openrouter' && p.status !== 'connected' ? freeStart(p) : '';
    const where = p.local
      ? tr('Модели работают на этом компьютере, без интернета и без оплаты.')
      : tr('Адрес API: {url}. Провайдер списывает деньги за каждый запрос со своего счёта.', { url: `<code>${esc(p.baseUrl)}</code>` });
    return section(tr('Подключение'), addressStatus(p) + free)
      + section(p.needsKey ? tr('Ключ и модель') : tr('Модель'),
        `${fields.join('')}<p class="prov-line-sub">${where}</p>
         <div class="modal-actions">${p.custom ? button('remove', tr('Удалить')) : (p.hasKey ? button('forget', tr('Забыть ключ')) : '')}<button type="button" class="k-btn k-btn--primary" data-act="key-save">${tr('Сохранить')}</button></div>`);
  }

  // «Добавить своё»: Ollama, LM Studio, OpenAI-совместимый адрес или свой агент ACP.
  const ADD_KINDS = [
    { kind: 'ollama', title: 'Ollama', sub: '127.0.0.1:11434', url: 'http://127.0.0.1:11434/v1' },
    { kind: 'lmstudio', title: 'LM Studio', sub: '127.0.0.1:1234', url: 'http://127.0.0.1:1234/v1' },
    { kind: 'openai', title: 'OpenAI-совместимый адрес', sub: 'vLLM, LocalAI, свой сервер, другие облака' },
    { kind: 'acp', title: 'Свой агент ACP', sub: 'Cursor, Copilot CLI, Qwen Code, Goose и другие' },
  ];
  let addKind = 'openai';

  function addPane() {
    const kinds = ADD_KINDS.map((k) => `
      <button type="button" class="prov-kind${addKind === k.kind ? ' active' : ''}" data-kind="${k.kind}">
        <b>${tr(k.title)}</b><span>${tr(k.sub)}</span>
      </button>`).join('');
    const kind = ADD_KINDS.find((k) => k.kind === addKind);
    let form = '';
    if (addKind === 'acp') {
      form = `<label class="field"><span>${tr('Название')}</span><input id="add-name" placeholder="Goose" /></label>
        <label class="field"><span>${tr('Команда запуска')}</span><input id="add-command" placeholder="goose acp" /></label>
        <p class="prov-line-sub">${tr('Программа должна говорить по протоколу ACP через stdin и stdout. Она запускается в папке бота.')}</p>`;
    } else {
      form = `<label class="field"><span>${tr('Название')}</span><input id="add-name" value="${esc(addKind === 'openai' ? '' : kind.title)}" placeholder="${tr('Мой сервер')}" /></label>
        <label class="field"><span>${tr('Адрес API')}</span><input id="add-url" value="${esc(kind.url || '')}" placeholder="http://127.0.0.1:8000/v1" /></label>
        <label class="field"><span>${tr('Ключ API, если нужен')}</span><input type="password" id="add-key" autocomplete="off" /></label>
        <label class="field"><span>${tr('Модель')}</span><input id="add-model" placeholder="${tr('можно выбрать потом')}" /></label>`;
    }
    const servers = (window.DeloistSettings?.modelServers || []).join('\n');
    return `<header class="prov-head"><span class="prov-icon" aria-hidden="true">+</span><div><div class="prov-title">${tr('Добавить своё')}</div><div class="prov-line-sub">${tr('Локальная модель, свой сервер или свой агент')}</div></div></header>
      <div class="prov-kinds">${kinds}</div>
      ${section(tr(kind.title), `<div class="prov-form">${form}<div class="modal-actions"><button type="button" class="k-btn k-btn--primary" data-act="add-create">${tr('Добавить')}</button></div></div>`)}
      ${section(tr('Другие адреса серверов моделей'), `<div class="prov-form"><label class="field"><span>${tr('По одному в строке. Их проверяет «Сканировать снова».')}</span><textarea id="add-servers" rows="3" placeholder="http://192.168.1.10:11434/v1">${esc(servers)}</textarea></label><div class="modal-actions">${button('servers-save', tr('Сохранить адреса'))}</div></div>`)}`;
  }

  // Карточка агента — компонент кита: живёт, пока выбран тот же агент, и обновляется без пересоздания.
  let agentCard = null;
  let agentCardId = '';
  function dropAgent() {
    agentCard?.destroy();
    agentCard = null;
    agentCardId = '';
  }

  // Логотип карточки: как в списке слева (кит, иначе значок из реестра, иначе буква).
  function cardLogo(p) {
    const r = row(p);
    return { svg: r.svg, img: r.img };
  }

  function renderDetail() {
    const box = el('prov-detail');
    const p = selected === ADD ? null : items.find((item) => item.id === selected);
    if (p?.kind === 'acp' && window.DeloistUI?.mountAgentCard) {
      if (agentCard && agentCardId === p.id && box.firstElementChild?.dataset.agent === '1') {
        agentCard.update({ provider: p, logo: cardLogo(p) });
        return;
      }
      dropAgent();
      box.innerHTML = '<div data-agent="1"></div>';
      agentCardId = p.id;
      agentCard = window.DeloistUI.mountAgentCard(box.firstElementChild, { provider: p, logo: cardLogo(p), onaction: (name, arg) => agentAction(p.id, name, arg) });
      return;
    }
    dropAgent();
    if (selected === ADD) {
      box.innerHTML = addPane();
      return;
    }
    if (!p) {
      box.innerHTML = '';
      return;
    }
    const about = [p.description ? esc(p.description) : '', p.website ? `<a href="${esc(p.website)}" target="_blank" rel="noopener">${esc(p.website.replace(/^https?:\/\//, ''))}</a>` : ''].filter(Boolean).join(' · ');
    const head = `<header class="prov-head">${monogram(p)}<div><div class="prov-title">${esc(p.name)} <span class="prov-badge" data-status="${esc(p.status)}">${esc(badge(p))}</span>${p.free ? ` <span class="prov-badge" data-status="free">${tr('Бесплатно')}</span>` : ''}${p.update ? ` <span class="prov-badge" data-status="update">${tr('Есть обновление')}</span>` : ''}</div><div class="prov-line-sub">${usedBy(p)}</div>${about ? `<div class="prov-line-sub">${about}</div>` : ''}</div></header>`;
    const easier = p.id === 'gemini' && p.kind === 'key'
      ? `<p class="prov-line-sub"><button type="button" class="link" data-act="open-gemini">${tr('Проще: войти через Google одной кнопкой')}</button></p>`
      : '';
    box.innerHTML = head + easier + addressCard(p) + models(p) + use(p);
  }

  // -------------------------------------------------------------------------
  // Действия
  // -------------------------------------------------------------------------

  function replace(view) {
    if (!view?.id) return;
    const index = items.findIndex((p) => p.id === view.id);
    if (index >= 0) items[index] = { ...items[index], ...view };
    else if (!query) items.push(view);
    renderList();
    if (view.id === selected) renderDetail();
  }

  async function removeProvider(p) {
    const question = p.custom
      ? tr('Удалить {name} из списка?', { name: p.name })
      : tr('Удалить скачанного агента {name}? Вход в его аккаунт останется на компьютере.', { name: p.name });
    if (!confirm(question)) return;
    const result = await api(`/api/providers/${encodeURIComponent(p.id)}`, { method: 'DELETE' });
    if (result.removed) {
      selected = '';
      await load();
      setView('list');
      onChanged();
    } else {
      replace(result);
    }
  }

  async function act(action, target) {
    const p = items.find((item) => item.id === selected);
    if (!p) return;
    const id = encodeURIComponent(p.id);
    try {
      switch (action) {
        case 'remove': await removeProvider(p); break;
        case 'forget':
          if (!confirm(tr('Забыть ключ {name}?', { name: p.name }))) return;
          replace(await api(`/api/providers/${id}`, { method: 'DELETE' }));
          onChanged();
          break;
        case 'verify':
          replace(await api(`/api/providers/${id}`, { method: 'PATCH', body: { verify: true } }));
          break;
        case 'default':
          replace(await api(`/api/providers/${id}`, { method: 'PATCH', body: { default: true } }));
          await load();
          await onChanged({ defaultChanged: true });
          toast(tr('Новые боты будут работать через {name}', { name: p.name }));
          break;
        case 'enabled':
          replace(await api(`/api/providers/${id}`, { method: 'PATCH', body: { enabled: target.checked } }));
          onChanged();
          break;
        case 'hide-all':
        case 'show-all': {
          const hiddenModels = action === 'hide-all' ? (p.models || []).map((m) => m.id) : [];
          replace(await api(`/api/providers/${id}`, { method: 'PATCH', body: { hiddenModels } }));
          onChanged();
          break;
        }
        case 'free-start':
          await api(`/api/providers/${id}/connect`, { method: 'POST' });
          replace(await api(`/api/providers/${id}`));
          break;
        case 'free-cancel':
          await api(`/api/providers/${id}/cancel`, { method: 'POST' });
          replace(await api(`/api/providers/${id}`));
          break;
        case 'key-save': {
          const body = { model: el('prov-key-model').value.trim(), verify: true };
          const key = el('prov-key')?.value.trim();
          if (key) body.apiKey = key;
          if (p.needsKey && !key && !p.hasKey) {
            toast(tr('Впиши ключ'));
            return;
          }
          if (el('prov-folder')) body.folder = el('prov-folder').value.trim();
          if (el('prov-url')) body.baseUrl = el('prov-url').value.trim();
          const view = await api(`/api/providers/${id}`, { method: 'PATCH', body });
          replace(view);
          onChanged();
          toast(view.error ? trServer(view.error) : view.status === 'connected' ? tr('Готово: {name} подключён', { name: p.name }) : tr('Не получилось подключить'), 5000);
          break;
        }
        default: break;
      }
    } catch (err) {
      toast(trServer(err.message), 6000);
    }
  }

  /// Действия карточки агента `id`. Адреса — под /api/providers/{id}, ключ программы их не достанет.
  /// Аккаунты, ключ, ссылка входа, выход и проверка обновлений есть на стороне программы только у Gemini (antigravity).
  async function agentAction(id, name, arg = {}) {
    const base = `/api/providers/${encodeURIComponent(id)}`;
    const current = () => items.find((item) => item.id === id);
    const post = (path, method = 'POST', body) => api(`${base}${path}`, { method, ...(body ? { body } : {}) });
    try {
      switch (name) {
        case 'install': replace(await post('/install')); break;
        case 'connect': replace(await post('/connect')); break;
        case 'check': replace(await post('/check')); break;
        case 'cancel': await post('/cancel'); break;
        case 'refresh': {
          const view = await post('/refresh');
          replace(view);
          toast(view.update ? tr('Есть обновление: {v}', { v: view.latest }) : tr('Установлена последняя версия'));
          break;
        }
        case 'signin-open':
        case 'signin-copy': {
          // Ссылку отдаёт отдельный адрес только окну; в разметку и в список она не попадает.
          const { link } = await api(`${base}/signin-link`);
          if (name === 'signin-open') window.open(link, '_blank', 'noopener');
          else {
            await navigator.clipboard.writeText(link);
            toast(tr('Ссылка скопирована'));
          }
          break;
        }
        case 'account-add': replace(await post('/accounts')); break;
        case 'account-remove':
          if (!confirm(tr('Выйти из этого аккаунта? Его вход на компьютере будет стёрт.'))) return;
          replace(await post(`/accounts/${encodeURIComponent(arg.id)}`, 'DELETE'));
          onChanged();
          break;
        case 'key-save': replace(await post('/key', 'PUT', { apiKey: arg.apiKey })); onChanged(); break;
        case 'key-remove': replace(await post('/key', 'DELETE')); onChanged(); break;
        case 'disconnect':
          if (!confirm(tr('Отключить Gemini? Вход во все аккаунты будет стёрт, программа останется.'))) return;
          replace(await post('/logout'));
          onChanged();
          break;
        case 'default':
          replace(await api(base, { method: 'PATCH', body: { default: true } }));
          await load();
          await onChanged({ defaultChanged: true });
          toast(tr('Новые боты будут работать через {name}', { name: current()?.name || id }));
          break;
        case 'models-all':
          replace(await api(base, { method: 'PATCH', body: { hiddenModels: arg.hide ? (current()?.models || []).map((m) => m.id) : [] } }));
          onChanged();
          break;
        case 'enabled':
          replace(await api(base, { method: 'PATCH', body: { enabled: arg.on } }));
          onChanged();
          break;
        case 'remove': {
          const p = current();
          if (p) await removeProvider(p);
          break;
        }
        case 'model': {
          const hidden = new Set(current()?.hiddenModels || []);
          if (arg.on) hidden.delete(arg.id);
          else hidden.add(arg.id);
          replace(await api(base, { method: 'PATCH', body: { hiddenModels: [...hidden] } }));
          onChanged();
          break;
        }
        default: break;
      }
    } catch (err) {
      toast(trServer(err.message), 6000);
    }
  }

  async function toggleModel(input) {
    const p = items.find((item) => item.id === selected);
    if (!p) return;
    const hidden = new Set(p.hiddenModels || []);
    if (input.checked) hidden.delete(input.dataset.model);
    else hidden.add(input.dataset.model);
    try {
      replace(await api(`/api/providers/${encodeURIComponent(p.id)}`, { method: 'PATCH', body: { hiddenModels: [...hidden] } }));
      onChanged();
    } catch (err) {
      toast(trServer(err.message));
    }
  }

  async function scan() {
    try {
      const result = await api('/api/providers/scan', { method: 'POST' });
      await load();
      const n = (result.found || []).length;
      toast(n ? tr('Найдено серверов моделей: {n}', { n }) : tr('Серверы моделей на этом компьютере не отвечают'));
    } catch (err) {
      toast(trServer(err.message));
    }
  }

  async function createOwn() {
    const name = el('add-name')?.value.trim();
    try {
      let view;
      if (addKind === 'acp') {
        view = await api('/api/providers', { method: 'POST', body: { kind: 'acp', name, command: el('add-command').value.trim() } });
        await api(`/api/providers/${encodeURIComponent(view.id)}/check`, { method: 'POST' }).catch(() => {});
      } else {
        const url = el('add-url').value.trim();
        const builtIn = ADD_KINDS.find((k) => k.kind === addKind && k.url === url);
        if (builtIn) {
          // Обычный адрес Ollama или LM Studio: их ищет сканирование, отдельная запись не нужна.
          await scan();
          selected = addKind;
          await load();
          return;
        }
        view = await api('/api/providers', { method: 'POST', body: { kind: 'openai', name, baseUrl: url, apiKey: el('add-key').value.trim(), model: el('add-model').value.trim() } });
      }
      selected = view.id;
      await load();
      onChanged();
    } catch (err) {
      toast(trServer(err.message), 6000);
    }
  }

  async function saveServers() {
    const modelServers = el('add-servers').value.split(/\s+/).map((u) => u.trim()).filter(Boolean);
    try {
      const settings = await api('/api/settings', { method: 'PUT', body: { modelServers } });
      window.DeloistSettings = settings;
      await scan();
    } catch (err) {
      toast(trServer(err.message));
    }
  }

  // -------------------------------------------------------------------------
  // Кружок лимитов подписки в левом меню
  // -------------------------------------------------------------------------

  // «codex 5h limit» → «5 ч», «Weekly limit» → «Неделя».
  function limitLabel(label) {
    const text = String(label || '').replace(/^codex\s+/i, '').replace(/\s*limit$/i, '').trim();
    const hours = /^(\d+)\s*h$/i.exec(text);
    if (hours) return tr('{n} ч', { n: hours[1] });
    if (/^weekly$/i.test(text)) return tr('Неделя');
    if (/^daily$/i.test(text)) return tr('День');
    if (/^monthly$/i.test(text)) return tr('Месяц');
    const scoped = /^weekly(?:\s+limit)?\s*·\s*(.+)$/i.exec(text);
    if (scoped) return tr('Неделя · {m}', { m: scoped[1] });
    return text;
  }

  // Два кольца: снаружи неделя, внутри окно 5 часов. Пустое кольцо — провайдер этого не сообщает.
  function arc(r, left) {
    const c = 2 * Math.PI * r;
    const track = `<circle cx="20" cy="20" r="${r}" class="usage-track"/>`;
    if (left == null) return track;
    const tone = left <= 20 ? ' low' : left <= 50 ? ' mid' : '';
    const share = Math.max(0, Math.min(100, left));
    return `${track}<circle cx="20" cy="20" r="${r}" class="usage-fill${tone}" stroke-dasharray="${(c * share) / 100} ${c}" transform="rotate(-90 20 20)"/>`;
  }
  const rings = (week, hours) => `<svg class="usage-ring" viewBox="0 0 40 40" aria-hidden="true">${arc(18, week)}${arc(13.5, hours)}</svg>`;

  // Из списка лимитов агента: окно 5 часов, общая неделя и прочие (недели по моделям).
  function windows(limits) {
    const list = (limits || []).map((l) => ({ ...l, left: Number(l.left) }));
    const hours = list.find((l) => /5\s*h/i.test(l.label));
    const week = list.find((l) => /weekly/i.test(l.label) && !/·/.test(l.label)) || list.find((l) => /weekly/i.test(l.label)) || list.find((l) => /monthly/i.test(l.label));
    return { hours, week, rest: list.filter((l) => l !== hours && l !== week) };
  }

  const LOCAL = new Set(['ollama', 'lmstudio']);
  const money = (kop) => ((Number(kop) || 0) / 100).toLocaleString(window.LOCALE, { style: 'currency', currency: 'RUB', minimumFractionDigits: 2 });
  const DELOIST_MARK = '<span class="k-logo k-logo--xs usage-mark" aria-hidden="true"></span>';

  let usageItems = [];
  let walletAt = 0;

  function usageRow(label, left, note) {
    return `<div class="usage-row"><span>${esc(label)}</span><span class="usage-bar"><span class="${left <= 20 ? 'low' : left <= 50 ? 'mid' : ''}" style="width:${Math.max(0, Math.min(100, left))}%"></span></span><b>${esc(tr('осталось {n} %', { n: Math.round(left) }))}</b><i>${note ? esc(note) : ''}</i></div>`;
  }

  function details(item) {
    const head = `<div class="usage-title">${DeloistUI.icon('gauge', { className: 'i' })}${esc(tr('Использование'))}</div>`;
    const name = `<div class="usage-name">${item.id === 'deloist' ? DELOIST_MARK : `<img src="/api/providers/${encodeURIComponent(item.icon || item.id)}/icon?color=${iconColor()}" alt="">`}<b>${esc(item.name)}</b></div>`;
    if (item.wallet) {
      const { startKop, nowKop } = item.wallet;
      const start = Math.max(startKop, nowKop, 0);
      const left = start > 0 ? (Math.max(nowKop, 0) / start) * 100 : 0;
      const spent = Math.max(0, start - Math.max(nowKop, 0));
      return `${head}${name}<div class="usage-money"><span>${esc(tr('Баланс'))}</span><b>${esc(money(nowKop))}</b></div>${usageRow(tr('Сегодня'), left, tr('потрачено {sum}', { sum: money(spent) }))}`;
    }
    if (item.local) return `${head}${name}<p class="usage-none">${esc(tr('Модель на этом компьютере: лимитов нет'))}</p>`;
    const { hours, week, rest } = windows(item.limits);
    const reason = ({
      auth_expired: 'Данные лимитов могут быть устаревшими: токен недействителен или истёк. Обратитесь к Gemini, чтобы обновить его',
      unavailable: 'Не удалось получить лимиты Gemini. Повторим позже',
      unsupported: 'Для входа по API-ключу лимиты подписки недоступны',
    })[item.limitsStatus];
    const lastRead = reason && item.at ? ` ${tr('Данные от {time}', { time: new Date(item.at).toLocaleString(window.LOCALE) })}` : '';
    const status = reason ? `<p class="usage-none">${esc(tr(reason) + lastRead)}</p>` : '';
    if (!hours && !week && !rest.length) return `${head}${name}${status || `<p class="usage-none">${esc(tr('Этот провайдер не сообщает лимиты'))}</p>`}`;
    const reset = (l) => (l.resets ? tr('сброс {t}', { t: l.resets }) : '');
    const rows = [hours, week, ...rest].filter(Boolean).map((l) => usageRow(limitLabel(l.label), l.left, reset(l))).join('');
    return `${head}${name}${rows}${status}`;
  }

  async function renderUsage() {
    const box = el('usage-rings');
    if (!box) return;
    // Баланс Deloist сервер помнит с последнего ответа сайта; раз в полчаса спрашиваем сами.
    if ((window.DeloistAccount || {}).signedIn && Date.now() - walletAt > 30 * 60 * 1000) {
      walletAt = Date.now();
      await api('/api/account/wallet').catch(() => {});
    }
    let limits;
    let list;
    try {
      [limits, list] = await Promise.all([api('/api/providers/limits'), api('/api/providers').catch(() => ({ providers: [] }))]);
    } catch {
      return;
    }
    const agents = new Map((limits.providers || []).map((p) => [p.id, p]));
    const items = [];
    if (limits.deloist) items.push({ id: 'deloist', name: 'Deloist', wallet: limits.deloist });
    for (const p of list.providers || []) {
      if (p.enabled === false) continue;
      if (p.kind === 'acp') {
        // У Gemini может быть до трёх аккаунтов Google: у каждого свой кружок (antigravity, antigravity@<id>).
        for (const a of limits.providers || []) {
          if (a.id === p.id || a.id.startsWith(p.id + '@')) items.push({ id: a.id, icon: p.id, name: a.name || p.name, limits: a.limits, limitsStatus: a.limitsStatus, at: a.at });
        }
      } else if (p.status === 'connected') {
        items.push({ id: p.id, name: p.name, limits: p.limits || [], local: LOCAL.has(p.id) });
      }
    }
    usageItems = items;
    box.hidden = !items.length;
    box.innerHTML = items.map((item, i) => {
      let outer = null;
      let inner = null;
      if (item.wallet) {
        const start = Math.max(item.wallet.startKop, item.wallet.nowKop, 0);
        outer = start > 0 ? (Math.max(item.wallet.nowKop, 0) / start) * 100 : 0;
      } else if (!item.local) {
        const { hours, week } = windows(item.limits);
        outer = week ? week.left : null;
        inner = hours ? hours.left : null;
      }
      const mark = item.id === 'deloist' ? DELOIST_MARK : `<img src="/api/providers/${encodeURIComponent(item.icon || item.id)}/icon?color=${iconColor()}" alt="" data-letter="${esc(item.name.charAt(0))}">`;
      return `<button type="button" class="usage-item" data-index="${i}" data-provider="${esc(item.id)}" aria-label="${esc(tr('Лимиты {name}', { name: item.name }))}">${rings(outer, inner)}${mark}</button>`;
    }).join('');
  }

  function startUsage() {
    const box = el('usage-rings');
    if (!box) return;
    const pop = document.createElement('div');
    pop.className = 'usage-pop';
    pop.setAttribute('role', 'tooltip');
    pop.hidden = true;
    document.body.append(pop);
    box.addEventListener('click', (e) => {
      const item = e.target.closest('[data-provider]');
      if (!item) return;
      if (item.dataset.provider === 'deloist') el('btn-account')?.click();
      else open(item.dataset.provider.split('@')[0]);
    });
    box.addEventListener('error', (e) => {
      if (e.target.tagName === 'IMG') e.target.replaceWith(Object.assign(document.createElement('span'), { className: 'usage-letter', textContent: e.target.dataset.letter || '' }));
    }, true);
    // Подсказка поверх окна, у самого кружка: левая панель обрезает всё, что за её краем.
    const show = (e) => {
      const button = e.target.closest('.usage-item');
      const item = button && usageItems[Number(button.dataset.index)];
      if (!item) return;
      pop.innerHTML = details(item);
      pop.hidden = false;
      const rect = button.getBoundingClientRect();
      const rail = document.body.classList.contains('side-collapsed');
      const h = pop.offsetHeight;
      if (rail) {
        pop.style.left = `${rect.right + 10}px`;
        pop.style.top = `${Math.max(8, Math.min(window.innerHeight - h - 8, rect.top + rect.height / 2 - h / 2))}px`;
      } else {
        pop.style.left = `${Math.max(8, rect.left - 6)}px`;
        pop.style.top = `${Math.max(8, rect.top - h - 10)}px`;
      }
    };
    const hide = (e) => {
      if (e.relatedTarget && e.target.closest('.usage-item')?.contains(e.relatedTarget)) return;
      pop.hidden = true;
    };
    box.addEventListener('mouseover', show);
    box.addEventListener('focusin', show);
    box.addEventListener('mouseout', hide);
    box.addEventListener('focusout', hide);
    renderUsage();
    setInterval(renderUsage, 5 * 60 * 1000);
  }

  // -------------------------------------------------------------------------
  // Снаружи
  // -------------------------------------------------------------------------

  async function open(id) {
    if (id) selected = id;
    modelFilter = '';
    window.openProviderSettings();
    setView(id ? 'detail' : 'list');
  }

  /// Событие `provider` из общего потока: ход скачивания, вход, смена статуса.
  function onEvent(ev) {
    if (ev.type !== 'provider') return;
    window.DeloistUpdates?.providers([ev.provider]);
    if (ev.provider?.limits?.length) renderUsage();
    if (!el('modal-settings').open || document.querySelector('[data-settings-section="providers"]').hidden) return;
    replace(ev.provider);
  }

  /// Скачанные и включённые агенты — для выбора «Откуда брать ответы» и модели бота.
  async function drivers() {
    try {
      const data = await api('/api/providers');
      return (data.providers || [])
        .filter((p) => p.enabled && (p.kind === 'acp' ? p.installed : p.status === 'connected'))
        .map((p) => ({ value: p.driver, label: p.name, models: (p.models || []).filter((m) => !(p.hiddenModels || []).includes(m.id)) }));
    } catch {
      return [];
    }
  }

  /// Всё для общего выбора модели: провайдеры как есть (с `models` и `hiddenModels`) и модели подписки Deloist.
  let choicesSnapshot = null;
  let choicesRequest = null;
  let choicesRevision = 0;
  const snapshot = () => choicesSnapshot;
  function invalidateChoices() { choicesRevision++; }
  function choices() {
    if (choicesRequest) return choicesRequest;
    const revision = choicesRevision;
    const request = (async () => {
      try {
        const data = await api('/api/providers');
        window.DeloistUpdates?.providers(data.providers || []);
        let plan = data.plan || null;
        if (!plan) plan = await api('/api/state').then((state) => state.plan || null).catch(() => null);
        const result = { providers: data.providers || [], plan };
        if (revision === choicesRevision) choicesSnapshot = result;
        return result;
      } catch (err) {
        if (choicesSnapshot) return choicesSnapshot;
        throw err;
      }
    })();
    choicesRequest = request.finally(() => { choicesRequest = null; })
      .then((data) => revision === choicesRevision ? data : choices());
    return choicesRequest;
  }

  function start(deps) {
    // Значок не скачался (нет сети, нет в реестре) — вместо него первая буква.
    el('modal-providers').addEventListener('error', (e) => {
      if (e.target.tagName === 'IMG' && e.target.dataset.letter) e.target.replaceWith(document.createTextNode(e.target.dataset.letter));
    }, true);
    api = deps.api;
    toast = deps.toast;
    onChanged = (change) => { invalidateChoices(); return deps.onChanged?.(change); };
    choices().catch(() => {});
    setInterval(() => { invalidateChoices(); choices().catch(() => {}); }, 5 * 60 * 1000);
    startUsage();
    listView = window.DeloistUI.mountComponent(window.DeloistUI.ProviderList, el('prov-list'), {
      groups: [],
      selected,
      label: tr('Провайдеры ИИ'),
      onselect: pickRow,
      onaction: (name) => { if (name === 'scan') scan(); },
    });
    const detail = el('prov-detail');
    detail.addEventListener('click', (e) => {
      const kind = e.target.closest('button[data-kind]');
      if (kind) {
        addKind = kind.dataset.kind;
        renderDetail();
        return;
      }
      const target = e.target.closest('button[data-act]');
      if (!target) return;
      if (target.dataset.act === 'open-gemini') pickRow('antigravity');
      else if (target.dataset.act === 'scan') scan();
      else if (target.dataset.act === 'add-create') createOwn();
      else if (target.dataset.act === 'servers-save') saveServers();
      else act(target.dataset.act, target);
    });
    detail.addEventListener('change', (e) => {
      if (e.target.matches('input[data-model]')) toggleModel(e.target);
      else if (e.target.matches('input[data-act="enabled"]')) act('enabled', e.target);
    });
    detail.addEventListener('input', (e) => {
      if (e.target.id !== 'prov-model-search') return;
      modelFilter = e.target.value.trim().toLowerCase();
      const caret = e.target.selectionStart;
      renderDetail();
      const search = el('prov-model-search');
      search.focus();
      search.setSelectionRange(caret, caret);
    });
    el('prov-search').addEventListener('input', (e) => {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => {
        query = e.target.value.trim();
        load();
      }, 250);
    });
  }

  // До start() адреса API нет: выбор модели, созданный раньше, загрузится в общем reloadModelPickers.
  const started = () => api !== null;

  return { start, started, open, load, showList: () => setView('list'), inDetail: () => screen === 'detail', onEvent, drivers, choices, snapshot, update: (id) => agentAction(id, 'install') };
})();
(() => {
  const init = () => {
    const trigger = document.getElementById('btn-side-menu');
    const menu = document.getElementById('side-menu');
    if (!trigger || !menu) return;
    document.body.append(menu);
    const close = (focus = false) => { menu.hidden = true; trigger.setAttribute('aria-expanded', 'false'); if (focus) trigger.focus(); };
    trigger.addEventListener('click', () => {
      if (!menu.hidden) return close();
      const rect = trigger.getBoundingClientRect();
      menu.style.left = `${Math.min(Math.max(8, rect.left), Math.max(8, innerWidth - 272))}px`;
      menu.style.bottom = `${Math.max(8, innerHeight - rect.top + 6)}px`;
      menu.style.maxHeight = `${Math.max(0, rect.top - 14)}px`;
      menu.hidden = false; trigger.setAttribute('aria-expanded', 'true');
      menu.querySelector('.side-menu-actions button')?.focus();
    });
    document.addEventListener('click', (event) => { if (!menu.hidden && !menu.contains(event.target) && !trigger.contains(event.target)) close(); });
    document.addEventListener('keydown', (event) => { if (event.key === 'Escape' && !menu.hidden) { event.preventDefault(); close(true); } });
    menu.addEventListener('click', (event) => { if (event.target.closest('button')) close(); });
    window.addEventListener('resize', () => close());
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init); else init();
})();
