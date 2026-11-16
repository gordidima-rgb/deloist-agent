// Распознавание речи на этом компьютере (stt_local.rs): подготовка модели перед записью
// и раздел «Голос» в настройках. Кнопка микрофона (voice.js) зовёт DeloistStt.prepare()
// перед записью: при первом нажатии модель скачивается, кнопка показывает ход скачивания.
(() => {
  const tr = window.tr || ((text, vars) => String(text).replace(/\{(\w+)\}/g, (_, k) => (vars && k in vars ? vars[k] : `{${k}}`)));
  const serverText = (text) => (window.trServer ? window.trServer(text) : text);
  const $ = (id) => document.getElementById(id);
  const POLL_MS = 300;

  async function call(path, method = 'GET', body) {
    const init = { method, cache: 'no-store' };
    if (body !== undefined) {
      init.headers = { 'content-type': 'application/json' };
      init.body = JSON.stringify(body);
    }
    const res = await fetch(path, init);
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(serverText(data.error) || `HTTP ${res.status}`);
    return data;
  }

  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  // Платный голос через сайт включается флагом сервера (`cloud` в /api/voice/settings); в 0.3.1 его нет.
  let cloud = false;

  /** Подсказка после неудачного скачивания: платный путь через сайт, если он включён. */
  function withCloudHint(message) {
    return cloud ? `${message}. ${tr('Можно распознавать через Deloist: Настройки → Голос')}` : `${message}.`;
  }

  /**
   * Готов ли движок слушать. Для этого компьютера при первом разе скачивает и загружает модель,
   * сообщая onStep('downloading', процент) и onStep('loading'). true — можно записывать,
   * false — человек отменил. Ошибка — исключение с текстом для человека.
   */
  async function prepare(onStep) {
    const first = await call('/api/voice/prepare', 'POST', {});
    if (first.ready) return true;
    let job = first.job;
    for (;;) {
      if (job?.state === 'downloading') onStep?.('downloading', Number(job.total) ? Math.min(100, Math.floor((job.received * 100) / job.total)) : 0);
      else if (job?.state === 'loading') onStep?.('loading', 100);
      else if (job?.state === 'ready') return true;
      else if (job?.state === 'error') {
        const text = serverText(job.error) || tr('Не получилось подготовить распознавание речи');
        throw new Error(/скачать|download/i.test(job.error || '') ? withCloudHint(text.replace(/\.$/, '')) : text);
      } else if (job?.state === 'idle') return false;
      await sleep(POLL_MS);
      job = (await call('/api/voice/local')).job;
    }
  }

  function cancel() {
    return call('/api/voice/local/cancel', 'POST', {}).catch(() => undefined);
  }

  // Кнопка микрофона, пока качается модель: кольцо прогресса вокруг значка.
  const style = document.createElement('style');
  style.textContent = `
    .mic-btn[data-state="downloading"], .mic-btn[data-state="loading"] { cursor: progress; }
    .mic-btn[data-state="downloading"]::after, .mic-btn[data-state="loading"]::after {
      content: ''; position: absolute; inset: -3px; border-radius: inherit; pointer-events: none;
      background: conic-gradient(var(--color-accent) calc(var(--progress, 0) * 1turn), color-mix(in srgb, var(--color-accent) 18%, transparent) 0);
      -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 2px));
              mask: radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 2px));
    }
    .mic-btn[data-state="loading"]::after { animation: stt-spin 1s linear infinite; --progress: 0.3; }
    @keyframes stt-spin { to { transform: rotate(1turn); } }
    .stt-models { display: flex; flex-direction: column; gap: 6px; }
    .stt-model { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: var(--radius-md, 10px); border: 1px solid var(--color-border-default); cursor: pointer; }
    .stt-model.chosen { border-color: var(--color-brand-primary, var(--color-accent)); }
    .stt-model input { flex: none; }
    .stt-model .stt-text { flex: 1; min-width: 0; }
    .stt-model .stt-meta { color: var(--color-text-muted); font-size: 12px; }
    .stt-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .stt-row input[type="text"], .stt-row input[type="password"] { flex: 1; min-width: 180px; }
    .stt-progress { color: var(--color-accent-text); font-size: 13px; }
    .stt-progress:empty { display: none; }
    .stt-block[hidden] { display: none; }
  `;
  document.head.append(style);

  // --- Раздел «Голос» в настройках --------------------------------------------------

  const CHEVRON = DeloistUI.icon('chevron-right', { className: 'i t-chev' });
  const ENGINES = [
    ['local', 'На этом компьютере', 'Бесплатно. Речь не уходит в интернет. Модель скачивается один раз.'],
    ['deloist', 'Через Deloist', 'Yandex SpeechKit, платно: списывается с баланса Deloist. Нужен вход в аккаунт.'],
    ['server', 'Свой сервер', 'Любой сервер с OpenAI-совместимым /audio/transcriptions: адрес и ключ.'],
  ];
  const LANG_NAMES = { auto: 'Авто', ru: 'Русский', en: 'Английский', de: 'Немецкий', fr: 'Французский', es: 'Испанский', it: 'Итальянский', uk: 'Украинский' };

  function el(tag, cls, text) {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function line(title, sub) {
    const row = el('div', 'prov-line');
    const text = el('div');
    text.append(el('div', 'prov-line-title', title));
    if (sub) text.append(el('div', 'prov-line-sub', sub));
    row.append(text);
    return row;
  }

  function injectSettings() {
    const anchor = $('set-hotkeys') || $('set-voice-anchor') || $('set-dock');
    if (!anchor || $('set-voice')) return false;
    const group = el('details', 'group');
    group.id = 'set-voice';
    const summary = el('summary');
    summary.innerHTML = CHEVRON;
    summary.append(tr('Распознавание речи'));
    const body = el('div', 'group-body');
    const box = el('div', 'dock-box');
    body.append(box);
    group.append(summary, body);
    anchor.after(group);

    // Где распознавать
    const engineRow = line(tr('Где распознавать речь'));
    const engineSelect = el('select');
    for (const [id, title] of ENGINES) {
      const option = el('option', '', tr(title));
      option.value = id;
      engineSelect.append(option);
    }
    engineRow.append(engineSelect);
    const engineNote = el('p', 'note');
    box.append(engineRow, engineNote);

    // Этот компьютер: модели и язык
    const local = el('div', 'stt-block');
    const models = el('div', 'stt-models');
    const progress = el('div', 'stt-progress');
    progress.setAttribute('role', 'status');
    const langRow = line(tr('Язык речи'));
    const langSelect = el('select');
    langRow.append(langSelect);
    const langNote = el('p', 'note');
    // Список моделей с кнопками «Скачать» живёт на подэкране «Модели распознавания»; здесь только строка-вход.
    const modelsHost = $('stt-models-host');
    const modelsRow = line(tr('Модель распознавания'), ' ');
    const modelsOpen = el('button', 'ghost voice-small', tr('Выбрать'));
    modelsOpen.type = 'button';
    modelsOpen.addEventListener('click', () => window.DeloistSettings?.openSub('stt-models'));
    modelsRow.append(modelsOpen);
    if (modelsHost) {
      modelsHost.append(models, progress);
      local.append(modelsRow, langRow, langNote);
    } else {
      local.append(models, progress, langRow, langNote);
    }

    // Свой сервер
    const server = el('div', 'stt-block');
    const urlRow = el('div', 'stt-row');
    const url = el('input');
    url.type = 'text';
    url.placeholder = 'http://127.0.0.1:8000/v1';
    url.setAttribute('aria-label', tr('Адрес сервера'));
    const modelName = el('input');
    modelName.type = 'text';
    modelName.placeholder = 'whisper-1';
    modelName.setAttribute('aria-label', tr('Модель на сервере'));
    urlRow.append(url, modelName);
    const keyRow = el('div', 'stt-row');
    const key = el('input');
    key.type = 'password';
    key.autocomplete = 'off';
    key.setAttribute('aria-label', tr('Ключ сервера'));
    const saveServer = el('button', 'ghost voice-small', tr('Сохранить'));
    saveServer.type = 'button';
    keyRow.append(key, saveServer);
    server.append(el('p', 'note', tr('Адрес до /audio/transcriptions, например http://127.0.0.1:8000/v1. Ключ хранится только на этом компьютере.')), urlRow, keyRow);

    const status = el('div', 'voice-status');
    status.setAttribute('role', 'status');
    box.append(local, server, status);

    let data = null;
    let timer = 0;

    function render() {
      if (!data) return;
      cloud = Boolean(data.cloud);
      for (const option of engineSelect.options) option.hidden = option.disabled = option.value === 'deloist' && !cloud;
      engineSelect.value = data.engine;
      const info = ENGINES.find(([id]) => id === data.engine);
      engineNote.textContent = info ? tr(info[2]) : '';
      if (data.engine === 'deloist' && !data.signedIn) engineNote.textContent += ` ${tr('Сейчас вход не выполнен.')}`;
      local.hidden = data.engine !== 'local';
      server.hidden = data.engine !== 'server';
      const job = data.job || {};
      const busy = job.state === 'downloading' || job.state === 'loading';

      models.replaceChildren();
      for (const m of data.models) {
        const row = el('label', `stt-model${m.id === data.model ? ' chosen' : ''}`);
        const radio = el('input');
        radio.type = 'radio';
        radio.name = 'stt-model';
        radio.checked = m.id === data.model;
        radio.addEventListener('change', () => save({ model: m.id }));
        const text = el('div', 'stt-text');
        text.append(el('div', 'prov-line-title', m.title), el('div', 'prov-line-sub', tr(m.note)));
        const meta = el('div', 'stt-meta', `${tr('{size} МБ', { size: m.sizeMb })} · ${m.license}`);
        text.append(meta);
        const action = el('button', 'ghost voice-small');
        action.type = 'button';
        if (busy && job.model === m.id) {
          action.textContent = tr('Отменить');
          action.addEventListener('click', (event) => {
            event.preventDefault();
            cancel().then(load);
          });
        } else if (m.installed) {
          action.textContent = tr('Удалить');
          action.disabled = busy;
          action.addEventListener('click', (event) => {
            event.preventDefault();
            remove(m.id);
          });
        } else {
          action.textContent = tr('Скачать');
          action.disabled = busy;
          action.addEventListener('click', (event) => {
            event.preventDefault();
            download(m.id);
          });
        }
        row.append(radio, text, action);
        models.append(row);
      }
      if (job.state === 'downloading') progress.textContent = tr('Скачиваю {title}: {percent}%', { title: titleOf(job.model), percent: data.progress ?? 0 });
      else if (job.state === 'loading') progress.textContent = tr('Загружаю модель…');
      else if (job.state === 'error') progress.textContent = withCloudHint(serverText(job.error).replace(/\.$/, ''));
      else progress.textContent = '';

      const chosen = data.models.find((m) => m.id === data.model);
      modelsRow.querySelector('.prov-line-sub').textContent = chosen ? `${chosen.title} · ${tr('{size} МБ', { size: chosen.sizeMb })}` : '';
      langSelect.replaceChildren();
      const langs = chosen?.languages === 'choice' ? data.languages : chosen?.languages === 'ru' ? ['ru'] : ['auto'];
      for (const code of langs) {
        const option = el('option', '', tr(LANG_NAMES[code] || code));
        option.value = code;
        langSelect.append(option);
      }
      langSelect.value = langs.includes(data.language) ? data.language : langs[0];
      langSelect.disabled = langs.length < 2;
      langNote.textContent = chosen?.languages === 'ru' ? tr('Эта модель понимает только русский. Для других языков выбери Parakeet.') : chosen?.languages === 'auto' ? tr('Модель сама определяет язык речи.') : '';

      if (document.activeElement !== url) url.value = data.serverUrl || '';
      if (document.activeElement !== modelName) modelName.value = data.serverModel || '';
      key.placeholder = data.serverKey ? tr('Ключ сохранён') : tr('Ключ, если нужен');

      clearTimeout(timer);
      if (busy && group.open) timer = setTimeout(refreshJob, 700);
    }

    const titleOf = (id) => data?.models.find((m) => m.id === id)?.title || id;

    async function load() {
      try {
        data = await call('/api/voice/settings');
        status.textContent = '';
      } catch (err) {
        status.textContent = err.message;
      }
      render();
    }

    async function refreshJob() {
      try {
        const res = await call('/api/voice/local');
        const wasBusy = data?.job && ['downloading', 'loading'].includes(data.job.state);
        data.job = res.job;
        data.progress = res.progress;
        // Скачивание кончилось: список «скачана/нет» поменялся.
        if (wasBusy && !['downloading', 'loading'].includes(res.job.state)) return load();
      } catch {
        // сервер перезапускается — спросим позже
      }
      render();
    }

    async function save(patch) {
      try {
        data = await call('/api/voice/settings', 'PUT', patch);
        status.textContent = '';
      } catch (err) {
        status.textContent = err.message;
      }
      render();
    }

    async function download(id) {
      try {
        const res = await call(`/api/voice/local/${encodeURIComponent(id)}/download`, 'POST', {});
        data.job = res.job;
        data.progress = res.progress;
      } catch (err) {
        status.textContent = err.message;
      }
      render();
    }

    async function remove(id) {
      try {
        data = await call(`/api/voice/local/${encodeURIComponent(id)}`, 'DELETE');
        status.textContent = tr('Модель удалена');
      } catch (err) {
        status.textContent = err.message;
      }
      render();
    }

    engineSelect.addEventListener('change', () => save({ engine: engineSelect.value }));
    langSelect.addEventListener('change', () => save({ language: langSelect.value }));
    saveServer.addEventListener('click', () => {
      const patch = { serverUrl: url.value, serverModel: modelName.value };
      if (key.value) patch.serverKey = key.value;
      key.value = '';
      save(patch).then(() => {
        if (!status.textContent) status.textContent = tr('Сохранено');
      });
    });
    group.addEventListener('toggle', () => group.open && load());
    return true;
  }

  window.DeloistStt = { prepare, cancel };
  // Раздел горячих клавиш voice.js вставляет при загрузке страницы; наш встаёт после него.
  if (!injectSettings()) document.addEventListener('DOMContentLoaded', injectSettings, { once: true });
})();
