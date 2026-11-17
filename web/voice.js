// Голос: микрофон → 16 кГц Int16 PCM → распознавание через программу (токен аккаунта остаётся в ней).
// Рекордер перенесён с сайта Deloist (frontend/src/lib/voice/recorder.ts и pcm.ts), где он работает в бою.
// Здесь же кнопка микрофона в чате и в мини-чате дока, снимок экрана в доке и настройки горячих клавиш.
(() => {
  const tr = (key, params) => (window.tr ? window.tr(key, params) : key);
  const $ = (id) => document.getElementById(id);

  // --- PCM ------------------------------------------------------------------

  /** Частота для сайта; SpeechKit принимает 8000 / 16000 / 48000. */
  const TARGET_RATE = 16000;
  /** Предел синхронного SpeechKit: запись останавливается сама. */
  const MAX_SECONDS = 30;
  /** Короче — случайное нажатие, а не речь; сайт такое тоже отвергает. */
  const MIN_SECONDS = 0.3;
  /**
   * Остановка по тишине, как у голосового ввода в телефоне: после речи SILENCE_MS тишины,
   * а если не сказали ничего — LEAD_IN_MS. Часы — счётчик отсчётов, а не настенное время.
   * SPEECH_RMS выше шума после шумоподавления браузера (~0.001–0.005) и ниже тихой речи (~0.03).
   */
  const SILENCE_MS = 2000;
  const LEAD_IN_MS = 6000;
  const SPEECH_RMS = 0.012;
  const MIC_PROMPT_TIMEOUT_MS = 60000;
  const CONTEXT_RESUME_TIMEOUT_MS = 2500;

  /** Усреднение по окну: дёшево, и для речи при 3:1 (48 → 16 кГц) этого сглаживания хватает. */
  function downsample(input, fromRate, toRate) {
    if (fromRate === toRate) return input;
    if (fromRate < toRate) throw new Error('upsampling is not supported');
    const ratio = fromRate / toRate;
    const outLen = Math.floor(input.length / ratio);
    const out = new Float32Array(outLen);
    for (let i = 0; i < outLen; i++) {
      const start = Math.floor(i * ratio);
      const end = Math.min(Math.floor((i + 1) * ratio), input.length);
      let sum = 0;
      for (let j = start; j < end; j++) sum += input[j];
      out[i] = end > start ? sum / (end - start) : 0;
    }
    return out;
  }

  /** Float32 [-1, 1] → Int16 little-endian с ограничением. */
  function encodePcm16(samples) {
    const out = new Int16Array(samples.length);
    for (let i = 0; i < samples.length; i++) {
      const s = Math.max(-1, Math.min(1, samples[i]));
      out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
    }
    return out;
  }

  function concatPcm(chunks) {
    let total = 0;
    for (const c of chunks) total += c.length;
    const out = new Int16Array(total);
    let offset = 0;
    for (const c of chunks) {
      out.set(c, offset);
      offset += c.length;
    }
    return out;
  }

  function rms(samples) {
    if (samples.length === 0) return 0;
    let sum = 0;
    for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
    return Math.sqrt(sum / samples.length);
  }

  // --- Рекордер -------------------------------------------------------------

  class RecorderStartError extends Error {
    constructor(reason) {
      super(reason);
      this.name = 'RecorderStartError';
      this.reason = reason;
    }
  }

  function audioContextCtor() {
    return window.AudioContext || window.webkitAudioContext;
  }

  function withTimeout(promise, ms) {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new RecorderStartError('timeout')), ms);
      promise.then(
        (v) => {
          clearTimeout(t);
          resolve(v);
        },
        (e) => {
          clearTimeout(t);
          reject(e);
        },
      );
    });
  }

  function isVoiceSupported() {
    return Boolean(navigator.mediaDevices?.getUserMedia) && audioContextCtor() !== undefined;
  }

  /**
   * Порядок важен (на сайте это ловили на Android Chrome): AudioContext создаётся и запускается
   * синхронно в обработчике нажатия, до любого await, потом микрофон, и каждое ожидание ограничено.
   * Горячая клавиша нажатия в странице не даёт: для этого окна дока и подсказки запускаются
   * с --autoplay-policy=no-user-gesture-required.
   */
  class PcmRecorder {
    constructor(events = {}) {
      this.events = events;
      this.rate = TARGET_RATE;
      this.ctx = null;
      this.stream = null;
      this.source = null;
      this.node = null;
      this.mute = null;
      this.chunks = [];
      this.samples = 0;
      this.stopped = false;
      this.limitFired = false;
      this.spoke = false;
      this.lastVoiceAt = 0;
      this.silenceFired = false;
    }

    get seconds() {
      return this.samples / this.rate;
    }

    /** Была ли речь громче порога: тишину на сайт не отправляем, это платный запрос. */
    get heard() {
      return this.spoke;
    }

    start() {
      const AC = audioContextCtor();
      if (!AC || !navigator.mediaDevices?.getUserMedia) return Promise.reject(new RecorderStartError('unsupported'));
      this.stopped = false;
      this.limitFired = false;
      this.spoke = false;
      this.lastVoiceAt = 0;
      this.silenceFired = false;
      this.chunks = [];
      this.samples = 0;
      const ctx = new AC();
      this.ctx = ctx;
      const resuming = ctx.state === 'running' ? Promise.resolve() : ctx.resume().catch(() => undefined);
      return this.acquire(ctx, resuming);
    }

    async acquire(ctx, resuming) {
      try {
        this.stream = await withTimeout(
          navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true }, video: false }),
          MIC_PROMPT_TIMEOUT_MS,
        );
      } catch (e) {
        await this.teardown();
        throw e;
      }
      if (this.ctx !== ctx) {
        this.stream?.getTracks().forEach((t) => t.stop());
        this.stream = null;
        throw new RecorderStartError('audio_blocked');
      }
      await withTimeout(resuming, CONTEXT_RESUME_TIMEOUT_MS).catch(() => undefined);
      if (ctx.state !== 'running') await withTimeout(ctx.resume(), CONTEXT_RESUME_TIMEOUT_MS).catch(() => undefined);
      if (ctx.state !== 'running') {
        await this.teardown();
        throw new RecorderStartError('audio_blocked');
      }
      this.source = ctx.createMediaStreamSource(this.stream);
      this.node = ctx.createScriptProcessor(4096, 1, 1);
      // ScriptProcessor тикает, только пока подключён к выходу; нулевая громкость не даёт эха.
      this.mute = ctx.createGain();
      this.mute.gain.value = 0;
      const ctxRate = ctx.sampleRate;
      this.node.onaudioprocess = (e) => {
        if (this.stopped) return;
        const input = e.inputBuffer.getChannelData(0);
        const level = rms(input);
        this.events.onlevel?.(level);
        const ds = downsample(input, ctxRate, this.rate);
        this.chunks.push(encodePcm16(ds));
        this.samples += ds.length;
        if (level >= SPEECH_RMS) {
          this.spoke = true;
          this.lastVoiceAt = this.samples;
        } else if (!this.silenceFired) {
          const quietMs = ((this.samples - this.lastVoiceAt) / this.rate) * 1000;
          if (quietMs >= (this.spoke ? SILENCE_MS : LEAD_IN_MS)) {
            this.silenceFired = true;
            this.stopped = true;
            this.events.onsilence?.();
            return;
          }
        }
        if (this.samples >= this.rate * MAX_SECONDS && !this.limitFired) {
          this.limitFired = true;
          this.stopped = true;
          this.events.onlimit?.();
        }
      };
      this.source.connect(this.node);
      this.node.connect(this.mute);
      this.mute.connect(ctx.destination);
    }

    async stop() {
      this.stopped = true;
      await this.teardown();
      const all = concatPcm(this.chunks);
      this.chunks = [];
      return { pcm: all.buffer, seconds: all.length / this.rate };
    }

    async cancel() {
      this.stopped = true;
      this.chunks = [];
      await this.teardown();
    }

    async teardown() {
      this.node?.disconnect();
      this.source?.disconnect();
      this.mute?.disconnect();
      if (this.node) this.node.onaudioprocess = null;
      this.stream?.getTracks().forEach((t) => t.stop());
      const ctx = this.ctx;
      this.node = this.source = this.mute = null;
      this.stream = null;
      this.ctx = null;
      if (ctx && ctx.state !== 'closed') {
        try {
          await ctx.close();
        } catch {
          // уже закрыт
        }
      }
    }
  }

  // --- Распознавание --------------------------------------------------------

  function toBase64(buffer) {
    const bytes = new Uint8Array(buffer);
    let text = '';
    for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(text);
  }

  async function transcribe(pcm, rate) {
    const res = await fetch('/api/voice/transcribe', {
      method: 'POST',
      cache: 'no-store',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ pcm: toBase64(pcm), rate, lang: window.LANG === 'en' ? 'en' : 'ru' }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(serverText(data.error) || `HTTP ${res.status}`);
    return String(data.text || '').trim();
  }

  /** Ошибки сервера приходят по-русски: переводим словарём и шаблонами (i18n.js). */
  function serverText(text) {
    return window.trServer ? window.trServer(text) : text;
  }

  function failureText(err) {
    const name = err?.name;
    if (name === 'NotAllowedError' || name === 'SecurityError') return tr('Нет доступа к микрофону. Разреши его в параметрах Windows: Конфиденциальность → Микрофон');
    if (name === 'NotFoundError' || name === 'OverconstrainedError') return tr('Микрофон не найден');
    if (name === 'NotReadableError') return tr('Микрофон занят другой программой');
    if (err?.reason === 'unsupported') return tr('Запись голоса здесь не работает');
    if (err?.reason === 'audio_blocked' || err?.reason === 'timeout') return tr('Не получилось включить микрофон. Попробуй ещё раз');
    return err?.message || tr('Не получилось распознать речь');
  }

  const MIC_ICON =
    DeloistUI.icon('mic-wide', { className: 'i' });

  /**
   * Кнопка микрофона: щелчок — слушать, второй щелчок — готово. Сама останавливается по тишине
   * и на 30 секундах. Распознанный текст отдаёт в onText.
   */
  function voiceButton({ button, onText, onError, onState, report, hotkey = false }) {
    let recorder = null;
    let state = 'idle';
    const titles = {
      listening: tr('Слушаю… щёлкни, когда закончишь'),
      working: tr('Распознаю…'),
      downloading: tr('Скачиваю модель речи… щёлкни, чтобы отменить'),
      loading: tr('Загружаю модель речи…'),
    };
    // В подсказке у кнопки — нынешнее сочетание голоса из настроек, а не записанное в коде.
    const idle = () => {
      const voice = hotkey ? config?.voice : null;
      return voice?.enabled && voice.keys ? `${tr('Сказать голосом')} · ${voice.keys}` : tr('Сказать голосом');
    };
    const set = (next) => {
      state = next;
      button.dataset.state = next;
      const title = next === 'idle' ? idle() : titles[next];
      button.title = title;
      button.setAttribute('aria-label', title);
      if (next !== 'listening') button.style.setProperty('--level', '0');
      onState?.(next);
    };

    async function finish() {
      if (state !== 'listening' || !recorder) return;
      const rec = recorder;
      recorder = null;
      set('working');
      report?.('transcribing');
      try {
        const { pcm, seconds } = await rec.stop();
        if (!rec.heard || seconds < MIN_SECONDS) {
          set('idle');
          report?.('done');
          onError?.(tr('Ничего не расслышал. Скажи ещё раз'));
          return;
        }
        const text = await transcribe(pcm, rec.rate);
        set('idle');
        if (text) {
          if (hotkey && config?.sendNow) report?.('sending');
          await onText(text);
          report?.('done');
        } else {
          report?.('done');
          onError?.(tr('Ничего не расслышал. Скажи ещё раз'));
        }
      } catch (err) {
        set('idle');
        report?.('error');
        onError?.(failureText(err));
      }
    }

    // Распознавание на компьютере (stt.js): при первом нажатии модель скачивается, кнопка
    // показывает ход, потом запись начинается сама. Повторный щелчок во время скачивания — отмена.
    let preparing = false;
    async function start() {
      if (state !== 'idle' || preparing) return;
      const stt = window.DeloistStt;
      if (stt) {
        preparing = true;
        let ready = false;
        try {
          ready = await stt.prepare((step, percent) => {
            if (state !== step) set(step);
            button.style.setProperty('--progress', (percent / 100).toFixed(2));
          });
        } catch (err) {
          preparing = false;
          set('idle');
          onError?.(failureText(err));
          return;
        }
        preparing = false;
        if (state !== 'idle') set('idle');
        if (!ready) return;
      }
      record();
    }

    function record() {
      const rec = new PcmRecorder({
        onlevel: (level) => {
          const shown = Math.min(1, level * 14);
          button.style.setProperty('--level', shown.toFixed(2));
          report?.('recording', shown);
        },
        onsilence: () => finish(),
        onlimit: () => finish(),
      });
      recorder = rec;
      set('listening');
      report?.('recording', 0);
      rec.start().catch((err) => {
        if (recorder === rec) recorder = null;
        set('idle');
        report?.('error');
        onError?.(failureText(err));
      });
    }

    async function cancel() {
      const rec = recorder;
      recorder = null;
      if (state === 'listening') {
        set('idle');
        report?.('done');
      }
      await rec?.cancel();
    }

    if (hotkey) micButtons.add(() => state === 'idle' && set('idle'));
    button.addEventListener('click', (event) => {
      event.preventDefault();
      if (state === 'idle') start();
      else if (state === 'listening') finish();
      else if (state === 'downloading' || state === 'loading') window.DeloistStt?.cancel();
    });
    set('idle');
    return {
      start,
      finish,
      cancel,
      get state() {
        return state;
      },
    };
  }

  /**
   * Шаги записи в общий поток событий (`voice_state`): док рисует по ним состояние у аватара бота.
   * Уровень — не чаще 20 раз в секунду (и сервер режет так же), текста записи здесь нет никогда.
   */
  function stateReporter(botOf) {
    let last = 0;
    let lastState = '';
    return (state, level) => {
      const botId = botOf();
      if (!botId) return;
      const now = performance.now();
      // Тишина (уровень 0) режется так же, как речь: иначе каждый кадр уходил бы на сервер.
      if (state === lastState && state === 'recording' && now - last < 50) return;
      last = now;
      lastState = state;
      const body = level === undefined ? { state, botId } : { state, botId, level };
      fetch('/api/voice/state', { method: 'POST', cache: 'no-store', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }).catch(() => {});
    };
  }

  function makeMicButton(className) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = className;
    button.innerHTML = MIC_ICON;
    return button;
  }

  /** Дописать распознанное в поле: через пробел, и поле узнаёт о правке как от клавиатуры. */
  function insertText(input, text) {
    const before = input.value.replace(/\s+$/, '');
    input.value = before ? `${before} ${text}` : text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.selectionStart = input.selectionEnd = input.value.length;
  }

  let config = null;
  const micButtons = new Set();
  const refreshTitles = () => micButtons.forEach((refresh) => refresh());
  async function loadConfig() {
    try {
      const res = await fetch('/api/hotkeys', { cache: 'no-store' });
      if (res.ok) config = await res.json();
    } catch {
      // сервер перезапускается: останутся прежние настройки
    }
    refreshTitles();
    return config;
  }

  function injectStyle() {
    const style = document.createElement('style');
    style.textContent = `
      .mic-btn { position: relative; transition: box-shadow 80ms linear, background 160ms; }
      .mic-btn.attach[data-state="idle"]:hover { border-color: var(--color-border-strong); color: var(--color-text-primary); }
      .mic-btn.chat-send { background: transparent; color: var(--color-text-secondary); }
      .mic-btn.chat-send:hover { background: var(--color-bg-surface-soft); }
      .mic-btn[data-state="listening"], .mic-btn[data-state="listening"]:hover {
        background: var(--color-accent); color: var(--color-on-accent); border-color: transparent;
        box-shadow: 0 0 0 calc(2px + var(--level, 0) * 9px) color-mix(in srgb, var(--color-accent) 28%, transparent);
      }
      .mic-btn[data-state="working"] { opacity: 0.6; cursor: progress; }
      .dock-mic { border-style: solid; }
      .mic-btn:disabled { opacity: 0.4; cursor: not-allowed; }
      .voice-note {
        position: fixed; left: 50%; bottom: 96px; transform: translateX(-50%); z-index: 50; max-width: min(520px, 90vw);
        padding: 8px 14px; border-radius: 12px; font-size: 13px; line-height: 1.35;
        background: var(--color-text-primary); color: var(--color-bg-surface); box-shadow: 0 6px 20px rgba(0,0,0,.18);
      }
      .chat .voice-note { position: absolute; bottom: 76px; width: max-content; max-width: 86%; }
      .snap-chip { display: flex; align-items: center; gap: 8px; margin: 0 12px 6px; padding: 6px 8px; border-radius: 12px;
        background: var(--color-bg-surface-soft); border: 1px solid var(--color-border-default); font-size: 13px; }
      .snap-chip img { width: 44px; height: 32px; object-fit: cover; border-radius: 6px; flex: none; }
      .snap-chip span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .snap-chip button { border: none; background: transparent; color: var(--color-text-muted); cursor: pointer; font-size: 16px; padding: 0 4px; }
      .voice-side { display: flex; flex-direction: column; align-items: flex-end; gap: 4px; flex: none; }
      .voice-keys { display: flex; align-items: center; gap: 6px; }
      .voice-combo { min-width: 118px; height: 34px; padding: 0 12px; border-radius: var(--radius-md); font-variant-numeric: tabular-nums; }
      .voice-combo.capturing { border-color: var(--color-brand-primary); color: var(--color-brand-text); }
      .voice-combo.off { color: var(--color-text-muted); }
      .voice-small { height: 34px; padding: 0 10px; border-radius: var(--radius-md); font-size: 13px; }
      .voice-small[hidden] { display: none; }
      .voice-status { color: var(--color-accent-text); font-size: 13px; text-align: right; max-width: 260px; }
      .voice-status:empty { display: none; }
    `;
    document.head.append(style);
  }

  function noteIn(parent) {
    let timer = 0;
    return (text, ms = 4500) => {
      let note = parent.querySelector(':scope > .voice-note');
      if (!note) {
        note = document.createElement('div');
        note.className = 'voice-note';
        note.setAttribute('role', 'status');
        parent.append(note);
      }
      note.textContent = text;
      clearTimeout(timer);
      timer = setTimeout(() => note.remove(), ms);
    };
  }

  // --- Мини-чат дока --------------------------------------------------------

  function setupDock() {
    const form = $('chat-form');
    const input = $('chat-input');
    const chat = $('chat');
    const hooks = () => window.dockHooks || {};
    const note = noteIn(chat);
    const button = makeMicButton('chat-send mic-btn');
    form.insertBefore(button, $('chat-send'));
    const mic = voiceButton({
      button,
      hotkey: true,
      report: stateReporter(() => hooks().openId?.()),
      onText: async (text) => {
        if (!hooks().openId?.()) return;
        insertText(input, text);
        if (config?.sendNow) await hooks().send?.();
        else input.focus();
      },
      onError: note,
    });

    // Микрофон на самом доке (не в чате): голос уходит боту из настройки «Голос уходит».
    // Место под него даёт dock.js (`mount`); потом его займёт кнопка из Svelte.
    const dockMic = makeMicButton('add mic-btn dock-mic');
    dockMic.addEventListener('click', async (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (mic.state === 'listening') {
        mic.finish();
        return;
      }
      try {
        const res = await fetch('/api/voice/target', { method: 'POST', cache: 'no-store' });
        const { botId } = await res.json();
        if (botId) openAndListen(botId);
      } catch {
        // сервер перезапускается — щёлкнут ещё раз
      }
    });
    micButtons.add(() => {
      const voice = config?.voice;
      dockMic.title = voice?.enabled && voice.keys ? `${tr('Сказать голосом')} · ${voice.keys}` : tr('Сказать голосом');
      dockMic.setAttribute('aria-label', dockMic.title);
    });
    // dock.js грузится после voice.js: место на доке появится в следующей задаче.
    setTimeout(() => hooks().mount?.('mic', dockMic), 0);

    // Снимок экрана ждёт отправки рядом с полем ввода.
    let attached = null; // { botId, path }
    const chip = document.createElement('div');
    chip.className = 'snap-chip';
    chip.hidden = true;
    form.before(chip);
    function renderChip() {
      const show = Boolean(attached) && attached.botId === hooks().openId?.();
      chip.hidden = !show;
      if (!show) return;
      chip.replaceChildren();
      const img = document.createElement('img');
      img.alt = '';
      img.src = `/api/bots/${encodeURIComponent(attached.botId)}/file/raw?path=${encodeURIComponent(attached.path)}`;
      const label = document.createElement('span');
      label.textContent = tr('Снимок экрана');
      label.title = attached.path;
      const drop = document.createElement('button');
      drop.type = 'button';
      drop.textContent = '×';
      drop.title = tr('Убрать снимок');
      drop.addEventListener('click', () => {
        attached = null;
        renderChip();
      });
      chip.append(img, label, drop);
    }

    // dock.js спрашивает текст перед отправкой: к нему добавляется путь к снимку.
    window.dockBeforeSend = () => {
      if (!attached || attached.botId !== hooks().openId?.()) return [];
      const paths = [attached.path];
      attached = null;
      renderChip();
      return paths;
    };
    // Отправка сорвалась: снимок возвращается к полю ввода.
    window.dockAfterFail = (botId, paths) => {
      if (paths?.length && !attached) {
        attached = { botId, path: paths[0] };
        renderChip();
      }
    };

    async function openAndListen(botId) {
      hooks().openChat?.(botId);
      renderChip();
      await loadConfig();
      // Мини-чат раскрывается анимацией: слушаем, когда поле уже на месте.
      setTimeout(() => {
        if (hooks().openId?.() === botId && mic.state === 'idle') mic.start();
      }, 260);
    }

    api.dockEvent = (event) => {
      if (event.target !== 'dock') return;
      if (event.type === 'voice_hotkey') openAndListen(event.botId);
      else if (event.type === 'snap' && event.attachment?.path) {
        attached = { botId: event.botId, path: event.attachment.path };
        openAndListen(event.botId);
      }
    };
    // Закрыли мини-чат — запись больше никому не нужна.
    new MutationObserver(() => {
      if (chat.hidden) mic.cancel();
      else firstRunHint((text) => note(text, 9000));
      renderChip();
    }).observe(chat, { attributes: true, attributeFilter: ['hidden'] });
    loadConfig();
  }

  // --- Главное окно ---------------------------------------------------------

  const ACTIONS = [
    { key: 'voice', title: 'Голос боту', sub: 'Открывает мини-чат последнего бота и сразу слушает. Текст попадает в поле ввода.' },
    { key: 'snap', title: 'Покажи на экране', sub: 'Обведи область экрана мышью: снимок уйдёт боту вместе с голосовым вопросом.' },
    { key: 'edit', title: 'Правка выделенного текста', sub: 'Выдели текст в любой программе, нажми сочетание и скажи, что поменять: «сделай вежливее». В полях пароля не работает.' },
  ];
  const STATUS = {
    taken: 'Занято другой программой',
    invalid: 'Сочетание не подходит',
    unavailable: 'Горячие клавиши работают, когда Deloist открыт как приложение.',
  };
  const CHEVRON =
    DeloistUI.icon('chevron-right', { className: 'i t-chev' });

  const MODIFIER_CODES = new Set(['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight', 'OSLeft', 'OSRight']);

  /** Клавиша из события в том виде, в каком её понимает программа. */
  function keyName(event) {
    const code = event.code || '';
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
    const named = { Space: 'Space', Tab: 'Tab', Insert: 'Insert', Delete: 'Delete', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', Pause: 'Pause' };
    if (named[code]) return named[code];
    // Без кода клавиши (экранная клавиатура, программы ввода) — по самой букве.
    if (!code && /^[a-z0-9]$/i.test(event.key || '')) return event.key.toUpperCase();
    if (!code && event.key === ' ') return 'Space';
    return '';
  }

  /** Модификаторы, зажатые сейчас: их видно в поле, пока человек ищет нужную клавишу. */
  function modifiersOf(event) {
    const parts = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    if (event.metaKey) parts.push('Win');
    return parts;
  }

  function comboOf(event) {
    const key = keyName(event);
    return key ? [...modifiersOf(event), key].join(' + ') : '';
  }

  function injectSettings() {
    const anchor = $('set-voice-anchor') || $('set-dock');
    if (!anchor) return;
    const group = document.createElement('details');
    group.className = 'group';
    group.id = 'set-hotkeys';
    const summary = document.createElement('summary');
    summary.innerHTML = CHEVRON;
    summary.append(tr('Горячие клавиши'));
    const body = document.createElement('div');
    body.className = 'group-body';
    const box = document.createElement('div');
    box.className = 'dock-box';
    const lead = document.createElement('p');
    lead.className = 'note';
    lead.textContent = tr('Щёлкни по полю и нажми новое сочетание: клавиша с Ctrl, Alt или Win либо одна из F13–F24. Работает сразу, без перезапуска.');
    body.append(lead, box);
    group.append(summary, body);
    anchor.after(group);

    const rows = {};
    const notes = {}; // ответ сервера на последнюю правку: «занято», «не подходит»
    for (const action of ACTIONS) {
      const row = document.createElement('div');
      row.className = 'prov-line';
      const text = document.createElement('div');
      const title = document.createElement('div');
      title.className = 'prov-line-title';
      title.textContent = tr(action.title);
      const sub = document.createElement('div');
      sub.className = 'prov-line-sub';
      sub.textContent = tr(action.sub);
      text.append(title, sub);
      const side = document.createElement('div');
      side.className = 'voice-side';
      const keys = document.createElement('div');
      keys.className = 'voice-keys';
      const combo = document.createElement('button');
      combo.type = 'button';
      combo.className = 'k-btn k-btn--ghost voice-combo';
      combo.title = tr('Щёлкни и нажми новое сочетание');
      const reset = document.createElement('button');
      reset.type = 'button';
      reset.className = 'k-btn k-btn--ghost voice-small';
      reset.textContent = tr('Сбросить');
      const toggle = document.createElement('button');
      toggle.type = 'button';
      toggle.className = 'k-btn k-btn--ghost voice-small';
      keys.append(combo, reset, toggle);
      const status = document.createElement('div');
      status.className = 'voice-status';
      status.setAttribute('role', 'status');
      side.append(keys, status);
      row.append(text, side);
      box.append(row);
      rows[action.key] = { combo, reset, toggle, status };

      reset.addEventListener('click', () => {
        const fallback = config?.defaults?.[action.key]?.keys;
        if (fallback) save(action.key, { [action.key]: { keys: fallback, enabled: true } });
      });
      toggle.addEventListener('click', () => {
        const on = Boolean(config?.[action.key]?.enabled);
        save(action.key, { [action.key]: { enabled: !on } });
      });

      let capturing = false;
      const stopCapture = () => {
        capturing = false;
        combo.classList.remove('capturing');
        render();
      };
      combo.addEventListener('click', () => {
        capturing = true;
        combo.classList.add('capturing');
        combo.textContent = tr('Нажми сочетание…');
      });
      combo.addEventListener('blur', () => capturing && stopCapture());
      combo.addEventListener('keyup', (event) => {
        if (!capturing) return;
        // Отпустили модификатор, не нажав клавишу: в поле снова то, что зажато сейчас.
        const held = modifiersOf(event);
        combo.textContent = held.length ? `${held.join(' + ')} + …` : tr('Нажми сочетание…');
      });
      combo.addEventListener('keydown', (event) => {
        if (!capturing) return;
        // Esc и Enter внутри окна настроек закрыли бы его: здесь они значат «отмена» и ничего.
        event.preventDefault();
        event.stopPropagation();
        if (event.key === 'Escape' && !modifiersOf(event).length) {
          stopCapture();
          return;
        }
        if (MODIFIER_CODES.has(event.code) || !keyName(event)) {
          // Пока только модификаторы: показываем их вживую.
          const held = modifiersOf(event);
          combo.textContent = held.length ? `${held.join(' + ')} + …` : tr('Нажми сочетание…');
          return;
        }
        const next = comboOf(event);
        capturing = false;
        combo.classList.remove('capturing');
        combo.textContent = next;
        save(action.key, { [action.key]: { keys: next, enabled: true } });
      });
    }

    // «Голос уходит»: кому идёт текст с Ctrl+Space и с микрофона на доке.
    const targetRow = document.createElement('div');
    targetRow.className = 'prov-line';
    const targetText = document.createElement('div');
    const targetTitle = document.createElement('div');
    targetTitle.className = 'prov-line-title';
    targetTitle.textContent = tr('Голос уходит');
    const targetSub = document.createElement('div');
    targetSub.className = 'prov-line-sub';
    targetSub.textContent = tr('Кому идёт текст с сочетания голоса и с микрофона на доке.');
    const targetNote = document.createElement('div');
    targetNote.className = 'voice-status';
    targetText.append(targetTitle, targetSub, targetNote);
    const targetSelect = document.createElement('select');
    targetSelect.className = 'dock-select';
    targetSelect.setAttribute('aria-label', tr('Голос уходит'));
    targetRow.append(targetText, targetSelect);
    box.append(targetRow);
    let bots = [];
    async function loadBots() {
      try {
        const res = await fetch('/api/state', { cache: 'no-store' });
        if (res.ok) bots = (await res.json()).bots || [];
      } catch {
        // останется прежний список
      }
    }
    targetSelect.addEventListener('change', () => save('target', { voiceTarget: targetSelect.value }));

    const sendRow = document.createElement('label');
    sendRow.className = 'prov-line';
    const sendText = document.createElement('div');
    const sendTitle = document.createElement('div');
    sendTitle.className = 'prov-line-title';
    sendTitle.textContent = tr('Отправлять сразу');
    const sendSub = document.createElement('div');
    sendSub.className = 'prov-line-sub';
    sendSub.textContent = tr('Распознанный текст уходит боту без проверки в поле ввода.');
    sendText.append(sendTitle, sendSub);
    const sendToggle = document.createElement('span');
    sendToggle.className = 'switch';
    const sendCheck = document.createElement('input');
    sendCheck.type = 'checkbox';
    sendToggle.append(sendCheck, document.createElement('span'));
    sendRow.append(sendText, sendToggle);
    box.append(sendRow);
    sendCheck.addEventListener('change', () => save(null, { sendNow: sendCheck.checked }));

    function render() {
      if (!config) return;
      for (const action of ACTIONS) {
        const row = rows[action.key];
        const binding = config[action.key] || {};
        const on = Boolean(binding.enabled);
        if (!row.combo.classList.contains('capturing')) row.combo.textContent = on ? binding.keys || '' : tr('Выключено');
        row.combo.classList.toggle('off', !on);
        row.toggle.textContent = on ? tr('Выключить') : tr('Включить');
        row.reset.hidden = on && binding.keys === config.defaults?.[action.key]?.keys;
        const status = config.status?.[action.key];
        // «Нет окна программы» касается всех сочетаний сразу: говорим об этом один раз.
        const once = status !== 'unavailable' || action === ACTIONS[0];
        row.status.textContent = notes[action.key] || (STATUS[status] && once ? tr(STATUS[status]) : '');
      }
      sendCheck.checked = Boolean(config.sendNow);
      const wanted = config.voiceTarget || 'last';
      const options = [{ id: 'last', name: tr('Последнему боту, с кем говорил') }, ...bots.map((bot) => ({ id: bot.id, name: bot.name }))];
      // Выбранного бота удалили: голос и так уйдёт последнему, а в списке видно, что выбор устарел.
      if (!options.some((o) => o.id === wanted)) options.push({ id: wanted, name: tr('Бот удалён — уйдёт последнему') });
      targetSelect.replaceChildren(
        ...options.map((o) => {
          const option = document.createElement('option');
          option.value = o.id;
          option.textContent = o.name;
          return option;
        }),
      );
      targetSelect.value = wanted;
      targetNote.textContent = notes.target || '';
    }

    async function save(key, patch) {
      for (const name of Object.keys(notes)) delete notes[name];
      try {
        const res = await fetch('/api/hotkeys', { method: 'PUT', cache: 'no-store', headers: { 'content-type': 'application/json' }, body: JSON.stringify(patch) });
        const data = await res.json().catch(() => ({}));
        if (!res.ok) throw new Error(serverText(data.error) || `HTTP ${res.status}`);
        config = data;
        // Новое сочетание оказалось занято: осталось прежнее рабочее, а рядом с полем — почему.
        for (const name of Object.keys(data.rejected || {})) notes[name] = tr('Занято другой программой. Оставил прежнее');
        refreshTitles();
      } catch (err) {
        if (key) notes[key] = err.message;
      }
      render();
    }

    group.addEventListener('toggle', async () => {
      if (!group.open) return;
      await Promise.all([loadConfig(), loadBots()]);
      render();
    });
  }

  /** Первый запуск: один раз рассказать, какие сочетания есть. Сочетания — нынешние, из настроек. */
  async function firstRunHint(show) {
    const KEY = 'deloist.hotkeysHint';
    try {
      if (localStorage.getItem(KEY)) return;
    } catch {
      return;
    }
    const current = await loadConfig();
    if (!current || current.status?.voice === 'unavailable') return;
    const parts = ACTIONS.filter((a) => current[a.key]?.enabled).map((a) => `${current[a.key].keys} — ${tr(a.title).toLowerCase()}`);
    if (!parts.length) return;
    show(tr('Горячие клавиши: {list}. Поменять можно в настройках.', { list: parts.join(' · ') }));
    try {
      localStorage.setItem(KEY, '1');
    } catch {
      // без памяти страницы подсказка просто покажется ещё раз
    }
  }

  function setupMain() {
    const input = $('input');
    const composer = input.closest('.composer');
    const note = noteIn(document.body);
    const button = makeMicButton('attach mic-btn');
    composer.insertBefore(button, $('btn-stop'));
    // Кнопка «Голосовой разговор» рядом с микрофоном.
    if (isVoiceSupported()) {
      const vcBtn = document.createElement('button');
      vcBtn.type = 'button';
      vcBtn.className = 'attach mic-btn';
      const vcLabel = tr('Голосовой разговор');
      vcBtn.title = vcLabel;
      vcBtn.setAttribute('aria-label', vcLabel);
      vcBtn.innerHTML = DeloistUI.icon('chat-wave', { className: 'i' });
      vcBtn.addEventListener('click', (e) => {
        e.preventDefault();
        window.DeloistVoiceChat?.openPanel?.();
      });
      composer.insertBefore(vcBtn, $('btn-stop'));
    }
    const mic = voiceButton({
      button,
      hotkey: true,
      report: stateReporter(() => window.DeloistApp?.currentBot?.()),
      onText: (text) => {
        insertText(input, text);
        if (config?.sendNow) $('btn-send')?.click();
        else input.focus();
      },
      onError: note,
    });
    // Без выбранного бота поле выключено — и микрофон вместе с ним.
    const follow = () => {
      button.disabled = input.disabled && mic.state === 'idle';
    };
    new MutationObserver(follow).observe(input, { attributes: true, attributeFilter: ['disabled'] });
    follow();

    // Дока нет: горячая клавиша открывает это окно, и оно слушает само.
    async function takePending() {
      try {
        const res = await fetch('/api/voice/pending', { method: 'POST', cache: 'no-store' });
        const { event } = await res.json();
        if (!event) return;
        await loadConfig();
        // Сначала чат того бота, у которого лежит снимок и которому адресован голос.
        if (event.botId) await window.DeloistApp?.selectBot?.(event.botId);
        if (event.type === 'snap' && event.attachment?.path) {
          const url = `/api/bots/${encodeURIComponent(event.botId)}/file/raw?path=${encodeURIComponent(event.attachment.path)}`;
          window.DeloistApp?.addSaved?.({ ...event.attachment, url });
        }
        if (!input.disabled && mic.state === 'idle') mic.start();
      } catch {
        // окно ещё грузится — заберёт при следующем событии
      }
    }
    // Вторая панель разделённого окна не забирает горячую клавишу и подсказку: они у самого окна.
    const inPane = Boolean(window.DELOIST_PANE);
    if (!inPane) try {
      const events = new EventSource('/api/events');
      events.addEventListener('message', ({ data }) => {
        try {
          const event = JSON.parse(data);
          if ((event.type === 'voice_hotkey' || event.type === 'snap') && event.target === 'main') takePending();
        } catch {
          // мусорное событие — пропускаем
        }
      });
    } catch {
      // без потока событий кнопка микрофона всё равно работает
    }
    // Окно из трея пересоздаёт WebView: страница грузится не сразу, поэтому спрашиваем ещё раз чуть позже.
    if (!inPane) {
      setTimeout(takePending, 1200);
      setTimeout(takePending, 4000);
    }
    injectSettings();
    loadConfig();
    if (!inPane) setTimeout(() => firstRunHint((text) => note(text, 9000)), 2500);
  }

  const api = { PcmRecorder, RecorderStartError, transcribe, voiceButton, isVoiceSupported, insertText, loadConfig, pcm: { downsample, encodePcm16, concatPcm, rms, toBase64 } };
  window.DeloistVoice = api;
  injectStyle();
  // Страница подсказки сама решает, что делать с записью.
  if (document.body?.dataset.voice !== 'manual') {
    if ($('chat-form') && $('chat-input')) setupDock();
    else if ($('input') && $('input').closest('.composer')) setupMain();
  }
})();
