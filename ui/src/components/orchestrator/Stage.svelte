<script>
  // Сцена оркестратора: большой шар по центру и разговор в пузырях, как в мессенджере.
  // Это другой вид того же разговора с ботом-оркестратором: события и карточки (ask_user, propose_team)
  // приходят из web/app.js, ответы уходят туда же. Здесь только вид и ввод.
  import { untrack, onMount, tick } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { markdown } from '../../lib/markdown.js';
  import Icon from '../kit/Icon.svelte';
  import { moodForPhase, phaseOf, bubbleOf } from './model.js';

  let {
    bot = {},
    stage = 'interview',
    history = [],
    mountOrb = null,
    voice = null,
    onsend,
    onleave,
    onready,
    onerror,
  } = $props();

  let seq = 0;
  const nextId = () => `o${++seq}`;

  let items = $state(
    untrack(() => history)
      .map(bubbleOf)
      .filter(Boolean)
      .map((b) => ({ id: nextId(), ...b })),
  );
  let busy = $state(false);
  let streaming = $state(false);
  let micState = $state('idle');
  let draft = $state('');
  let leaving = $state(false);
  let leaveTransform = $state('');
  let orb = $state.raw(null);
  let micAvailable = $state(true);

  let orbHost;
  let orbWrap;
  let root;
  let streamEl;
  let micBtn;
  let liveKey = '';
  let liveId = '';
  let mic = null;
  // Тексты, которые человек уже видит в своих пузырях: эхо того же сообщения с сервера не рисуем второй раз.
  const echoes = [];

  const openCards = $derived(items.filter((i) => i.kind === 'card').length);
  const questionOpen = $derived(items.some((i) => i.kind === 'card' && i.sub === 'question'));
  const proposalOpen = $derived(items.some((i) => i.kind === 'card' && i.sub === 'proposal'));
  const phase = $derived(phaseOf({ stage, hearing: micState === 'listening', openCards, streaming, busy }));
  const waiting = $derived(stage === 'interview' && busy && !questionOpen && !proposalOpen);
  const started = $derived(items.some((i) => i.kind === 'user'));

  // ---- события разговора ----

  function pushBubble(kind, text, extra = {}) {
    items.push({ id: nextId(), kind, text, ...extra });
  }

  function closeLive() {
    if (liveId) {
      const it = items.find((i) => i.id === liveId);
      if (it) {
        it.live = false;
        if (!it.text.trim()) items.splice(items.indexOf(it), 1);
      }
    }
    liveId = '';
    liveKey = '';
    streaming = false;
  }

  function onMessage(message) {
    if (!message) return;
    if (message.role === 'user') {
      const text = String(message.text ?? '').trim();
      const at = echoes.indexOf(text);
      if (at >= 0) {
        echoes.splice(at, 1);
        return;
      }
      if (text) pushBubble('user', text);
      return;
    }
    if (message.role !== 'assistant') return;
    const text = String(message.text ?? '').trim();
    const it = liveId ? items.find((i) => i.id === liveId) : null;
    if (it) {
      if (text) it.text = text;
      it.error = Boolean(message.error);
      closeLive();
    } else if (text) {
      pushBubble('bot', text, { error: Boolean(message.error) });
    }
  }

  function event(ev) {
    if (!ev || typeof ev !== 'object') return;
    switch (ev.type) {
      case 'turn_start':
        busy = true;
        break;
      case 'turn_end':
        busy = false;
        closeLive();
        break;
      case 'text_delta': {
        busy = true;
        const key = `${ev.turnId ?? ''}:${ev.step ?? ''}`;
        if (liveId && key !== liveKey) closeLive();
        if (!liveId) {
          liveId = nextId();
          liveKey = key;
          items.push({ id: liveId, kind: 'bot', text: '', live: true });
        }
        const it = items.find((i) => i.id === liveId);
        if (it) it.text += String(ev.text ?? '');
        streaming = true;
        break;
      }
      case 'message':
        onMessage(ev.message);
        break;
      default:
        break;
    }
  }

  // Карточку (вопрос, предложение команды) рисует web/app.js; сцена даёт ей место в ленте.
  function addCard(node) {
    if (!(node instanceof Element)) return;
    const sub = node.classList.contains('team-proposal') ? 'proposal' : node.classList.contains('question') ? 'question' : 'other';
    const id = nextId();
    // Прицепленный к элементу код стоит один на карточку: новая функция на каждую отрисовку пересоздавала бы узел.
    const attach = (el) => {
      el.append(node);
      const watch = new MutationObserver(() => {
        if (node.parentNode === el) return;
        const at = items.findIndex((i) => i.id === id);
        if (at >= 0) items.splice(at, 1);
      });
      watch.observe(el, { childList: true });
      return () => watch.disconnect();
    };
    items.push({ id, kind: 'card', sub, node, attach });
  }

  // ---- отправка ----

  async function send(raw, opts = {}) {
    const text = String(raw ?? '').trim();
    if (!text) return;
    pushBubble('user', text);
    echoes.push(text);
    busy = true;
    try {
      await onsend?.(text, opts);
    } catch (err) {
      busy = false;
      onerror?.(err?.message || String(err));
    }
  }

  function submit() {
    const text = draft;
    draft = '';
    send(text);
  }

  function keydown(e) {
    if (e.isComposing) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  // «Хватит, предлагай»: открытый вопрос закрываем («Пропустить»), иначе ход ждал бы ответа, а сообщение стояло в очереди.
  function enough() {
    for (const it of items) {
      if (it.kind === 'card' && it.sub === 'question') it.node.querySelector('[data-dismiss]')?.click();
    }
    send(tr('Хватит, предлагай'));
  }

  // Ответ в карточке вопроса виден в ленте справа, как реплика человека.
  function echoAnswer(e) {
    const choice = e.target?.closest?.('[data-choice]');
    if (choice) {
      const text = choice.dataset.choice || choice.textContent.trim();
      pushBubble('user', text);
      echoes.push(text);
      return;
    }
    const sendBtn = e.target?.closest?.('[data-send]');
    if (sendBtn) {
      const val = sendBtn.closest('.question')?.querySelector('.q-input')?.value?.trim();
      if (val) {
        pushBubble('user', val);
        echoes.push(val);
      }
    }
  }

  // ---- шар ----

  const defaultOrb = (host, { size }) =>
    typeof window !== 'undefined' && window.CrewAvatar
      ? window.CrewAvatar.mount(host, { size, skin: 13, color: bot.color, name: bot.name || '' })
      : null;

  let lastPhase = '';
  let lastOrb = null;
  $effect(() => {
    const p = phase;
    const current = orb;
    if (p === lastPhase && current === lastOrb) return;
    lastPhase = p;
    lastOrb = current;
    const mood = moodForPhase(p);
    if (mood) current?.feel?.(mood, 3600);
  });

  const reduced = () => typeof window !== 'undefined' && Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches);

  function startLeave() {
    if (leaving) return;
    leaving = true;
    if (reduced()) {
      queueMicrotask(() => onleave?.());
      return;
    }
    const r = orbWrap?.getBoundingClientRect();
    const s = root?.getBoundingClientRect();
    if (r && s && r.width) {
      const dx = s.left + 44 - (r.left + r.width / 2);
      const dy = s.top + 44 - (r.top + r.height / 2);
      leaveTransform = `translate(${Math.round(dx)}px, ${Math.round(dy)}px) scale(0.18)`;
    }
    setTimeout(() => onleave?.(), 520);
  }

  $effect(() => {
    if (stage === 'ready') untrack(startLeave);
  });

  // Лента едет вниз за новым текстом.
  $effect(() => {
    items.length;
    items.at(-1)?.text;
    waiting;
    tick().then(() => {
      if (streamEl) streamEl.scrollTop = streamEl.scrollHeight;
    });
  });

  onMount(() => {
    if (orbHost) orb = (mountOrb ?? defaultOrb)(orbHost, { size: 240 }) || null;
    const lib = voice ?? (typeof window !== 'undefined' ? window.DeloistVoice : null);
    if (lib?.voiceButton && micBtn && (!lib.isVoiceSupported || lib.isVoiceSupported())) {
      mic = lib.voiceButton({
        button: micBtn,
        onText: (text) => send(text, { voice: true }),
        onError: (message) => onerror?.(message),
        onState: (s) => {
          micState = s;
        },
      });
    } else {
      micAvailable = false;
    }
    onready?.({ event, addCard, send });
    return () => {
      mic?.cancel?.();
      orb?.destroy?.();
    };
  });
