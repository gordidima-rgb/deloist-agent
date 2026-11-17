<script>
  // Экран «Discord» окна настроек: свой бот, привязка личных сообщений кодом и отключение моста.
  // Токен живёт только в поле ввода и в PUT: обратно его не показываем и никуда не пишем.
  import { untrack } from 'svelte';
import { Section, Field, TextInput, Select, Button } from '../kit';
  import { tr } from '../../lib/i18n.js';

  let { api, bots = [], onchanged } = $props();

  const POLL_MS = 3000;
  const CODE_TTL_MS = 10 * 60 * 1000;

  let status = $state(null);
  let loading = $state(true);
  let error = $state('');
  let token = $state('');
  let selectedBotId = $state('');
  let connecting = $state(false);
  let pairing = $state(false);
  let checking = $state(false);
  let forgetting = $state(false);
  let changingBot = $state(false);
  let code = $state('');
  let expired = $state(false);

  let pollTimer = null;
  let codeExpiresAt = 0;

  const botOptions = $derived(bots.map((bot) => ({ value: bot.id, label: bot.name })));
  const botName = $derived(
    (bots.find((bot) => bot.id === status?.botId) || {}).name || status?.botId || '',
  );

  // Если список ботов обновился, устаревший выбор заменяем: сначала отвечающим из статуса, потом первым.
  $effect(() => {
    if (!bots.length || bots.some((bot) => bot.id === selectedBotId)) return;
    const preferred = bots.some((bot) => bot.id === status?.botId) ? status.botId : bots[0].id;
    selectedBotId = preferred;
  });

  function stopPolling() {
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function fail(err) {
    error = err?.message || tr('Не удалось выполнить запрос');
  }

  async function loadStatus() {
    try {
      const next = await api('/api/discord');
      status = next;
      if (next?.botId && bots.some((bot) => bot.id === next.botId)) selectedBotId = next.botId;
      else if (!bots.some((bot) => bot.id === selectedBotId) && bots.length) selectedBotId = bots[0].id;
      error = '';
      return next;
    } catch (err) {
      fail(err);
      return null;
    } finally {
      loading = false;
    }
  }

  function finishPairing() {
    stopPolling();
    code = '';
    expired = false;
  }

  async function checkPair() {
    if (checking) return;
    checking = true;
    try {
      const next = await loadStatus();
      if (next?.paired) finishPairing();
      else if (Date.now() >= codeExpiresAt) {
        stopPolling();
        code = '';
        expired = true;
      }
    } finally {
      checking = false;
    }
  }

  async function pollPair() {
    if (Date.now() >= codeExpiresAt) {
      stopPolling();
      code = '';
      expired = true;
      return;
    }
    const next = await loadStatus();
    if (next?.paired) finishPairing();
  }

  function startPolling(deadline) {
    stopPolling();
    codeExpiresAt = deadline;
    pollTimer = setInterval(pollPair, POLL_MS);
  }

  async function connect() {
    if (connecting) return;
    const value = token.trim();
    if (!value || !selectedBotId) return;
    connecting = true;
    error = '';
    try {
      const data = await api('/api/discord', {
        method: 'PUT',
        body: { token: value, botId: selectedBotId },
      });
      token = '';
      status = {
        connected: true,
        enabled: true,
        botId: selectedBotId,
        paired: false,
        username: data?.username || '',
      };
      onchanged?.();
    } catch (err) {
      fail(err);
    } finally {
      connecting = false;
    }
  }

  async function getCode() {
    if (pairing) return;
    pairing = true;
    error = '';
    expired = false;
    try {
      const data = await api('/api/discord/pair', { method: 'POST' });
      code = String(data?.code || '');
      const at = Number(data?.expiresAt);
      const deadline = Number.isFinite(at) && at > Date.now() ? at : Date.now() + CODE_TTL_MS;
      if (code) startPolling(deadline);
    } catch (err) {
      fail(err);
    } finally {
      pairing = false;
    }
  }

  // Смена отвечающего бота: сервер разрешает PUT без токена, пока мост подключён.
  async function changeBot(event) {
    const botId = event?.currentTarget?.value || selectedBotId;
    if (changingBot || !botId || botId === status?.botId) return;
    changingBot = true;
    error = '';
    try {
      const data = await api('/api/discord', { method: 'PUT', body: { botId } });
      status = { ...status, botId, username: data?.username || status?.username || '' };
    } catch (err) {
      selectedBotId = status?.botId || selectedBotId;
      fail(err);
    } finally {
      changingBot = false;
    }
  }

  async function forget() {
    if (forgetting) return;
    if (!confirm(tr('Отключить Discord? Бот перестанет отвечать в личных сообщениях Discord.'))) return;
    forgetting = true;
    error = '';
    try {
      await api('/api/discord', { method: 'DELETE' });
      stopPolling();
      code = '';
      expired = false;
      status = { connected: false, enabled: false, botId: '', paired: false, username: '' };
      onchanged?.();
    } catch (err) {
      fail(err);
    } finally {
      forgetting = false;
    }
  }

  $effect(() => {
    untrack(() => {
      loadStatus();
    });
    return () => stopPolling();
  });
</script>

{#if loading && !status}
  <Section title={tr('Свой бот в Discord')}>
    <p class="dc-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !status}
  <Section title={tr('Свой бот в Discord')}>
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="dc-actions"><Button onclick={loadStatus}>{tr('Повторить')}</Button></div>
  </Section>
{:else if !status.connected}
  <Section
    title={tr('Свой бот в Discord')}
    description={tr('Бот команды будет отвечать в Discord от твоего имени. Токен хранится только на этом компьютере.')}
  >
    <ol class="dc-steps">
      <li>{tr('Открой discord.com/developers/applications и создай приложение (New Application)')}</li>
      <li>{tr('Во вкладке Bot нажми Reset Token и скопируй токен')}</li>
      <li>{tr('Во вкладке OAuth2 → URL Generator отметь bot, открой ссылку и добавь бота на свой сервер')}</li>
      <li>{tr('Вставь токен сюда и выбери, кто будет отвечать')}</li>
    </ol>
    <Field label={tr('Токен бота')} hint={tr('Токен показывается один раз: после сохранения его не увидеть.')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={token} />
      {/snippet}
    </Field>
    <Field label={tr('Кто отвечает')}>
      {#snippet children(f)}
        <Select id={f.id} options={botOptions} bind:value={selectedBotId} disabled={!bots.length} />
      {/snippet}
    </Field>
    {#if !bots.length}
      <p class="k-hint is-error">{tr('Сначала создай хотя бы одного бота')}</p>
    {/if}
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
    <div class="dc-actions">
      <Button variant="primary" loading={connecting} disabled={!token.trim() || !selectedBotId} onclick={connect}>
        {tr('Подключить')}
      </Button>
    </div>
  </Section>
{:else if !status.paired}
  <Section title={tr('Осталось привязать личку')}>
    <p class="dc-lead">
      {status.username ? tr('Бот {username} подключён.', { username: status.username }) : tr('Бот подключён.')}
    </p>
    {#if botName}<p class="dc-note">{tr('Отвечает: {bot}', { bot: botName })}</p>{/if}
    {#if code && !expired}
      <div class="dc-code-box">
        <span class="dc-code-label">{tr('Код для привязки')}</span>
        <code class="dc-code">{code}</code>
        <p class="dc-note">{tr('Напиши своему боту в личные сообщения Discord: start {code}. Код действует 10 минут.', { code })}</p>
        <div class="dc-actions">
          <Button variant="primary" loading={checking} onclick={checkPair}>{tr('Проверить')}</Button>
        </div>
      </div>
    {:else}
      {#if expired}<p class="k-hint is-error">{tr('Код истёк. Получи новый.')}</p>{/if}
      <div class="dc-actions">
        <Button variant="primary" loading={pairing} onclick={getCode}>{tr('Получить код')}</Button>
      </div>
    {/if}
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
  </Section>
{:else}
  <Section title={tr('Личка привязана')}>
    <p class="dc-lead">
      {status.username
        ? tr('Личка привязана. Пиши боту {username} — отвечает {bot}.', { username: status.username, bot: botName })
        : tr('Личка привязана. Отвечает {bot}.', { bot: botName })}
    </p>
    <Field label={tr('Кто отвечает')}>
      {#snippet children(f)}
        <Select
          id={f.id}
          options={botOptions}
          bind:value={selectedBotId}
          disabled={!bots.length || changingBot}
          onchange={changeBot}
        />
      {/snippet}
    </Field>
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
    <div class="dc-actions">
      <Button variant="attention" loading={forgetting} onclick={forget}>{tr('Отключить')}</Button>
    </div>
  </Section>
{/if}

<style>
  /* Тело раздела — как у разделов настроек с классом s-pad: колонка с отступами сверху и снизу. */
  :global(#sub-discord .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .dc-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: dc-step;
  }
  .dc-steps li {
    position: relative;
    padding-left: 32px;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    counter-increment: dc-step;
  }
  .dc-steps li::before {
    content: counter(dc-step);
    position: absolute;
    top: 0;
    left: 0;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: var(--k-radius-pill);
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
    font: 700 var(--k-text-xs) / 1 var(--k-font-body);
  }
  .dc-lead {
    margin: 0;
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .dc-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .dc-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .dc-code-box {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .dc-code-label {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .dc-code {
    color: var(--k-text);
    font: 800 var(--k-text-3xl) / 1.1 var(--k-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.12em;
  }
  @media (max-width: 520px) {
    .dc-code {
      font-size: var(--k-text-2xl);
    }
  }
</style>
