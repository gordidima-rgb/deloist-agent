// Голосовой разговор («Голос»): hands-free панель поверх текущего бота.
(() => {
  const tr = (key, params) => (window.tr ? window.tr(key, params) : key);
  const $ = (id) => document.getElementById(id);

  function isVoiceSupported() {
    return Boolean(window.DeloistVoice?.isVoiceSupported?.()) && 'speechSynthesis' in window;
  }

  // --- Движок озвучки -----------------------------------------------------
  function pickEngine() {
    return createSpeechSynthesisEngine();
  }

  function createSpeechSynthesisEngine() {
    let speaking = false;
    let current = null;
    const synth = window.speechSynthesis;
    function selectedVoice() {
      const voices = synth.getVoices() || [];
      let name = '';
      try { name = localStorage.getItem('vc.voice') || ''; } catch {}
      if (name) {
        const v = voices.find((x) => x.name === name);
        if (v) return v;
      }
      const want = window.LANG === 'en' ? 'en' : 'ru';
      return voices.find((v) => (v.lang || '').toLowerCase().startsWith(want)) || null;
    }
    function speak(text) {
      return new Promise((resolve) => {
        if (!text || !text.trim()) { resolve(); return; }
        const u = new SpeechSynthesisUtterance(text);
        const voice = selectedVoice();
        if (voice) u.voice = voice;
        let rate = 1;
        try {
          const saved = localStorage.getItem('vc.speed');
          if (saved) rate = Math.min(1.5, Math.max(0.8, Number(saved)));
        } catch {}
        u.rate = rate;
        u.onend = () => { speaking = false; resolve(); };
        u.onerror = () => { speaking = false; resolve(); };
        speaking = true;
        current = u;
        try { synth.speak(u); } catch { speaking = false; resolve(); }
      });
    }
    function cancel() {
      try { synth.cancel(); } catch {}
      speaking = false;
      current = null;
    }
    return { speak, cancel, get speaking() { return speaking; } };
  }

  // --- Библиотека ---------------------------------------------------------
  function lib() { return window.DeloistUI?.voiceChat || {}; }
  function SentenceSplitterCtor() { return lib().SentenceSplitter; }
  function splitSentences(text) { return (lib().splitSentences || (() => ({ sentences: [], remainder: text })))(text); }
  function stripForSpeech(text) { return (lib().stripForSpeech || ((t) => t))(text); }
  function initialVoiceState(botId) { return (lib().createVoiceLoopState || ((b) => ({ phase: 'listening', botId: b })))(botId); }

  const V = window.DeloistVoice || {};

  // --- Панель -------------------------------------------------------------
  function openPanel(initialBotId) {
    if (!isVoiceSupported()) return null;
    const currentBotId = initialBotId || window.DeloistApp?.currentBot?.();
    if (!currentBotId) return null;

    const host = document.createElement('div');
    host.id = 'voice-chat-host';
    document.body.appendChild(host);

    let view = null;
    try {
      view = window.DeloistUI?.mountVoiceChat ? window.DeloistUI.mountVoiceChat(host, {
        state: 'listening',
        level: 0,
        lastPhrase: '',
        lastSentence: '',
        voices: [],
        voiceName: (function(){ try { return localStorage.getItem('vc.voice') || ''; } catch { return ''; } })(),
        speed: (function(){ try { const s = localStorage.getItem('vc.speed'); return s ? Number(s) : 1; } catch { return 1; } })(),
        onpause: () => togglePause(),
        onfinish: () => close(),
        onvoicechange: (v) => { try { localStorage.setItem('vc.voice', v || ''); } catch {} },
        onspeedchange: (v) => { try { localStorage.setItem('vc.speed', String(v)); } catch {} },
      }) : null;
    } catch (e) { console.error(e); }

    const engine = pickEngine();
    const Splitter = SentenceSplitterCtor();
    let splitter = Splitter ? new Splitter() : { buffer: '', feed(t){ this.buffer += t; const r = splitSentences(this.buffer); this.buffer = r.remainder; return r.sentences; }, flush(){ const s=this.buffer.trim(); this.buffer=''; return s?[s]:[]; } };

    let state = initialVoiceState(currentBotId);
    let phase = 'listening';
    let botId = currentBotId;
    let paused = false;
    let approvalActive = false;
    let approvalSpoken = false;
    let ignoreSpeech = false;
    let turnEnded = false;
    let pendingSpeech = 0;
    let speechChain = Promise.resolve();

    let recorder = null;
    let monitorRecorder = null;
    let aboveStart = 0;
    let destroyed = false;

    function setView(patch) { try { view?.update?.(patch); } catch {} }

    function refreshVoices() {
      const voices = (window.speechSynthesis.getVoices() || []).filter((v) => /^(ru|en)/i.test(v.lang || ''));
      setView({ voices });
    }
    refreshVoices();
    const voicesChanged = () => refreshVoices();
    try { window.speechSynthesis.addEventListener('voiceschanged', voicesChanged); } catch {}

    function close() {
      if (destroyed) return;
      destroyed = true;
      try { window.speechSynthesis.removeEventListener('voiceschanged', voicesChanged); } catch {}
      document.removeEventListener('keydown', onKey);
      window.removeEventListener('deloist:event', onEvent);
      if (recorder) { try { recorder.cancel(); } catch {} recorder = null; }
      if (monitorRecorder) { try { monitorRecorder.cancel(); } catch {} monitorRecorder = null; }
      try { engine.cancel(); } catch {}
      try { view?.destroy?.(); } catch {}
      try { host.remove(); } catch {}
    }

    function onKey(e) {
      if (e.key === 'Escape') { e.preventDefault(); close(); }
    }
    document.addEventListener('keydown', onKey);

    async function startListening() {
      if (destroyed || paused || approvalActive) return;
      if (monitorRecorder) { try { monitorRecorder.cancel(); } catch {} monitorRecorder = null; }
      const rec = new V.PcmRecorder({
        onlevel: (level) => {
          const shown = Math.min(1, (level || 0) * 14);
          setView({ level: shown, state: 'listening' });
        },
        onsilence: () => finishUtterance(rec),
        onlimit: () => finishUtterance(rec),
      });
      recorder = rec;
      setView({ state: 'listening', level: 0 });
      try { await rec.start(); } catch (err) {
        if (recorder === rec) recorder = null;
        setView({ state: 'listening' });
      }
    }

    async function finishUtterance(rec) {
      if (recorder !== rec) return;
      recorder = null;
      setView({ state: 'recognizing', level: 0 });
      try {
        const { pcm, seconds } = await rec.stop();
        if (!rec.heard || seconds < 0.3) { startListening(); return; }
        const text = await V.transcribe(pcm, rec.rate);
        if (!text) { setView({ lastPhrase: '' }); startListening(); return; }
        setView({ lastPhrase: text, state: 'sending' });
        // Фраза идёт мимо поля ввода: черновик и вложения человека остаются как были.
        try {
          await window.DeloistApp.sendMessage({ text, voice: true });
        } catch (err) {
          console.error('voice-chat send', err);
          setView({ state: 'listening', lastSentence: tr('Не получилось отправить фразу боту') });
          startListening();
          return;
        }
        setView({ state: 'thinking' });
      } catch (err) {
        startListening();
      }
    }

    function enqueueSpeech(text) {
      if (!text || ignoreSpeech) return;
      pendingSpeech++;
      const needsMonitor = pendingSpeech === 1;
      if (needsMonitor) startMonitor();
      speechChain = speechChain.then(async () => {
        if (ignoreSpeech) return;
        await engine.speak(text);
      }).finally(() => {
        pendingSpeech = Math.max(0, pendingSpeech - 1);
        if (pendingSpeech <= 0 && turnEnded && !ignoreSpeech && !paused && !approvalActive && !destroyed) {
          turnEnded = false;
          startListening();
        }
      });
    }

    function startMonitor() {
      if (monitorRecorder || destroyed || paused || approvalActive) return;
      const rec = new V.PcmRecorder({
        onlevel: (level) => {
          const thr = 0.012 * 1.5;
          if ((level || 0) >= thr) {
            if (!aboveStart) aboveStart = performance.now();
            if (performance.now() - aboveStart >= 250) {
              aboveStart = 0;
              bargeIn();
            }
          } else {
            aboveStart = 0;
          }
        },
        onsilence: () => {
          if (monitorRecorder === rec) monitorRecorder = null;
          try { rec.cancel(); } catch {}
          if (!destroyed && !paused && !approvalActive && pendingSpeech > 0) startMonitor();
        },
        onlimit: () => {
          if (monitorRecorder === rec) monitorRecorder = null;
          try { rec.cancel(); } catch {}
          if (!destroyed && !paused && !approvalActive && pendingSpeech > 0) startMonitor();
        },
      });
      monitorRecorder = rec;
      rec.start().catch(() => { if (monitorRecorder === rec) monitorRecorder = null; });
    }

    function bargeIn() {
      if (ignoreSpeech || destroyed) return;
      ignoreSpeech = true;
      turnEnded = false;
      pendingSpeech = 0;
      try { engine.cancel(); } catch {}
      splitter = Splitter ? new Splitter() : splitter;
      if (monitorRecorder) { try { monitorRecorder.cancel(); } catch {} monitorRecorder = null; }
      setView({ state: 'listening', level: 0 });
      startListening();
    }

    function approvalPause() {
      if (approvalActive || destroyed) return;
      approvalActive = true;
      paused = false;
      ignoreSpeech = true;
      turnEnded = false;
      pendingSpeech = 0;
      splitter = Splitter ? new Splitter() : splitter;
      if (recorder) { try { recorder.cancel(); } catch {} recorder = null; }
      if (monitorRecorder) { try { monitorRecorder.cancel(); } catch {} monitorRecorder = null; }
      try { engine.cancel(); } catch {}
      setView({ state: 'approval', level: 0, lastSentence: tr('Нужно твоё решение в чате') });
      if (!approvalSpoken) {
        approvalSpoken = true;
        try { engine.speak(tr('Нужно твоё решение в чате')); } catch {}
      }
    }

    function approvalResume() {
      if (!approvalActive || destroyed) return;
      approvalActive = false;
      ignoreSpeech = false;
      approvalSpoken = false;
      setView({ state: 'listening', level: 0 });
      startListening();
    }

    function togglePause() {
      if (destroyed) return;
      if (paused) {
        paused = false;
        ignoreSpeech = false;
        setView({ state: 'listening', level: 0 });
        startListening();
      } else {
        paused = true;
        if (recorder) { try { recorder.cancel(); } catch {} recorder = null; }
        if (monitorRecorder) { try { monitorRecorder.cancel(); } catch {} monitorRecorder = null; }
        try { engine.cancel(); } catch {}
        splitter = Splitter ? new Splitter() : splitter;
        pendingSpeech = 0;
        turnEnded = false;
        ignoreSpeech = false;
        setView({ state: 'paused', level: 0 });
      }
    }

    function onEvent(ev) {
      if (destroyed) return;
      const e = ev?.detail || ev;
      if (!e) return;
      // switching bot ends panel
      if (e.botId && e.botId !== botId && (e.type === 'turn_start' || e.type === 'message' || e.type === 'text_delta')) {
        close();
        return;
      }
      if (e.type === 'turn_start' && e.botId === botId) {
        turnEnded = false;
        ignoreSpeech = false;
      }
      if (e.type === 'text_delta' && e.botId === botId) {
        const delta = e.text ?? e.delta ?? e.content ?? '';
        if (!delta || ignoreSpeech) return;
        const sentences = splitter.feed(delta);
        for (const s of sentences) {
          const clean = stripForSpeech(s);
          if (clean) {
            setView({ lastSentence: clean, state: 'speaking' });
            enqueueSpeech(clean);
          }
        }
        if (!sentences.length) setView({ state: 'speaking' });
      }
      if (e.type === 'turn_end' && e.botId === botId) {
        const rest = splitter.flush();
        for (const s of rest) {
          const clean = stripForSpeech(s);
          if (clean) {
            setView({ lastSentence: clean });
            enqueueSpeech(clean);
          }
        }
        turnEnded = true;
        if (pendingSpeech <= 0 && !paused && !approvalActive) {
          turnEnded = false;
          setView({ state: 'listening', level: 0 });
          startListening();
        }
      }
      if (e.type === 'approval' && e.botId === botId) approvalPause();
      if (e.type === 'approval_closed') approvalResume();
    }

    window.addEventListener('deloist:event', onEvent);

    // start
    setView({ state: 'listening', level: 0 });
    startListening();

    return { close, view };
  }

  window.DeloistVoiceChat = { isVoiceSupported, pickEngine, openPanel, splitSentences, stripForSpeech };
})();