</script>

<section class="orch" class:is-leaving={leaving} aria-label={tr('Оркестратор')} bind:this={root}>
  <div class="orch-orb" bind:this={orbWrap} style:transform={leaveTransform}>
    <div class="orch-orb-host" bind:this={orbHost} data-phase={phase}></div>
  </div>

  <div class="orch-stream" bind:this={streamEl} aria-live="polite">
    {#each items as it (it.id)}
      {#if it.kind === 'card'}
        <div class="orch-row">
          <div class="orch-cardbox" class:is-proposal={it.sub === 'proposal'}>
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div class="orch-card" {@attach it.attach} onclick={echoAnswer}></div>
            {#if it.sub === 'question'}
              <button type="button" class="orch-enough" onclick={enough}>{tr('Хватит, предлагай')}</button>
            {/if}
          </div>
        </div>
      {:else}
        <div class="orch-row" class:is-user={it.kind === 'user'}>
          <div class="orch-bub" class:is-live={it.live} class:is-error={it.error}>
            {#if it.kind === 'user'}{it.text}{:else}{@html markdown(it.text)}{/if}
          </div>
        </div>
      {/if}
    {/each}
    {#if waiting}
      <div class="orch-row">
        <div class="orch-bub orch-wait" role="status">
          <span class="orch-dots" aria-hidden="true"><i></i><i></i><i></i></span>
          <span class="orch-wait-label">{streaming ? tr('Печатает…') : tr('Думает…')}</span>
        </div>
      </div>
    {/if}
  </div>

  <div class="orch-foot">
    {#if started && !questionOpen && !proposalOpen && stage === 'interview'}
      <button type="button" class="orch-enough" onclick={enough}>{tr('Хватит, предлагай')}</button>
    {/if}
    <form class="orch-composer" onsubmit={(e) => { e.preventDefault(); submit(); }}>
      <textarea
        class="orch-input"
        rows="1"
        bind:value={draft}
        onkeydown={keydown}
        placeholder={tr('Напиши ответ или скажи голосом')}
        aria-label={tr('Ответ оркестратору')}
      ></textarea>
      <button type="button" class="orch-mic" bind:this={micBtn} hidden={!micAvailable} data-state={micState} aria-label={tr('Сказать голосом')} title={tr('Сказать голосом')}>
        <Icon name="mic" size={20} />
      </button>
      <button type="submit" class="orch-send" disabled={!draft.trim()} aria-label={tr('Отправить')} title={tr('Отправить')}>
        <Icon name="send" size={20} />
      </button>
    </form>
  </div>
</section>

<style>
  .orch {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    height: 100%;
    min-height: 0;
    padding: var(--k-space-4) var(--k-space-4) var(--k-space-4);
    gap: var(--k-space-3);
    background: var(--k-canvas);
    color: var(--k-text);
    font-family: var(--k-font-body);
    box-sizing: border-box;
  }
  .orch-orb {
    flex: none;
    display: flex;
    justify-content: center;
    transform-origin: center;
    transition: transform var(--k-dur-morph) var(--k-ease-morph);
    z-index: 2;
  }
  .orch-orb-host {
    position: relative;
    width: 240px;
    height: 240px;
    max-width: 100%;
  }
  .orch-stream {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    width: 100%;
    max-width: 720px;
    margin: 0 auto;
    padding: var(--k-space-1) var(--k-space-1);
    box-sizing: border-box;
    transition: opacity var(--k-dur-exit) var(--k-ease-in);
  }
  .orch-row {
    display: flex;
    justify-content: flex-start;
    animation: orch-in var(--k-dur-base) var(--k-ease-out);
  }
  .orch-row.is-user {
    justify-content: flex-end;
  }
  .orch-bub {
    max-width: 82%;
    padding: var(--k-space-2) var(--k-space-4);
    border-radius: var(--k-radius-md) var(--k-radius-md) var(--k-radius-md) var(--k-radius-xs);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-1);
    font-size: var(--k-text-md);
    line-height: var(--k-leading-body);
    overflow-wrap: anywhere;
  }
  .orch-bub :global(p) {
    margin: 0;
  }
  .orch-bub :global(p + p) {
    margin-top: var(--k-space-2);
  }
  .is-user .orch-bub {
    border-radius: var(--k-radius-md) var(--k-radius-md) var(--k-radius-xs) var(--k-radius-md);
    background: var(--k-brand);
    color: var(--k-on-brand);
    box-shadow: none;
    white-space: pre-wrap;
  }
  .orch-bub.is-error {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .orch-wait {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-2);
    color: var(--k-text-2);
    font-size: var(--k-text-sm);
  }
  .orch-dots {
    display: inline-flex;
    gap: var(--k-space-1);
  }
  .orch-dots i {
    width: 6px;
    height: 6px;
    border-radius: var(--k-radius-pill);
    background: var(--k-text-3);
    animation: orch-dot 1.1s var(--k-ease-inout) infinite;
  }
  .orch-dots i:nth-child(2) {
    animation-delay: 0.16s;
  }
  .orch-dots i:nth-child(3) {
    animation-delay: 0.32s;
  }
  .orch-cardbox {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    align-items: flex-start;
    max-width: 92%;
    min-width: min(420px, 100%);
  }
  .orch-cardbox.is-proposal {
    max-width: 100%;
    width: 100%;
    font-size: var(--k-text-md);
  }
  .orch-card {
    width: 100%;
  }
  .orch-card:empty {
    display: none;
  }
  .orch-card :global(.card) {
    margin: 0;
    max-width: none;
  }
  .is-proposal .orch-card :global(.card) {
    padding: var(--k-space-6);
    border-radius: var(--k-radius-lg);
  }
  .orch-foot {
    flex: none;
    width: 100%;
    max-width: 720px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    align-items: flex-start;
    transition: opacity var(--k-dur-exit) var(--k-ease-in);
  }
  .orch-enough {
    padding: var(--k-space-1) var(--k-space-3);
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-pill);
    background: var(--k-surface);
    color: var(--k-text);
    font: inherit;
    font-size: var(--k-text-sm);
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out);
  }
  .orch-enough:hover {
    background: var(--k-surface-2);
  }
  .orch-enough:focus-visible,
  .orch-mic:focus-visible,
  .orch-send:focus-visible,
  .orch-input:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  .orch-composer {
    display: flex;
    align-items: flex-end;
    gap: var(--k-space-2);
    width: 100%;
    padding: var(--k-space-2);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-1);
    box-sizing: border-box;
  }
  .orch-input {
    flex: 1;
    min-height: 40px;
    max-height: 160px;
    padding: var(--k-space-2) var(--k-space-3);
    border: 0;
    border-radius: var(--k-radius-md);
    background: transparent;
    color: var(--k-text);
    font: inherit;
    font-size: var(--k-text-md);
    resize: none;
    box-sizing: border-box;
  }
  .orch-mic,
  .orch-send {
    flex: none;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: var(--k-radius-pill);
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out), color var(--k-dur-fast) var(--k-ease-out);
  }
  .orch-mic {
    background: var(--k-surface-2);
    color: var(--k-text-2);
  }
  .orch-mic[data-state='listening'] {
    background: var(--k-attention);
    color: var(--k-on-brand);
  }
  .orch-mic[data-state='working'],
  .orch-mic[data-state='downloading'],
  .orch-mic[data-state='loading'] {
    background: var(--k-ai-tint);
    color: var(--k-ai);
  }
  .orch-send {
    background: var(--k-brand);
    color: var(--k-on-brand);
  }
  .orch-send:hover:not(:disabled) {
    background: var(--k-brand-hover);
  }
  .orch-send:disabled {
    background: var(--k-surface-3);
    color: var(--k-text-3);
    cursor: default;
  }
  .is-leaving .orch-stream,
  .is-leaving .orch-foot {
    opacity: 0;
  }
  @keyframes orch-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.98);
    }
  }
  @keyframes orch-dot {
    0%,
    80%,
    100% {
      opacity: 0.35;
    }
    40% {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .orch-row,
    .orch-dots i {
      animation: none;
    }
    .orch-orb,
    .orch-stream,
    .orch-foot {
      transition: none;
    }
  }
</style>
