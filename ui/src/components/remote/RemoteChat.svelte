<script>
  // Чат с ботом другого ПК: переписка, отправка, остановка, карточки разрешений на месте и живые обновления.
  // Правила для чужих данных: все id в адресах — через enc(); текст сообщений — только через markdown() в режиме
  // «чужой текст» (экранирует всё, ссылки на файлы выключены, у ссылок виден настоящий адрес и пометка источника);
  // ошибки и подписи карточек — только текстом; ссылки сами не открываются.
  import { onMount, tick, untrack } from 'svelte';
import { Button, Icon } from '../kit';
  import ApprovalCard from '../chat/ApprovalCard.svelte';
  import RemoteOrb from './RemoteOrb.svelte';
  import RemoteExtras from './RemoteExtras.svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { enc, errorText } from '../../lib/remote.js';
  import { markdown } from '../../lib/markdown.js';
  import { whenText } from '../../lib/when.js';

  // pc: { id, name, online }, bot: элемент state.
  let { api, pc, bot, onmenu } = $props();

  let messages = $state([]);
  let cards = $state([]);
  let info = $state(untrack(() => ({ busy: Boolean(bot?.busy), queued: Number(bot?.queued) || 0 })));
  let working = $state('');
  let text = $state('');
  let sending = $state(false);
  let error = $state('');
  let extras = $state(false);
  let list = $state();
  let loaded = $state(false);

  const base = $derived(`/api/remote/${enc(pc.id)}`);
  const botPath = $derived(`${base}/bots/${enc(bot.id)}`);
  const offline = $derived(!pc.online);

  const mark = () => tr('с другого ПК');
  const html = (value) => markdown(value, { remote: { mark: mark() } });

  async function loadThread() {
    try {
      const data = await api(`${botPath}/thread?limit=100`);
      messages = Array.isArray(data?.messages) ? data.messages : [];
      error = '';
      loaded = true;
      await tick();
      if (list) list.scrollTop = list.scrollHeight;
    } catch (err) {
      error = trServer(errorText(err));
      loaded = true;
    }
  }

  async function loadInfo() {
    try {
      const data = await api(`${base}/state`);
      const mine = (Array.isArray(data?.bots) ? data.bots : []).find((b) => b.id === bot.id);
      if (mine) {
        info = { busy: Boolean(mine.busy), queued: Number(mine.queued) || 0 };
        if (!mine.busy) working = '';
      }
    } catch {
      /* прежнее состояние */
    }
  }

  async function loadNeeds() {
    try {
      const data = await api(`${base}/needs`);
      cards = (Array.isArray(data?.items) ? data.items : []).filter((c) => c.botId === bot.id).map((c) => ({ ...c, note: '', busy: false, sent: false }));
    } catch {
      /* карточек нет */
    }
  }

  const timers = new Map();
  function later(key, fn, delay = 500) {
    clearTimeout(timers.get(key));
    timers.set(key, setTimeout(fn, delay));
  }

  function onEvent(event) {
    const ev = event.detail;
    if (ev?.type !== 'remote_event' || ev.pc !== pc.id) return;
    const kind = ev.event?.type;
    const data = ev.event?.data ?? {};
    if (kind === 'state_changed') {
      later('info', loadInfo);
      later('thread', loadThread);
    } else if (kind === 'journal' && (!data.botId || data.botId === bot.id)) {
      later('thread', loadThread);
      later('info', loadInfo);
    } else if (kind === 'tool_start' && data.botId === bot.id) {
      working = String(data.name ?? '') || tr('работает');
      info = { ...info, busy: true };
    } else if ((kind === 'approval' || kind === 'question') && data.botId === bot.id) {
      if (data.id && !cards.some((c) => c.id === data.id)) cards = [...cards, { ...data, kind: kind === 'question' ? 'question' : data.kind, note: '', busy: false, sent: false }];
    } else if (kind === 'approval_closed' || kind === 'question_closed') {
      cards = cards.filter((c) => c.id !== data.id);
    }
  }

  onMount(() => {
    loadThread();
    loadInfo();
    loadNeeds();
    window.addEventListener('deloist:event', onEvent);
    return () => {
      window.removeEventListener('deloist:event', onEvent);
      for (const id of timers.values()) clearTimeout(id);
    };
  });

  async function send() {
    const body = text.trim();
    if (!body || sending || offline) return;
    sending = true;
    error = '';
    try {
      await api(`${botPath}/messages`, { method: 'POST', body: { text: body } });
      messages = [...messages, { id: `local-${Date.now()}`, role: 'user', text: body, at: Date.now() }];
      text = '';
      info = { ...info, busy: true };
      await tick();
      if (list) list.scrollTop = list.scrollHeight;
      later('thread', loadThread, 800);
    } catch (err) {
      error = trServer(errorText(err));
    } finally {
      sending = false;
    }
  }

  async function stop() {
    try {
      await api(`${botPath}/stop`, { method: 'POST' });
      later('info', loadInfo, 300);
    } catch (err) {
      error = trServer(errorText(err));
    }
  }

  const patchCard = (id, next) => (cards = cards.map((c) => (c.id === id ? { ...c, ...next } : c)));

  async function answerCard(card, answer) {
    patchCard(card.id, { busy: true, note: '' });
    try {
      await api(`${base}/approvals/${enc(card.id)}`, { method: 'POST', body: { answer } });
      patchCard(card.id, { busy: false, sent: true });
      setTimeout(() => (cards = cards.filter((c) => c.id !== card.id)), 1200);
    } catch (err) {
      if (err?.status === 404) cards = cards.filter((c) => c.id !== card.id);
      // ПК не открыл эту карточку устройствам: кнопок разрешения нет, остаётся строка и «Запретить».
      else if ((err?.code || err?.data?.code) === 'on_pc') patchCard(card.id, { busy: false, remote: false });
      else patchCard(card.id, { busy: false, note: trServer(errorText(err)) });
    }
  }

  const questionDraft = $state({});
  async function answerQuestion(card, answer) {
    if (!String(answer).trim()) return;
    patchCard(card.id, { busy: true, note: '' });
    try {
      await api(`${base}/questions/${enc(card.id)}`, { method: 'POST', body: { answer: String(answer) } });
      cards = cards.filter((c) => c.id !== card.id);
    } catch (err) {
      if (err?.status === 404) cards = cards.filter((c) => c.id !== card.id);
      else patchCard(card.id, { busy: false, note: trServer(errorText(err)) });
    }
  }

  const riskLabels = $derived({ low: tr('Риск: низкий'), medium: tr('Риск: средний'), high: tr('Риск: высокий') });
  // Разрешить можно только то, что ПК открыл для устройств; «Запретить» работает всегда.
  function actionsOf(card) {
    const answers = Array.isArray(card.answers) ? card.answers : [];
    const out = [];
    if (card.remote && answers.includes('once')) out.push({ answer: 'once', label: tr('Разрешить') });
    out.push({ answer: 'deny', label: tr('Запретить') });
    return out;
  }
  const isQuestion = (card) => card.kind === 'question' || typeof card.question === 'string';
  const humanOf = (card) => (window.LANG === 'en' && card.human?.textEn ? card.human.textEn : card.human?.text);
  const onKey = (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  };
