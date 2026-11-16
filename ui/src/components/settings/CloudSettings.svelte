<script>
  // Экран «Облако» окна настроек: своя машина Instabox, на которой боты работают без компьютера.
  // Ответы /api/cloud/instabox/* не содержат секретов (ключи остаются на ПК), поэтому здесь их нет.
  // Адрес облака показываем текстом; открывать его можно только кнопкой «Открыть облако».
  import { untrack } from 'svelte';
  import Section from '../kit/Section.svelte';
  import Button from '../kit/Button.svelte';
  import Badge from '../kit/Badge.svelte';
  import { tr } from '../../lib/i18n.js';

  let { api } = $props();

  const BASE = '/api/cloud/instabox';
  const POLL_MS = 2000;
  const WAKE_MS = 4000;

  let status = $state(null);
  let loading = $state(true);
  let error = $state('');
  let notice = $state('');
  let starting = $state(false);
  let reopening = $state(false);
  let provisioning = $state(false);
  let opening = $state(false);
  let updating = $state(false);
  let deleting = $state(false);
  // «Отмена» входа: скрываем код у себя, пока человек не нажмёт «Подключить» снова.
  let dismissed = $state(false);

  let pollTimer = null;
  let wakeTimer = null;
  let alive = true;

  const signin = $derived(status?.signin || null);
  const pending = $derived(signin?.phase === 'pending' && !dismissed);
  const machine = $derived(status?.machine || null);
  const job = $derived(status?.job || null);
  const jobRunning = $derived(job?.state === 'running');
  const hasMachine = $derived(!!machine && machine.state !== 'missing');

  function fail(err) {
    error = err?.message || tr('Не удалось выполнить запрос');
  }

  function stopTimers() {
    if (pollTimer) clearInterval(pollTimer);
    if (wakeTimer) clearTimeout(wakeTimer);
    pollTimer = null;
    wakeTimer = null;
  }

  async function loadStatus() {
    try {
      const next = await api(`${BASE}/status`);
      if (!alive) return null;
      status = next;
      error = '';
      return next;
    } catch (err) {
      if (alive) fail(err);
      return null;
    } finally {
      loading = false;
    }
  }

  // Пока идёт вход или фоновая работа, спрашиваем ход раз в пару секунд.
  async function tick() {
    if (status?.signin?.phase === 'pending' && !dismissed) {
      try {
        const s = await api(`${BASE}/signin/status`);
        if (!alive) return;
        if (s?.phase !== 'pending') {
          await loadStatus();
          return;
        }
        status = { ...status, signin: s };
      } catch (err) {
        if (alive) fail(err);
      }
      return;
    }
    await loadStatus();
  }

  const needsPolling = $derived(pending || jobRunning);
  $effect(() => {
    const on = needsPolling;
    untrack(() => {
      if (on && !pollTimer) pollTimer = setInterval(tick, POLL_MS);
      if (!on && pollTimer) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    });
  });

  async function startSignin() {
    if (starting) return;
    starting = true;
    error = '';
    dismissed = false;
    try {
      const s = await api(`${BASE}/signin/start`, { method: 'POST', body: { open: true } });
      status = { ...(status || { signedIn: false, machine: null, job: null }), signin: s };
    } catch (err) {
      fail(err);
    } finally {
      starting = false;
    }
  }

  // Instabox теряет код после входа через Google: повторный start возвращает тот же код и снова открывает страницу.
  async function reopenSignin() {
    if (reopening) return;
    reopening = true;
    error = '';
    try {
      const s = await api(`${BASE}/signin/start`, { method: 'POST', body: { open: true } });
      status = { ...status, signin: s };
    } catch (err) {
      fail(err);
    } finally {
      reopening = false;
    }
  }

  function cancelSignin() {
    dismissed = true;
    error = '';
  }

  async function provision() {
    if (provisioning) return;
    provisioning = true;
    error = '';
    notice = '';
    try {
      await api(`${BASE}/provision`, { method: 'POST', body: {} });
      await loadStatus();
    } catch (err) {
      fail(err);
    } finally {
      provisioning = false;
    }
  }

  async function update() {
    if (updating) return;
    updating = true;
    error = '';
    notice = '';
    try {
      await api(`${BASE}/update`, { method: 'POST', body: {} });
      await loadStatus();
    } catch (err) {
      fail(err);
    } finally {
      updating = false;
    }
  }

  async function removeMachine() {
    if (deleting) return;
    if (!confirm(tr('Машина и все данные ботов в облаке будут удалены. Это нельзя отменить.'))) return;
    deleting = true;
    error = '';
    notice = '';
    try {
      await api(`${BASE}/delete`, { method: 'POST', body: { confirm: true } });
      await loadStatus();
    } catch (err) {
      fail(err);
    } finally {
      deleting = false;
    }
  }

  // Машина могла спать: сервер будит её в фоне, и мы повторяем запрос, пока не придёт ссылка.
  async function openCloud() {
    if (opening) return;
    opening = true;
    error = '';
    notice = '';
    try {
      await tryOpen();
    } finally {
      opening = false;
    }
  }

  async function tryOpen() {
    try {
      const data = await api(`${BASE}/open`, { method: 'POST', body: { open: true } });
      if (!alive) return;
      if (data?.ok) {
        notice = tr('Облако открыто в браузере.');
        wakeTimer = null;
        loadStatus();
        return;
      }
      if (data?.state === 'waking') {
        notice = tr('Машина просыпается, это занимает несколько секунд…');
        const wait = Math.max(WAKE_MS, (Number(data.retryAfterSeconds) || 0) * 1000);
        wakeTimer = setTimeout(tryOpen, wait);
        return;
      }
      notice = '';
      error = reason(data?.state, data?.retryAfterSeconds);
      loadStatus();
    } catch (err) {
      notice = '';
      if (alive) fail(err);
    }
  }

  function clock(ms) {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }

  function until(m) {
    const iso = m?.cooldownUntil ? Date.parse(m.cooldownUntil) : NaN;
    if (Number.isFinite(iso)) return clock(iso);
    const sec = Number(m?.retryAfterSeconds);
    return Number.isFinite(sec) && sec > 0 ? clock(Date.now() + sec * 1000) : '';
  }

  function reason(state, retry) {
    if (state === 'cooldown') {
      const at = retry > 0 ? clock(Date.now() + retry * 1000) : '';
      return at ? tr('Машина на перерыве до {time}.', { time: at }) : tr('Машина на перерыве.');
    }
    if (state === 'stopped') return tr('Машина остановлена в кабинете Instabox.');
    if (state === 'missing') return tr('Машина удалена в кабинете Instabox.');
    return tr('Сейчас облако открыть нельзя.');
  }

  const chip = $derived.by(() => {
    const m = machine;
    if (!m) return null;
    switch (m.state) {
      case 'running':
        return { tone: 'brand', text: tr('работает') };
      case 'starting':
        return { tone: 'neutral', text: tr('запускается') };
      case 'paused':
        return { tone: 'neutral', text: tr('спит') };
      case 'cooldown': {
        const at = until(m);
        return { tone: 'attention', text: at ? tr('пауза до {time}', { time: at }) : tr('пауза') };
      }
      case 'stopped':
        return { tone: 'attention', text: tr('остановлена') };
      case 'missing':
        return { tone: 'attention', text: tr('удалена в кабинете') };
      case 'unreachable':
      case 'signin_needed':
      case 'error':
        return { tone: 'attention', text: tr('нет связи') };
      default:
        return { tone: 'neutral', text: m.state };
    }
  });

  const jobTitle = $derived(
    job?.kind === 'delete'
      ? tr('Удаляю машину')
      : job?.kind === 'update'
        ? tr('Обновляю облако')
        : job?.kind === 'wake'
          ? tr('Бужу машину')
          : tr('Размещаю ботов в облаке'),
  );

  // Подписи шагов приходят от сервера по-русски; общие шаги переводим по id, остальное — как есть.
  const STEP_LABELS = {
    machine: () => tr('Создаю машину'),
    port: () => tr('Открываю порт'),
    keys: () => tr('Готовлю ключи'),
    files: () => tr('Кладу настройки'),
    docker: () => tr('Запускаю Deloist'),
    health: () => tr('Жду ответа сервера'),
    wake: () => tr('Бужу машину'),
    delete: () => tr('Удаляю машину'),
  };
  const stepLabel = (step) => (STEP_LABELS[step.id] ? STEP_LABELS[step.id]() : step.label);

  function stepState(i) {
    if (job?.state === 'done') return 'done';
    if (i < job.step) return 'done';
    if (i === job.step) return job.state === 'error' ? 'error' : 'active';
    return 'todo';
  }

  const signinProblem = $derived(
    signin?.phase === 'denied'
      ? tr('Вход отклонён. Попробуйте ещё раз.')
      : signin?.phase === 'expired'
        ? tr('Код истёк. Начните вход заново.')
        : signin?.phase === 'error'
          ? signin.error || tr('Не удалось войти в Instabox.')
          : '',
  );

  $effect(() => {
    untrack(() => {
      loadStatus();
    });
    return () => {
      alive = false;
      stopTimers();
    };
  });
