// Поддельный голос для дока: только в отладочной сборке (сервер отдаёт файл под cfg(debug_assertions)).
// Нужен, пока настоящий голос (ветка горячих клавиш) не влит, и для проверки дока без микрофона.
// Говорит тем же договором, что настоящий: зовёт window.dockVoice({ type: 'voice_state', state, botId, level, progress, error }).
// Настоящий window.DeloistVoice, если он есть, не трогаем.
(() => {
  if (window.DeloistVoice?.toggle) return;
  let state = 'idle';
  let modelReady = false;
  let timer = 0;
  let levels = 0;
  const emit = (patch) => window.dockVoice?.({ type: 'voice_state', ...patch });
  const target = () => window.dockVoiceTarget?.() || '';
  function stop() {
    clearTimeout(timer);
    clearInterval(levels);
    levels = 0;
  }
  function set(next, extra = {}) {
    state = next;
    emit({ state: next, botId: target(), ...extra });
  }
  function record() {
    set('recording', { level: 0 });
    let level = 0;
    let goal = 0;
    let until = 0;
    levels = setInterval(() => {
      const now = performance.now();
      if (now > until) {
        const speaking = Math.random() < 0.72;
        goal = speaking ? 0.35 + Math.random() * 0.65 : 0;
        until = now + (speaking ? 140 + Math.random() * 220 : 120 + Math.random() * 380);
      }
      level += (goal - level) * 0.45;
      emit({ state: 'recording', botId: target(), level: Math.round(level * 1000) / 1000 });
    }, 33);
  }
  function download() {
    let progress = 0;
    const tick = () => {
      progress = Math.min(1, progress + 0.05);
      set('downloading', { progress });
      if (progress < 1) {
        timer = setTimeout(tick, 150);
        return;
      }
      set('loading');
      timer = setTimeout(() => {
        modelReady = true;
        record();
      }, 900);
    };
    tick();
  }
  function finish() {
    stop();
    set('transcribing');
    timer = setTimeout(() => {
      if (window.dockFakeVoiceError === 'money') {
        set('error', { error: 'money' });
        return;
      }
      set('sending');
      timer = setTimeout(() => {
        set('done');
        timer = setTimeout(() => set('idle'), 700);
      }, 500);
    }, 1200);
  }
  window.DeloistVoice = {
    fake: true,
    toggle() {
      if (state === 'idle' || state === 'error') {
        stop();
        if (window.dockFakeVoiceError === 'mic') set('error', { error: 'mic' });
        else if (!modelReady) download();
        else record();
      } else if (state === 'recording') {
        finish();
      } else if (state === 'downloading') {
        stop();
        set('idle');
      }
    },
    cancel() {
      stop();
      set('idle');
    },
    // Чтение вслух: window.DeloistVoice.speak(botId, seconds) шлёт уровень речи 20 раз в секунду, потом «spoken».
    speak(botId = target(), seconds = 4) {
      stopSpeaking();
      let level = 0;
      let goal = 0;
      let until = 0;
      const end = performance.now() + seconds * 1000;
      speech = setInterval(() => {
        const now = performance.now();
        if (now > end) {
          stopSpeaking(botId);
          return;
        }
        if (now > until) {
          const voiced = Math.random() < 0.8;
          goal = voiced ? 0.3 + Math.random() * 0.7 : 0;
          until = now + (voiced ? 120 + Math.random() * 200 : 80 + Math.random() * 200);
        }
        level += (goal - level) * 0.5;
        emit({ state: 'speaking', botId, level: Math.round(level * 1000) / 1000 });
      }, 50);
    },
    stopSpeaking(botId) {
      stopSpeaking(botId);
    },
  };
  let speech = 0;
  function stopSpeaking(botId) {
    if (!speech) return;
    clearInterval(speech);
    speech = 0;
    emit({ state: 'spoken', botId: botId || '' });
  }
})();
