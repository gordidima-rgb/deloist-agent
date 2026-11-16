// Карточка хода бота: появляется в ленте в момент отправки, до первого события с сервера, и дальше
// только дополняется теми же событиями. Аватар, имя, «Думает… 12 с» с живым счётчиком и кнопка «Стоп».
//
//   CrewTurnCard.create(deps) → контроллер:
//     begin({ key, bot, name, phase })  — показать карточку (повторный вызов тот же ход не сбрасывает);
//     phase('choosing' | 'thinking')    — подпись до первого действия;
//     status(text)                      — что бот делает сейчас («Читает файл…»);
//     note({ failed })                  — шаг закончился (failed — с ошибкой);
//     fail(key, message)                — отправка не удалась: карточка становится ошибкой, а не крутится;
//     end(key) → { ms, steps, failed }  — ход кончился, карточка уходит; счётчик секунд виден в итоговой строке;
//     forget(key), detach(), toEnd(), has(key), current().
//
// Секунды считает клиент от нажатия «Отправить»: без запросов к серверу, одним setInterval в секунду.
// Все подписи приходят в deps.labels (их делает app.js через tr()), русских строк здесь нет.
(() => {
  const PIX = '<span class="k-pix k-pix--paper k-pix--think" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>';
  const STOP = DeloistUI.icon('stop-large', { size: 12, stroke: 2.5 });

  function create(deps) {
    const { root, labels, mountAvatar, onStop } = deps;
    const now = deps.now || (() => Date.now());
    const doc = deps.doc || document;
    // Когда нажали «Отправить»: переживает перерисовку ленты (человек ушёл в другой чат и вернулся).
    const started = new Map();
    let card = null;
    let timer = 0;

    const fmt = (ms) => {
      const total = Math.max(0, Math.floor(ms / 1000));
      return total < 60
        ? labels.sec.replace('{n}', String(total))
        : labels.min.replace('{m}', String(Math.floor(total / 60))).replace('{n}', String(total % 60));
    };

    const elapsed = () => (card ? now() - card.startedAt : 0);

    function paintTime() {
      if (!card) return;
      const text = fmt(elapsed());
      if (card.time.textContent !== text) card.time.textContent = text;
    }

    function stopTimer() {
      if (timer) clearInterval(timer);
      timer = 0;
    }

    function paintStatus(raw) {
      // Подпись шага — одна короткая строка: ни кода команды, ни JSON, ни переводов строк.
      const text = String(raw ?? '').replace(/\s+/g, ' ').trim().slice(0, 100);
      card.statusText = text;
      const shown = card.state === 'live' ? text + '…' : text;
      if (card.status.textContent !== shown) card.status.textContent = shown;
    }

    function paintWho(bot, name) {
      const who = name || bot?.name || '';
      if (card.name.textContent !== who) card.name.textContent = who;
      card.name.hidden = !who;
      card.el.setAttribute('aria-label', who);
      const id = bot?.id || '';
      if (card.botId === id && (card.avatar || !id)) return;
      card.botId = id;
      card.avatar?.destroy?.();
      card.avatar = null;
      card.orb.innerHTML = '';
      const handle = bot ? mountAvatar(card.orb, bot) : null;
      if (handle) card.avatar = handle;
      else card.orb.innerHTML = PIX;
    }

    function release() {
      stopTimer();
      card?.avatar?.destroy?.();
      card?.el.remove();
      card = null;
    }

    function build(key, startedAt) {
      const el = doc.createElement('div');
      el.className = 'turn-card';
      el.dataset.state = 'live';
      el.setAttribute('role', 'group');
      el.innerHTML = '<span class="tc-orb"></span><div class="tc-main"><div class="tc-name"></div>'
        + '<div class="tc-line"><span class="tc-status" aria-live="polite"></span><span class="tc-time" aria-hidden="true"></span></div></div>'
        + '<button type="button" class="tc-stop"></button>';
      const stop = el.querySelector('.tc-stop');
      stop.innerHTML = STOP + '<span></span>';
      stop.querySelector('span').textContent = labels.stop;
      stop.setAttribute('aria-label', labels.stopHint);
      stop.addEventListener('click', () => onStop?.());
      return {
        key, el, startedAt, state: 'live', botId: null, avatar: null, steps: 0, failed: 0, statusText: '',
        orb: el.querySelector('.tc-orb'), name: el.querySelector('.tc-name'),
        status: el.querySelector('.tc-status'), time: el.querySelector('.tc-time'), stop,
      };
    }

    return {
      begin({ key, bot = null, name = '', phase = 'thinking' }) {
        if (card && card.key !== key) release();
        if (card?.state === 'error') release();
        if (!card) {
          if (!started.has(key)) started.set(key, now());
          card = build(key, started.get(key));
          root().append(card.el);
          paintWho(bot, name);
          paintStatus(labels[phase] || labels.thinking);
          paintTime();
          timer = setInterval(paintTime, 1000);
          return { created: true };
        }
        if (!card.el.isConnected) root().append(card.el);
        if (bot || name) paintWho(bot, name);
        return { created: false };
      },

      phase(name) {
        if (card?.state === 'live') paintStatus(labels[name] || labels.thinking);
      },

      status(text) {
        if (card?.state === 'live' && text) paintStatus(text);
      },

      note({ failed = false } = {}) {
        if (!card) return;
        card.steps += 1;
        if (failed) card.failed += 1;
      },

      fail(key, message) {
        if (!card || card.key !== key) return;
        stopTimer();
        started.delete(key);
        card.state = 'error';
        card.el.dataset.state = 'error';
        card.status.textContent = message ? labels.failed + ': ' + message : labels.failed;
        card.status.setAttribute('role', 'alert');
        card.time.textContent = '';
        card.stop.hidden = true;
      },

      end(key) {
        started.delete(key);
        if (!card || card.key !== key) return null;
        const info = { ms: elapsed(), steps: card.steps, failed: card.failed };
        release();
        return info;
      },

      // Ход чужого чата кончился, а карточки на экране нет: только забыть время старта.
      forget(key) {
        started.delete(key);
      },

      // Лента перерисована (другой чат): таймер и аватар отпускаем, время старта помним.
      detach() {
        if (!card) return;
        stopTimer();
        card.avatar?.destroy?.();
        card = null;
      },

      // Сообщение человека пришло уже после карточки: карточка должна остаться под ним.
      toEnd() {
        if (card?.el.isConnected) root().append(card.el);
      },

      has: (key) => Boolean(card && card.key === key),
      current: () => (card ? { key: card.key, state: card.state, elapsed: elapsed(), steps: card.steps, failed: card.failed } : null),
      el: () => card?.el || null,
    };
  }

  window.CrewTurnCard = { create };
})();
