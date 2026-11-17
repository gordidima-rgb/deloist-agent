// Вход в аккаунт Deloist. Без входа программа показывает только этот экран.
// Почта: код приходит письмом. VK и Яндекс: вход на deloist.ru в обычном браузере и кнопка «Разрешить» там.
window.DeloistSignin = (() => {
  let api = null;
  let root = null;
  let resolveReady = null;
  let email = '';
  let resendAt = 0;
  let resendTimer = 0;
  let pollTimer = 0;
  let busy = false;

  const LEGAL = 'https://deloist.ru/legal';

  const ICONS = {
    mark: '<span class="k-logo k-logo--lg" aria-hidden="true"></span>',
    vk: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12.9 17.5c-5.5 0-8.6-3.8-8.8-10h2.8c.1 4.6 2.1 6.5 3.7 6.9V7.5h2.6v3.9c1.6-.2 3.3-2 3.8-3.9h2.6c-.4 2.4-2.2 4.2-3.5 4.9 1.3.6 3.3 2.1 4.1 5.1h-2.9c-.6-1.9-2.1-3.4-4.1-3.6v3.6h-.3z"/></svg>',
    yandex: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M13.6 20.5h2.6V3.5h-3.8c-3.8 0-5.8 2-5.8 4.9 0 2.3 1.1 3.7 3.1 5.1L6.2 20.5h2.8l3.9-5.9-1.3-.9c-1.6-1.1-2.4-2-2.4-3.8 0-1.6 1.1-2.7 3.3-2.7h1.1v13.3z"/></svg>',
    lock: DeloistUI.icon('lock', { className: 'i' }),
  };

  const $ = (sel) => root.querySelector(sel);

  function build() {
    root = document.createElement('div');
    root.id = 'signin';
    root.className = 'signin';
    root.innerHTML = `
      <div class="signin-card" role="dialog" aria-modal="true" aria-labelledby="signin-title">
        <div class="signin-mark">${ICONS.mark}</div>
        <section data-step="email">
          <h1 id="signin-title">${tr('Вход в Deloist')}</h1>
          <p class="signin-lead">${tr('Войди или создай аккаунт по почте. Пришлём одноразовый код.')}</p>
          <form id="signin-email-form" novalidate>
            <input id="signin-login" type="text" inputmode="email" autocomplete="email" spellcheck="false"
              placeholder="${tr('Почта или телефон')}" aria-label="${tr('Почта или телефон')}" />
            <button type="submit" class="k-btn k-btn--primary signin-wide" id="signin-send">${tr('Получить код')}</button>
          </form>
          <div class="signin-or"><span>${tr('или войди через')}</span></div>
          <div class="signin-providers">
            <button type="button" class="k-btn k-btn--ghost" data-via="vk">${ICONS.vk}VK ID</button>
            <button type="button" class="k-btn k-btn--ghost" data-via="yandex">${ICONS.yandex}${tr('Яндекс')}</button>
          </div>
        </section>
        <section data-step="code" hidden>
          <h1>${tr('Проверь почту')}</h1>
          <p class="signin-lead" id="signin-sent"></p>
          <form id="signin-code-form" novalidate>
            <input id="signin-code" class="signin-code" type="text" inputmode="numeric" autocomplete="one-time-code"
              maxlength="6" placeholder="000000" aria-label="${tr('Код из письма')}" />
            <button type="submit" class="k-btn k-btn--primary signin-wide" id="signin-verify">${tr('Войти')}</button>
          </form>
          <div class="signin-links">
            <button type="button" class="signin-link" id="signin-resend"></button>
            <button type="button" class="signin-link" id="signin-back">${tr('Другая почта')}</button>
          </div>
        </section>
        <section data-step="browser" hidden>
          <h1>${tr('Продолжи в браузере')}</h1>
          <p class="signin-lead">${tr('Мы открыли deloist.ru. Войди там и нажми «Разрешить» — программа войдёт сама.')}</p>
          <p class="signin-code-show" id="signin-user-code"></p>
          <p class="signin-hint">${tr('Сверь этот код с тем, что покажет сайт.')}</p>
          <div class="signin-links">
            <button type="button" class="signin-link" id="signin-reopen">${tr('Открыть сайт ещё раз')}</button>
            <button type="button" class="signin-link" id="signin-browser-back">${tr('Назад')}</button>
          </div>
        </section>
        <p class="signin-error" id="signin-error" role="alert" hidden></p>
        <p class="signin-private">${ICONS.lock}<span>${tr('Твои боты, чаты и файлы остаются на этом компьютере.')}</span></p>
        <p class="signin-legal">${tr('Документы: {offer} и {privacy}. Согласие с ними спросим отдельно после входа.', {
          offer: `<a href="${LEGAL}/offer" target="_blank" rel="noopener">${tr('оферта')}</a>`,
          privacy: `<a href="${LEGAL}/privacy" target="_blank" rel="noopener">${tr('политика конфиденциальности')}</a>`,
        })}</p>
      </div>`;
    document.body.appendChild(root);

    $('#signin-email-form').addEventListener('submit', sendCode);
    $('#signin-code-form').addEventListener('submit', verify);
    $('#signin-code').addEventListener('input', (e) => {
      e.target.value = e.target.value.replace(/\D/g, '').slice(0, 6);
      if (e.target.value.length === 6) verify(e);
    });
    $('#signin-resend').addEventListener('click', () => sendCode(null, true));
    $('#signin-back').addEventListener('click', () => step('email'));
    root.querySelectorAll('[data-via]').forEach((btn) => btn.addEventListener('click', () => viaBrowser(btn.dataset.via)));
    $('#signin-reopen').addEventListener('click', () => { if (root.dataset.url) window.open(root.dataset.url, '_blank'); });
    $('#signin-browser-back').addEventListener('click', () => {
      stopPoll();
      api('/api/account/browser/cancel', { method: 'POST' }).catch(() => {});
      step('email');
    });
  }

  function step(name) {
    root.querySelectorAll('[data-step]').forEach((s) => { s.hidden = s.dataset.step !== name; });
    showError('');
    const focus = { email: '#signin-login', code: '#signin-code' }[name];
    if (focus) setTimeout(() => $(focus).focus(), 0);
  }

  function showError(text) {
    const box = $('#signin-error');
    box.textContent = text;
    box.hidden = !text;
  }

  function setBusy(button, on, label) {
    busy = on;
    button.disabled = on;
    if (label) button.textContent = label;
  }

  function tickResend() {
    const left = Math.ceil((resendAt - Date.now()) / 1000);
    const btn = $('#signin-resend');
    btn.disabled = left > 0;
    btn.textContent = left > 0 ? tr('Отправить ещё раз через {sec} с', { sec: left }) : tr('Отправить ещё раз');
    if (left <= 0) clearInterval(resendTimer);
  }

  async function sendCode(event, again = false) {
    event?.preventDefault();
    if (busy) return;
    const login = again ? email : $('#signin-login').value.trim();
    const button = again ? $('#signin-resend') : $('#signin-send');
    setBusy(button, true, again ? null : tr('Отправляю…'));
    showError('');
    try {
      const res = await api('/api/account/code', { method: 'POST', body: { login } });
      email = res.email || login;
      $('#signin-sent').textContent = tr('Код отправили на {email}. Введи его здесь — он действует 10 минут.', { email });
      $('#signin-code').value = '';
      step('code');
      resendAt = Date.now() + 60_000;
      clearInterval(resendTimer);
      resendTimer = setInterval(tickResend, 1000);
      tickResend();
    } catch (err) {
      showError(trServer(err.message));
    } finally {
      setBusy(button, false, again ? null : tr('Получить код'));
      if (again) tickResend();
    }
  }

  async function verify(event) {
    event?.preventDefault();
    if (busy) return;
    const code = $('#signin-code').value.trim();
    if (code.length !== 6) {
      showError(tr('В коде шесть цифр'));
      return;
    }
    const button = $('#signin-verify');
    setBusy(button, true, tr('Проверяю…'));
    showError('');
    try {
      const account = await api('/api/account/verify', { method: 'POST', body: { login: email, code } });
      done(account);
    } catch (err) {
      showError(trServer(err.message));
      $('#signin-code').select();
    } finally {
      setBusy(button, false, tr('Войти'));
    }
  }

  async function viaBrowser(via) {
    if (busy) return;
    busy = true;
    showError('');
    try {
      const res = await api('/api/account/browser', { method: 'POST', body: { via } });
      root.dataset.url = res.url;
      $('#signin-user-code').textContent = res.userCode || '';
      step('browser');
      window.open(res.url, '_blank');
      startPoll();
    } catch (err) {
      showError(trServer(err.message));
    } finally {
      busy = false;
    }
  }

  function stopPoll() {
    clearTimeout(pollTimer);
    pollTimer = 0;
  }

  function startPoll() {
    stopPoll();
    const tick = async () => {
      try {
        const res = await api('/api/account/browser/poll', { method: 'POST' });
        if (res.status === 'done') return done(res.account);
        if (res.status === 'expired' || res.status === 'none') {
          step('email');
          showError(tr('Время на вход вышло. Нажми кнопку ещё раз'));
          return;
        }
      } catch {
        // Сеть мигнула — спросим ещё раз.
      }
      pollTimer = setTimeout(tick, 2500);
    };
    pollTimer = setTimeout(tick, 2500);
  }

  function done(account) {
    stopPoll();
    clearInterval(resendTimer);
    window.DeloistAccount = account;
    root.remove();
    root = null;
    resolveReady?.();
  }

  // Первый запуск: выбор языка. Подписи берутся из выбранного языка, а не из текущего,
  // поэтому они здесь, а не в словаре tr().
  const LANGS = [
    { code: 'ru', tag: 'RU', name: 'Русский', title: 'Выбери язык', go: 'Продолжить' },
    { code: 'en', tag: 'EN', name: 'English', title: 'Choose your language', go: 'Continue' },
  ];

  function chooseLanguage() {
    let index = Math.max(0, LANGS.findIndex((l) => l.code === window.LANG));
    const box = document.createElement('div');
    box.id = 'lang-pick';
    box.className = 'signin';
    box.innerHTML = `
      <div class="signin-card" role="dialog" aria-modal="true" aria-labelledby="lang-title">
        <div class="signin-mark">${ICONS.mark}</div>
        <p class="lang-title" id="lang-title"></p>
        <div class="lang-list" role="radiogroup" aria-labelledby="lang-title">
          ${LANGS.map((l, i) => `<button type="button" class="lang-option" role="radio" data-i="${i}" lang="${l.code}">
            <span class="lang-tag">${l.tag}</span><span class="lang-name">${l.name}</span></button>`).join('')}
        </div>
        <button type="button" class="k-btn k-btn--primary signin-wide lang-go" id="lang-go"></button>
      </div>`;
    document.body.appendChild(box);
    const options = [...box.querySelectorAll('.lang-option')];

    const mark = () => {
      const lang = LANGS[index];
      options.forEach((btn, i) => {
        btn.setAttribute('aria-checked', String(i === index));
        btn.tabIndex = i === index ? 0 : -1;
      });
      box.querySelector('#lang-title').textContent = lang.title;
      box.querySelector('#lang-go').innerHTML = `${lang.go}${DeloistUI.icon('arrow-right', { className: 'i' })}`;
      document.documentElement.lang = lang.code;
    };

    return new Promise((resolve) => {
      const go = async () => {
        const lang = LANGS[index].code;
        box.querySelector('#lang-go').disabled = true;
        try {
          await api('/api/settings', { method: 'PUT', body: { uiLang: lang } });
        } catch {
          // Не сохранилось — не держим человека на этом экране, язык спросим в настройках.
        }
        // Язык страницы выбирается при загрузке: другой язык — одна перезагрузка, дальше сразу вход.
        if (lang !== window.LANG && window.rememberLang(lang)) {
          location.reload();
          return;
        }
        box.remove();
        resolve();
      };
      options.forEach((btn, i) => btn.addEventListener('click', () => {
        if (index === i) go();
        index = i;
        mark();
      }));
      box.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
          e.preventDefault();
          index = (index + (e.key === 'ArrowDown' ? 1 : LANGS.length - 1)) % LANGS.length;
          mark();
          options[index].focus();
        } else if (e.key === 'Enter') {
          e.preventDefault();
          go();
        }
      });
      box.querySelector('#lang-go').addEventListener('click', go);
      mark();
      options[index].focus();
    });
  }

  // Ждёт входа. Если вход уже есть или не нужен, сразу возвращается.
  async function ensure(deps) {
    api = deps.api;
    let account;
    try {
      account = await api('/api/account');
    } catch {
      return; // Локальный сервер не ответил: дальше об этом скажет обычная загрузка.
    }
    window.DeloistAccount = account;
    if (!account.lang) await chooseLanguage();
    if (!account.required) return;
    if (!account.signedIn) {
      await new Promise((resolve) => {
        resolveReady = resolve;
        build();
        step('email');
      });
    }
    await askConsent();
    if (!account.onboarded) await chooseTeam();
  }

  // Отдельное согласие с документами Deloist: после входа и на любой ответ сайта 451.
  // Сайт без этого адреса (ещё не обновлён) отвечает пустым списком — ничего не показываем.
  // «Позже» закрывает карточку: без согласия не работает только баланс Deloist.
  let consentOpen = null;
  function askConsent() {
    if (consentOpen) return consentOpen;
    consentOpen = (async () => {
      let docs = [];
      try {
        docs = (await api('/api/account/consent')).required || [];
      } catch {
        return; // Нет сети или входа: спросим в следующий раз.
      }
      if (!docs.length || !window.DeloistUI?.mountConsentCard) return;
      const box = document.createElement('div');
      box.id = 'consent-card';
      document.body.appendChild(box);
      await new Promise((resolve) => {
        let handle = null;
        const done = () => {
          setTimeout(() => { handle?.destroy(); box.remove(); }, 250);
          resolve();
        };
        handle = DeloistUI.mountConsentCard(box, {
          docs,
          onaccept: async (accepted) => {
            let left;
            try {
              left = await api('/api/account/consent', { method: 'POST', body: { accepted } });
            } catch (err) {
              // Документы обновились, пока карточка была открыта: показываем новые.
              if (err.code === 'stale_version') {
                const fresh = (await api('/api/account/consent').catch(() => ({}))).required || [];
                if (fresh.length) handle.update({ docs: fresh });
              }
              throw new Error(trServer(err.message));
            }
            if (left?.required?.length) {
              handle.update({ docs: left.required });
              throw new Error(tr('Сайт просит принять ещё документы'));
            }
            done();
          },
          onlater: done,
        });
      });
    })().finally(() => { consentOpen = null; });
    return consentOpen;
  }

  // После первого входа: на чём будут работать боты.
  const LEGAL_LINKS = () => tr('Документы: {terms} и {privacy}.', {
    terms: `<a href="${LEGAL}/offer" target="_blank" rel="noopener">${tr('Условия использования')}</a>`,
    privacy: `<a href="${LEGAL}/privacy" target="_blank" rel="noopener">${tr('Политика конфиденциальности')}</a>`,
  });
  // Ряд из семи шаров над выбором: номера шаров из web/avatar.js.
  const ORB_ROW = [1, 4, 0, 5, 9, 11, 7];
  const CHEVRON = DeloistUI.icon('chevron-right', { className: 'i team-chev' });

  function chooseTeam() {
    const box = document.createElement('div');
    box.id = 'team-pick';
    box.className = 'signin team-pick';
    box.innerHTML = `
      <div class="team-card" role="dialog" aria-modal="true" aria-labelledby="team-title">
        <div class="team-mascots" aria-hidden="true">
          ${ORB_ROW.map((skin, i) => `<span class="team-mascot" data-skin="${skin}" style="--i:${i - (ORB_ROW.length - 1) / 2}"></span>`).join('')}
        </div>
        <h1 id="team-title">${tr('Собери свою ИИ‑команду.')}</h1>
        <p class="signin-lead">${tr('Выбери, на чём будут работать твои боты. Поменять можно в любой момент в настройках.')}</p>
        <div class="team-options">
          <button type="button" class="team-option" data-plan="deloist">
            <span class="team-text"><b>${tr('Подписка Deloist')}</b>
              <span>${tr('100 ₽ в подарок, чтобы попробовать. Ничего настраивать не нужно.')}</span></span>${CHEVRON}
          </button>
          <button type="button" class="team-option" data-plan="own">
            <span class="team-text"><b>${tr('Подключить свою ИИ')}</b>
              <span>${tr('Если у тебя уже есть ChatGPT, Claude или другой ИИ')}</span></span>${CHEVRON}
          </button>
        </div>
        <p class="signin-error" id="team-error" role="alert" hidden></p>
        <p class="signin-legal">${LEGAL_LINKS()}</p>
      </div>`;
    document.body.appendChild(box);
    // Без WebGL2 ряд просто пустой.
    box.querySelectorAll('.team-mascot').forEach((el) => {
      const size = 88 - Math.abs(Number(el.style.getPropertyValue('--i'))) * 14;
      if (!window.CrewAvatar?.mount(el, { size, skin: Number(el.dataset.skin), color: getComputedStyle(document.documentElement).getPropertyValue('--k-bot-tangerine').trim(), still: true })) el.remove();
    });

    return new Promise((resolve) => {
      let busy = false;
      box.querySelectorAll('[data-plan]').forEach((btn) => btn.addEventListener('click', async () => {
        if (busy) return;
        busy = true;
        const error = box.querySelector('#team-error');
        error.hidden = true;
        btn.classList.add('is-busy');
        try {
          const res = await api('/api/account/plan', { method: 'POST', body: { plan: btn.dataset.plan } });
          window.DeloistAccount = { ...window.DeloistAccount, onboarded: true, giftDenied: Boolean(res.giftDenied) };
          box.remove();
          resolve();
          // Сервер завёл «Главную команду» с оркестратором: окно открывает её сцену.
          window.dispatchEvent(new Event('deloist:team-chosen'));
          // Своя подписка: сразу к настройкам модели, когда окно загрузится.
          if (res.plan === 'own') window.addEventListener('deloist:ready', () => window.DeloistProviders?.open(), { once: true });
        } catch (err) {
          error.textContent = trServer(err.message);
          error.hidden = false;
        } finally {
          busy = false;
          btn.classList.remove('is-busy');
        }
      }));
    });
  }

  async function signOut() {
    await api('/api/account/logout', { method: 'POST' });
    location.reload();
  }

  return { ensure, signOut, askConsent };
})();
