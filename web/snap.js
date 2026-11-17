// Подсказка «правка выделенного текста»: маленькое окно у курсора. Сразу слушает просьбу
// («сделай вежливее»), можно и напечатать. Программа переписывает текст служебной моделью,
// подсказка прячется, возвращает фокус окну человека, и программа вставляет результат.
(() => {
  const tr = (key, params) => (window.tr ? window.tr(key, params) : key);
  const $ = (id) => document.getElementById(id);
  const ask = $('ask');
  const go = $('go');
  const micButton = $('mic');
  const status = $('status');
  const what = $('what');
  const voice = window.DeloistVoice;
  // Значок кнопки отправки в snap.html (svg с data-icon) заполняется сразу.
  DeloistUI.paintIcons();
  const MIC_ICON =
    DeloistUI.icon('mic-wide', { className: 'i' });
  const NOTICES = {
    password: 'В поле пароля правка голосом не работает',
    empty: 'Сначала выдели текст, потом нажми сочетание',
    keys: 'Отпусти клавиши сочетания и нажми его ещё раз',
    long: 'Слишком длинный текст: выдели до 20 000 знаков',
    console: 'В консоли правка голосом не работает: Ctrl+C там прерывает программу, а вставка выполняет команды',
    clipboard: 'Не смог сохранить буфер обмена, поэтому не трогаю его. Скопируй текст ещё раз и попробуй снова',
    unsure: 'Не понял, что за поле в фокусе, поэтому не трогаю его. Попробуй ещё раз',
  };
  // Почему готовый текст не вставили: человек видит его и копирует сам.
  const NOT_PASTED = {
    focus: 'Не вставил: окно сменилось',
    password: 'Не вставил: курсор стоит в поле пароля',
    keys: 'Не вставил: отпусти клавиши',
    clipboard: 'Не вставил: не смог сохранить буфер обмена',
    unread: 'Не вставилось: программа не забрала текст',
    unsure: 'Не удалось проверить поле — не вставляю',
  };

  let sessionId = null;
  let busy = false;
  let closing = false;
  // Вставка не удалась: текст показан, его можно скопировать или вставить ещё раз.
  let failed = false;
  // Окно подсказки точно впереди. Пока это не подтвердилось, Enter ничего не делает: нажатие могло
  // уйти в программу человека и, например, отправить там сообщение. Остаются кнопки.
  let focused = false;
  window.promptFocused = (yes) => {
    focused = Boolean(yes);
    if (failed) renderFailed();
  };

  const ipc = (message) => {
    try {
      window.ipc?.postMessage(message);
    } catch {}
  };
  const say = (text, error = false) => {
    status.textContent = text;
    status.classList.toggle('error', error);
  };
  const close = (delay = 0) => {
    if (closing) return;
    closing = true;
    setTimeout(() => ipc('close'), delay);
  };

  async function post(path, body) {
    const res = await fetch(path, { method: 'POST', cache: 'no-store', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body ?? {}) });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error((window.trServer ? window.trServer(data.error) : data.error) || `HTTP ${res.status}`);
    return data;
  }

  $('close').setAttribute('aria-label', tr('Закрыть'));
  go.setAttribute('aria-label', tr('Переписать'));
  go.title = tr('Переписать');
  ask.placeholder = tr('Скажи или напиши, что сделать: «сделай вежливее»');
  micButton.innerHTML = MIC_ICON;

  const mic = voice.voiceButton({
    button: micButton,
    onText: (text) => {
      voice.insertText(ask, text);
      // Сказали голосом — сразу в работу: отменить можно Ctrl+Z в своей программе.
      submit();
    },
    onError: (text) => say(text, true),
    onState: (state) => {
      if (state === 'listening') say(tr('Слушаю…'));
      else if (state === 'working') say(tr('Распознаю…'));
    },
  });

  function fit() {
    go.disabled = busy || !ask.value.trim();
  }

  async function submit() {
    const instruction = ask.value.trim();
    if (!instruction || busy || !sessionId) return;
    busy = true;
    fit();
    mic.cancel();
    say(tr('Переписываю…'));
    try {
      await post(`/api/selection/${encodeURIComponent(sessionId)}/rewrite`, { instruction });
    } catch (err) {
      busy = false;
      fit();
      say(err.message, true);
      return;
    }
    await pasteNow();
  }

  // Спрятаться и вернуть фокус окну человека, потом программа вставит текст.
  async function pasteNow() {
    focused = false;
    ipc('handoff');
    await new Promise((resolve) => setTimeout(resolve, 120));
    let done;
    try {
      done = await post(`/api/selection/${encodeURIComponent(sessionId)}/apply`);
    } catch (err) {
      done = { ok: false, reason: 'error', error: err.message };
    }
    if (done.ok) {
      sessionId = null;
      document.body.classList.add('notice');
      what.textContent = tr('Готово');
      say(tr('Текст заменён. Ctrl+Z — вернуть как было'));
      ipc('notice');
      close(2600);
      return;
    }
    // Вслепую не вставляли. Правка жива: текст можно скопировать или вставить ещё раз.
    busy = false;
    failed = true;
    if (done.text) ask.value = done.text;
    ask.readOnly = true;
    what.textContent = done.error || tr(NOT_PASTED[done.reason] || NOT_PASTED.focus);
    ipc('show');
    renderFailed();
  }

  // Кнопки «Скопировать» и «Вставить ещё раз» вместо голоса; Enter — только если окно точно в фокусе.
  let actions = null;
  function renderFailed() {
    micButton.hidden = true;
    go.hidden = true;
    if (!actions) {
      actions = document.createElement('div');
      actions.className = 'actions';
      const copy = document.createElement('button');
      copy.type = 'button';
      copy.className = 'k-btn k-btn--ghost';
      copy.textContent = tr('Скопировать');
      copy.addEventListener('click', copyAndClose);
      const retry = document.createElement('button');
      retry.type = 'button';
      retry.className = 'k-btn k-btn--ghost';
      retry.textContent = tr('Вставить ещё раз');
      retry.addEventListener('click', () => {
        if (busy || !sessionId) return;
        busy = true;
        say(tr('Вставляю…'));
        pasteNow();
      });
      actions.append(copy, retry);
      status.after(actions);
    }
    say(focused ? tr('Enter — скопировать текст') : tr('Щёлкни «Скопировать» или «Вставить ещё раз»'));
    if (focused) {
      ask.focus();
      ask.select();
    }
  }

  async function copyAndClose() {
    try {
      await navigator.clipboard.writeText(ask.value);
      say(tr('Скопировано'));
    } catch {
      say(tr('Не получилось скопировать'), true);
    }
    close(900);
  }

  async function cancel() {
    mic.cancel();
    if (sessionId) await post(`/api/selection/${encodeURIComponent(sessionId)}/cancel`).catch(() => {});
    close();
  }

  ask.addEventListener('input', () => {
    // Начали печатать — микрофон больше не нужен.
    if (!ask.readOnly && mic.state === 'listening') mic.cancel().then(() => say(''));
    fit();
  });
  ask.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      if (!failed) submit();
      else if (focused && document.hasFocus()) copyAndClose();
    }
  });
  go.addEventListener('click', () => {
    if (!failed) submit();
  });
  $('close').addEventListener('click', cancel);
  addEventListener('keydown', (event) => {
    if (event.key === 'Escape') cancel();
  });

  async function boot() {
    let info;
    try {
      const res = await fetch('/api/selection/current', { cache: 'no-store' });
      info = await res.json();
    } catch {
      close();
      return;
    }
    if (!info?.id) {
      close();
      return;
    }
    if (info.notice) {
      document.body.classList.add('notice');
      what.textContent = 'Deloist';
      say(tr(NOTICES[info.notice] || NOTICES.empty));
      await post(`/api/selection/${encodeURIComponent(info.id)}/cancel`).catch(() => {});
      close(2600);
      return;
    }
    sessionId = info.id;
    what.textContent = tr('Выделено знаков: {count}', { count: info.chars });
    ask.focus();
    mic.start();
  }

  fit();
  boot();
})();