</script>

<Section
  title={tr('Боты в облаке — бесплатно')}
  description={tr('Своя машина Instabox на ваше имя: боты отвечают в Telegram, даже когда компьютер выключен. Машина засыпает без дела и просыпается от сообщения; на бесплатном тарифе бывают паузы до 2 часов после долгой работы.')}
>
  {#if loading && !status}
    <p class="cl-note">{tr('Загружаю…')}</p>
  {:else if !status}
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="cl-actions"><Button onclick={loadStatus}>{tr('Повторить')}</Button></div>
  {:else if !status.signedIn}
    {#if pending}
      <div class="cl-code-box">
        <span class="cl-code-label">{tr('Код для проверки')}</span>
        <code class="cl-code" data-testid="cloud-code">{signin.userCode}</code>
        <p class="cl-note">{tr('В браузере открылась страница Instabox. Войдите через Google или GitHub, проверьте код и нажмите Approve.')}</p>
        <div class="cl-actions">
          <Button variant="primary" loading={reopening} onclick={reopenSignin}>{tr('Открыть страницу ещё раз')}</Button>
          <Button variant="ghost" onclick={cancelSignin}>{tr('Отмена')}</Button>
        </div>
      </div>
    {:else}
      {#if signinProblem}<p class="k-hint is-error" role="alert">{signinProblem}</p>{/if}
      <div class="cl-actions">
        <Button variant="primary" loading={starting} onclick={startSignin}>{tr('Подключить Instabox')}</Button>
      </div>
    {/if}
  {:else if jobRunning || (job?.state === 'error' && job.kind === 'provision' && !hasMachine)}
    <p class="cl-lead">{jobTitle}</p>
    <ol class="cl-steps">
      {#each job.steps || [] as step, i (step.id)}
        <li class="cl-step is-{stepState(i)}">{stepLabel(step)}</li>
      {/each}
    </ol>
    {#if job.state === 'error'}
      <p class="k-hint is-error" role="alert">{job.error || tr('Не удалось разместить ботов в облаке.')}</p>
      {#if job.retryAfterSeconds > 0}
        <p class="cl-note">{tr('Повторить можно примерно через {min} мин.', { min: Math.max(1, Math.ceil(job.retryAfterSeconds / 60)) })}</p>
      {/if}
      <div class="cl-actions">
        <Button variant="primary" loading={provisioning} onclick={provision}>{tr('Повторить')}</Button>
      </div>
    {:else}
      <p class="cl-note">{tr('Это занимает пару минут. Окно можно не закрывать.')}</p>
    {/if}
  {:else if !hasMachine}
    {#if machine?.state === 'missing'}
      <p class="k-hint is-error">{tr('Машина удалена в кабинете Instabox.')}</p>
    {/if}
    <p class="cl-note">{tr('Вход в Instabox выполнен{login}.', { login: status.login ? ` (${status.login})` : '' })}</p>
    <div class="cl-actions">
      <Button variant="primary" loading={provisioning} onclick={provision}>{tr('Разместить ботов в облаке')}</Button>
    </div>
  {:else}
    <div class="cl-head">
      <span class="cl-label">{tr('Состояние')}</span>
      {#if chip}<Badge tone={chip.tone} text={chip.text} data-testid="cloud-chip" />{/if}
    </div>
    {#if machine.origin}
      <div class="cl-addr">
        <span class="cl-label">{tr('Адрес облака')}</span>
        <code class="cl-origin" data-testid="cloud-origin">{machine.origin}</code>
      </div>
    {/if}
    {#if machine.error}<p class="k-hint is-error">{machine.error}</p>{/if}
    {#if job?.state === 'error'}
      <p class="k-hint is-error" role="alert">{job.error || tr('Не удалось выполнить действие.')}</p>
    {/if}
    {#if notice}<p class="cl-note" role="status">{notice}</p>{/if}
    <div class="cl-actions">
      <Button variant="primary" loading={opening} onclick={openCloud}>{tr('Открыть облако')}</Button>
      <Button loading={updating} onclick={update}>{tr('Обновить')}</Button>
      <Button variant="attention" loading={deleting} onclick={removeMachine}>{tr('Удалить машину')}</Button>
    </div>
  {/if}
  {#if error && status}
    <p class="k-hint is-error" role="alert">{error}</p>
  {/if}
</Section>

<style>
  :global(#sub-cloud .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .cl-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .cl-lead {
    margin: 0;
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .cl-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .cl-head,
  .cl-addr {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-2);
  }
  .cl-label,
  .cl-code-label {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .cl-origin {
    color: var(--k-text);
    font: 500 var(--k-text-sm) / 1.4 var(--k-font-mono);
    word-break: break-all;
    user-select: text;
  }
  .cl-code-box {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .cl-code {
    color: var(--k-text);
    font: 800 var(--k-text-3xl) / 1.1 var(--k-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.12em;
    user-select: text;
  }
  .cl-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .cl-step {
    position: relative;
    padding-left: 28px;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .cl-step::before {
    content: '';
    position: absolute;
    top: 4px;
    left: 4px;
    width: 12px;
    height: 12px;
    border-radius: var(--k-radius-pill);
    border: 2px solid var(--k-line);
  }
  .cl-step.is-active {
    color: var(--k-text);
    font-weight: 600;
  }
  .cl-step.is-active::before {
    border-color: var(--k-brand-text);
    background: var(--k-brand-tint);
  }
  .cl-step.is-done {
    color: var(--k-text-2);
  }
  .cl-step.is-done::before {
    border-color: var(--k-brand-text);
    background: var(--k-brand-text);
  }
  .cl-step.is-error {
    color: var(--k-text);
  }
  .cl-step.is-error::before {
    border-color: var(--k-attention-text, var(--k-text));
  }
  @media (max-width: 520px) {
    .cl-code {
      font-size: var(--k-text-2xl);
    }
  }
</style>
