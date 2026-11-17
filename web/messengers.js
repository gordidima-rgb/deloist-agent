// Мессенджеры бота в шапке чата: значок на каждый, куда бот заходил в своём браузере, точка статуса
// входа и карточка с правилами (нельзя / спросить / можно). Правила применяет сервер; здесь только
// показ и PUT. Список мессенджеров приходит с сервера (`registry`), своего списка здесь нет.
(function () {
  const tr = (text, vars) => (window.tr ? window.tr(text, vars) : text);
  const RULES = [
    ['read', 'Читать'],
    ['join', 'Вступать в группы и каналы'],
    ['group', 'Писать в группы и комментарии'],
    ['dm', 'Писать в личку'],
    ['files', 'Отправлять файлы'],
    ['delete', 'Удалять сообщения'],
  ];
  const LEVELS = [
    ['deny', 'Нельзя'],
    ['ask', 'Спросить'],
    ['allow', 'Можно'],
  ];
  // Свои простые значки для трёх первых (имена в lib/icons.js); у остальных круглая плашка с буквами.
  const GLYPHS = { telegram: 'msg-telegram', whatsapp: 'msg-whatsapp', max: 'msg-max' };

  let botId = null;
  let list = [];
  let registry = [];
  let pop = null;
  let hideTimer = 0;
  let openId = null;
  let seq = 0;

  const row = () => document.getElementById('head-messengers');
  const esc = (v) => String(v).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const nameOf = (m) => tr(m.name);

  // Адрес из разрешения («https://web.telegram.org», «web.telegram.org:443») → хост; совпадение как в сервере:
  // сам хост или его поддомен, а не «evilweb.telegram.org».
  function hostOf(place) {
    return String(place || '').split('://').pop().split(/[/:?#]/)[0].toLowerCase();
  }
  function placeIn(place, meta) {
    const host = hostOf(place);
    return Boolean(host) && (meta.hosts || []).some((h) => host === h || host.endsWith('.' + h));
  }
  function visibleMessenger(m) {
    const meta = registry.find((r) => r.id === m.id) || m;
    return meta;
  }
  // Разрешения этого разговора, которые относятся к мессенджеру m.
  function grantsOf(m) {
    const all = window.DeloistGrants ? window.DeloistGrants.all() : [];
    const meta = visibleMessenger(m);
    return all.filter((g) => placeIn(g.place, meta));
  }
  // app.js спрашивает, показывает ли шапка этот адрес сама: только если значок мессенджера сейчас на экране.
  function ownsPlace(place) {
    const box = row();
    if (!box || box.hidden) return false;
    return list.some((m) => placeIn(place, visibleMessenger(m)));
  }
  window.DeloistMessengers = { ownsPlace };

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

  function badge(m) {
    const meta = registry.find((r) => r.id === m.id) || m;
    const glyph = GLYPHS[m.id];
    const inner = glyph
      ? DeloistUI.icon(glyph, { stroke: 1.9 })
      : `<span class="msg-abbr">${esc(meta.abbr || m.name.charAt(0))}</span>`;
    const style = glyph ? '' : ` style="background:${esc(meta.color || '#667')};color:#fff;border-color:transparent"`;
    return `<span class="msg-badge${glyph ? ' msg-glyph' : ''}"${style}>${inner}</span>`;
  }

  function statusText(m) {
    if (m.status === 'signed_in') return tr('Подключён');
    if (m.status === 'signed_out') return tr('Вход слетел — нужно переподключиться');
    return tr('Ещё не проверяли');
  }

  function render() {
    const box = row();
    if (!box) return;
    const browserShown = !document.getElementById('btn-browser')?.hidden;
    box.hidden = !browserShown || !list.length;
    box.innerHTML = list
      .map((m) => `<button type="button" class="msg-chip k-btn k-btn--ghost k-btn--icon k-btn--sm" data-id="${esc(m.id)}" data-status="${esc(m.status)}" aria-label="${esc(`${nameOf(m)}: ${statusText(m)}`)}">${badge(m)}<i class="msg-dot k-dot"></i></button>`)
      .join('');
    if (openId) {
      const m = list.find((x) => x.id === openId);
      if (m && !box.hidden) fillPop(m);
      else closePop();
    }
    window.dispatchEvent(new CustomEvent('deloist:messengers'));
  }

  async function refresh() {
    const id = botId;
    const mine = ++seq;
    if (!id) {
      list = [];
      render();
      return;
    }
    try {
      const data = await call(`/api/bots/${encodeURIComponent(id)}/messengers`);
      if (mine !== seq || id !== botId) return;
      list = data.messengers || [];
      registry = data.registry || registry;
    } catch {
      if (mine !== seq) return;
      list = [];
    }
    render();
  }

  function ensurePop() {
    if (pop) return pop;
    pop = document.createElement('div');
    pop.className = 'msg-pop';
    pop.setAttribute('role', 'dialog');
    pop.hidden = true;
    pop.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    pop.addEventListener('mouseleave', scheduleHide);
    pop.addEventListener('focusin', () => clearTimeout(hideTimer));
    pop.addEventListener('focusout', (e) => {
      if (!pop.contains(e.relatedTarget)) scheduleHide();
    });
    pop.addEventListener('click', onPopClick);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !pop.hidden) closePop();
    });
    document.body.append(pop);
    return pop;
  }

  function fillPop(m) {
    const p = ensurePop();
    const rules = RULES.map(([key, label]) => {
      const now = (m.rules && m.rules[key]) || 'ask';
      const seg = LEVELS.map(([value, text]) => `<button type="button" role="radio" aria-checked="${now === value}" class="msg-seg k-chip k-chip--tool msg-${value}${now === value ? ' on is-on' : ''}" data-rule="${key}" data-value="${value}">${esc(tr(text))}</button>`).join('');
      return `<div class="msg-rule"><span>${esc(tr(label))}</span><div class="msg-segs" role="radiogroup" aria-label="${esc(tr(label))}">${seg}</div></div>`;
    }).join('');
    const grants = grantsOf(m);
    const what = grants.map((g) => `${window.DeloistGrants.toolTitle(g.tool)}: ${window.DeloistGrants.place(g.place)}`).join(', ');
    const allowed = grants.length
      ? `<div class="msg-grants"><span>${esc(tr('Разрешено в этом разговоре: {what}', { what }))}</span> <button type="button" class="msg-grants-off" data-act="grants-off">${esc(tr('Выключить'))}</button></div>`
      : '';
    const reconnect = m.status === 'signed_out' ? `<button type="button" class="msg-open k-btn k-btn--sm" data-act="open-browser">${esc(tr('Открыть браузер бота'))}</button>` : '';
    p.dataset.id = m.id;
    p.innerHTML = `<div class="msg-title">${badge(m)}<b>${esc(nameOf(m))}</b></div><div class="msg-status ${esc(m.status)}">${esc(statusText(m))}</div>${reconnect}${allowed}<div class="msg-rules">${rules}</div>`;
  }

  function openPop(chip) {
    const m = list.find((x) => x.id === chip.dataset.id);
    if (!m) return;
    clearTimeout(hideTimer);
    openId = m.id;
    fillPop(m);
    const p = ensurePop();
    p.hidden = false;
    const rect = chip.getBoundingClientRect();
    const w = p.offsetWidth;
    p.style.left = `${Math.max(8, Math.min(window.innerWidth - w - 8, rect.left + rect.width / 2 - w / 2))}px`;
    p.style.top = `${rect.bottom + 8}px`;
  }

  function closePop() {
    openId = null;
    if (pop) pop.hidden = true;
  }

  function scheduleHide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(closePop, 220);
  }

  async function onPopClick(e) {
    if (e.target.closest('[data-act="open-browser"]')) {
      closePop();
      document.getElementById('btn-browser')?.click();
      return;
    }
    if (e.target.closest('[data-act="grants-off"]')) {
      const m = list.find((x) => x.id === openId);
      if (m && window.DeloistGrants) await window.DeloistGrants.revoke(grantsOf(m));
      return;
    }
    const seg = e.target.closest('.msg-seg');
    if (!seg || !botId || !openId) return;
    const messenger = openId;
    const id = botId;
    try {
      const entry = await call(`/api/bots/${encodeURIComponent(id)}/messengers/${encodeURIComponent(messenger)}/rules`, 'PUT', { rule: seg.dataset.rule, value: seg.dataset.value });
      if (id !== botId) return;
      const at = list.findIndex((x) => x.id === messenger);
      if (at >= 0) list[at] = entry;
      else list.push(entry);
      render();
    } catch {
      window.toast?.(tr('Не удалось сохранить правило'));
      refresh();
    }
  }

  function onEvent(ev) {
    if (!ev || (ev.type !== 'messenger_status' && ev.type !== 'messenger_rules')) return;
    const id = ev.botId || ev.bot;
    if (id !== botId) return;
    if (ev.type === 'messenger_status' && ev.status === 'signed_out') {
      const meta = registry.find((r) => r.id === ev.messenger);
      window.toast?.(tr('{name}: вход слетел. Откройте браузер бота и войдите заново', { name: meta ? nameOf(meta) : ev.messenger }), 6000);
    }
    refresh();
  }

  function start() {
    const box = row();
    if (!box) return;
    box.addEventListener('mouseover', (e) => {
      const chip = e.target.closest('.msg-chip');
      if (chip) openPop(chip);
    });
    box.addEventListener('focusin', (e) => {
      const chip = e.target.closest('.msg-chip');
      if (chip) openPop(chip);
    });
    box.addEventListener('click', (e) => {
      const chip = e.target.closest('.msg-chip');
      if (chip) openPop(chip);
    });
    box.addEventListener('mouseleave', scheduleHide);
    box.addEventListener('focusout', (e) => {
      if (!pop || !pop.contains(e.relatedTarget)) scheduleHide();
    });
    const browser = document.getElementById('btn-browser');
    if (browser) new MutationObserver(render).observe(browser, { attributes: true, attributeFilter: ['hidden'] });
    window.addEventListener('deloist:bot', (e) => {
      botId = e.detail || null;
      closePop();
      refresh();
    });
    window.addEventListener('deloist:event', (e) => onEvent(e.detail));
    // Список разрешений обновился (включили, выключили): перерисовать строку в открытой карточке.
    window.addEventListener('deloist:grants', () => {
      const m = openId && list.find((x) => x.id === openId);
      if (m) fillPop(m);
    });
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