</script>

<section class="rc" data-remote-chat>
  <header class="rc-head">
    {#if onmenu}
      <button type="button" class="rc-menu" aria-label={tr('Открыть боковую панель')} onclick={onmenu}>
        <Icon name="menu" size={20} />
      </button>
    {/if}
    <RemoteOrb {bot} busy={info.busy} size={40} />
    <div class="rc-title">
      <div class="rc-name">{bot.name || tr('Бот')}</div>
      <div class="rc-sub">
        <span class="rc-dot" class:is-on={pc.online} aria-hidden="true"></span>
        {tr('ПК «{name}»', { name: pc.name })} · {pc.online ? (info.busy ? working ? tr('работает: {tool}', { tool: working }) : tr('работает') : tr('в сети')) : tr('не в сети')}
      </div>
    </div>
    <div class="rc-actions">
      {#if info.busy}<Button size="sm" variant="ghost" onclick={stop}>{tr('Остановить')}</Button>{/if}
      <Button size="sm" variant="ghost" aria-expanded={extras} onclick={() => (extras = !extras)}>{tr('Рутины и лимиты')}</Button>
    </div>
  </header>

  {#if extras}<RemoteExtras {api} {pc} />{/if}

  <div class="rc-list" bind:this={list} role="log" aria-live="polite" aria-label={tr('Переписка с ботом другого ПК')}>
    {#if loaded && !messages.length && !error}<p class="rc-empty">{tr('Сообщений пока нет')}</p>{/if}
    {#each messages as m (m.id)}
      <div class="rc-msg" class:is-user={m.role === 'user'} class:is-error={m.error} data-role={m.role}>
        {#if m.role === 'user'}
          <p class="rc-plain">{m.text}</p>
        {:else}
          <!-- markdown() экранирует весь текст сам; режим remote выключает ссылки на файлы и показывает адреса ссылок -->
          <div class="rc-md">{@html html(m.text)}</div>
        {/if}
        {#if m.at}<time class="rc-time">{whenText(m.at)}</time>{/if}
      </div>
    {/each}
    {#each cards as card (card.id)}
      {#if isQuestion(card)}
        <section class="rc-card rc-question" data-question={card.id}>
          <p class="rc-plain">{card.question}</p>
          {#if card.form?.length}
            <p class="rc-note">{tr('Этот вопрос с формой: ответьте на том ПК')}</p>
          {:else}
            {#each card.choices ?? [] as choice}
              <Button size="sm" disabled={card.busy} onclick={() => answerQuestion(card, choice)}>{choice}</Button>
            {/each}
            <form class="rc-row" onsubmit={(e) => { e.preventDefault(); answerQuestion(card, questionDraft[card.id] ?? ''); }}>
              <input class="k-input" placeholder={tr('Ответ…')} bind:value={questionDraft[card.id]} />
              <Button size="sm" variant="primary" type="submit" disabled={card.busy}>{tr('Ответить')}</Button>
            </form>
          {/if}
          {#if card.note}<p class="rc-note" role="alert">{card.note}</p>{/if}
        </section>
      {:else}
        <div class="rc-card" data-card={card.id}>
          <ApprovalCard
            title={humanOf(card) || card.title || card.tool || tr('Нужно разрешение')}
            what={humanOf(card) ? '' : card.detail || ''}
            details={humanOf(card) ? card.detail || '' : ''}
            detailsLabel={tr('Показать технические детали')}
            risk={card.human?.risk || ''}
            riskLabel={riskLabels[card.human?.risk] || ''}
            purpose={card.human?.purpose || ''}
            purposeLabel={tr('Бот объясняет:')}
            scope={card.scope?.label || ''}
            danger={Boolean(card.danger)}
            busy={card.busy}
            actions={actionsOf(card)}
            resolved={card.sent ? tr('Ответ отправлен') : ''}
            onanswer={(answer) => answerCard(card, answer)}
          />
          {#if !card.remote && !card.sent}<p class="rc-note">{tr('Одобрите на том ПК')}</p>{/if}
          {#if card.note}<p class="rc-note" role="alert">{card.note}</p>{/if}
        </div>
      {/if}
    {/each}
  </div>

  {#if error}<p class="rc-error" role="alert">{error}</p>{/if}
  <form class="rc-composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <textarea
      class="k-input rc-input"
      rows="2"
      maxlength="8000"
      placeholder={offline ? tr('ПК не в сети') : tr('Написать боту на другом ПК…')}
      aria-label={tr('Сообщение боту')}
      disabled={offline}
      bind:value={text}
      onkeydown={onKey}
    ></textarea>
    <Button variant="primary" type="submit" loading={sending} disabled={offline || !text.trim()}>{tr('Отправить')}</Button>
  </form>
</section>

<style>
  .rc { display: flex; flex-direction: column; flex: 1; min-height: 0; background: var(--k-canvas, transparent); }
  .rc-head { display: flex; align-items: center; gap: var(--k-space-3, 12px); padding: var(--k-space-3, 12px) var(--k-space-4, 16px); border-bottom: 1px solid var(--k-line); }
  .rc-menu { display: none; border: 0; background: transparent; color: inherit; cursor: pointer; }
  @media (max-width: 820px) { .rc-menu { display: inline-grid; } }
  .rc-title { flex: 1; min-width: 0; }
  .rc-name { font: 600 var(--k-text-md, 15px) / 1.3 var(--k-font-body, inherit); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rc-sub { display: flex; align-items: center; gap: 6px; font: 400 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, inherit); color: var(--k-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rc-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--k-text-3); flex: none; }
  .rc-dot.is-on { background: var(--k-brand); }
  .rc-actions { display: flex; gap: var(--k-space-2, 8px); }
  .rc-list { flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: var(--k-space-3, 12px); padding: var(--k-space-4, 16px); }
  .rc-empty { margin: auto; color: var(--k-text-3); font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, inherit); }
  .rc-msg { max-width: min(680px, 92%); padding: var(--k-space-3); border-radius: var(--k-radius-lg, 16px); background: var(--k-surface-2); overflow-wrap: anywhere; animation: rc-in 0.3s cubic-bezier(0.34, 1.36, 0.64, 1); }
  .rc-msg.is-user { align-self: flex-end; background: var(--k-brand-tint); }
  .rc-msg.is-error { box-shadow: inset 0 0 0 1px var(--k-attention); }
  .rc-plain { margin: 0; white-space: pre-wrap; font: 400 var(--k-text-sm, 14px) / 1.5 var(--k-font-body, inherit); }
  .rc-md :global(p) { margin: 0 0 6px; }
  .rc-md :global(.md-remote-url) { font-size: var(--k-text-xs, 12px); color: var(--k-text-3); word-break: break-all; }
  .rc-time { display: block; margin-top: 4px; font: 400 var(--k-text-micro, 11px) / 1 var(--k-font-body, inherit); color: var(--k-text-3); }
  .rc-card { display: flex; flex-direction: column; gap: 6px; max-width: min(680px, 100%); }
  .rc-question { padding: var(--k-space-3, 12px); border-radius: var(--k-radius-lg, 16px); background: var(--k-surface-2); }
  .rc-row { display: flex; gap: var(--k-space-2, 8px); }
  .rc-row .k-input { flex: 1; }
  .rc-note, .rc-error { margin: 0; font: 600 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, inherit); color: var(--k-attention-text); padding: 0 var(--k-space-4, 16px); }
  .rc-card .rc-note, .rc-question .rc-note { padding: 0; }
  .rc-composer { display: flex; gap: var(--k-space-2, 8px); align-items: flex-end; padding: var(--k-space-3, 12px) var(--k-space-4, 16px); border-top: 1px solid var(--k-line); }
  .rc-input { flex: 1; resize: none; }
  @keyframes rc-in { from { opacity: 0; transform: translateY(6px) scale(0.98); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { .rc-msg { animation: none; } }
</style>
