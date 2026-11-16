<script>
  // Экран «Telegram» окна настроек: свой бот, привязка чата кодом и отключение моста.
  // Токен живёт только в поле ввода и в PUT: обратно его не показываем и никуда не пишем.
  import { untrack } from 'svelte';
  import Section from '../kit/Section.svelte';
  import Field from '../kit/Field.svelte';
  import TextInput from '../kit/TextInput.svelte';
  import Select from '../kit/Select.svelte';
  import Button from '../kit/Button.svelte';
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
  let code = $state('');
  let expired = $state(false);

  let pollTimer = null;
  let codeExpiresAt = 0;

  const botOptions = $derived(bots.map((bot) => ({ value: bot.id, label: bot.name })));
  const botName = $derived(
    (bots.find((bot) => bot.id === status?.botId) || {}).name || status?.botId || '',
  );

  // Первый бот команды выбран по умолчанию; если список обновился, устаревший выбор заменяем.
  $effect(() => {
    if (bots.length && !bots.some((bot) => bot.id === selectedBotId)) selectedBotId = bots[0].id;
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
      const next = await api('/api/telegram');
      status = next;
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
      if (next?.chatPaired) finishPairing();
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
    if (next?.chatPaired) finishPairing();
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
      const data = await api('/api/telegram', {
        method: 'PUT',
        body: { token: value, botId: selectedBotId },
      });
      token = '';
      status = {
        connected: true,
        enabled: true,
        botId: selectedBotId,
        chatPaired: false,
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
      const data = await api('/api/telegram/pair', { method: 'POST' });
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

  async function forget() {
    if (forgetting) return;
    if (!confirm(tr('Отключить Telegram? Бот перестанет отвечать в Telegram.'))) return;
    forgetting = true;
    error = '';
    try {
      await api('/api/telegram', { method: 'DELETE' });
      stopPolling();
      code = '';
      expired = false;
      status = { connected: false, enabled: false, botId: '', chatPaired: false, username: '' };
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
  <Section title={tr('Свой бот в Telegram')}>
    <p class="tg-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !status}
  <Section title={tr('Свой бот в Telegram')}>
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="tg-actions"><Button onclick={loadStatus}>{tr('Повторить')}</Button></div>
  </Section>
{:else if !status.connected}
  <Section
    title={tr('Свой бот в Telegram')}
    description={tr('Бот команды будет отвечать в Telegram от твоего имени. Токен хранится только на этом компьютере.')}
  >
    <ol class="tg-steps">
      <li>{tr('Открой @BotFather в Telegram и создай бота командой /newbot')}</li>
      <li>{tr('Скопируй токен, который он пришлёт')}</li>
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
    <div class="tg-actions">
      <Button variant="primary" loading={connecting} disabled={!token.trim() || !selectedBotId} onclick={connect}>
        {tr('Подключить')}
      </Button>
    </div>
  </Section>
{:else if !status.chatPaired}
  <Section title={tr('Осталось привязать чат')}>
    <p class="tg-lead">
      {status.username ? tr('Бот @{username} подключён.', { username: status.username }) : tr('Бот подключён.')}
    </p>
    {#if botName}<p class="tg-note">{tr('Отвечает: {bot}', { bot: botName })}</p>{/if}
    {#if code && !expired}
      <div class="tg-code-box">
        <span class="tg-code-label">{tr('Код для привязки')}</span>
        <code class="tg-code">{code}</code>
        <p class="tg-note">{tr('Отправь своему боту в Telegram: /start {code}. Код действует 10 минут.', { code })}</p>
        <div class="tg-actions">
          <Button variant="primary" loading={checking} onclick={checkPair}>{tr('Проверить')}</Button>
        </div>
      </div>
    {:else}
      {#if expired}<p class="k-hint is-error">{tr('Код истёк. Получи новый.')}</p>{/if}
      <div class="tg-actions">
        <Button variant="primary" loading={pairing} onclick={getCode}>{tr('Получить код')}</Button>
      </div>
    {/if}
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
  </Section>
{:else}
  <Section title={tr('Чат привязан')}>
    <p class="tg-lead">
      {status.username
        ? tr('Чат привязан. Пиши боту @{username} — отвечает {bot}.', { username: status.username, bot: botName })
        : tr('Чат привязан. Отвечает {bot}.', { bot: botName })}
    </p>
    <p class="tg-note">{tr('Чтобы сменить отвечающего бота, отключи Telegram и подключи снова: сервер проверяет токен при смене.')}</p>
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
    <div class="tg-actions">
      <Button variant="attention" loading={forgetting} onclick={forget}>{tr('Отключить')}</Button>
    </div>
  </Section>
{/if}

<style>
  /* Тело раздела — как у разделов настроек с классом s-pad: колонка с отступами сверху и снизу. */
  :global(#sub-telegram .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .tg-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: tg-step;
  }
  .tg-steps li {
    position: relative;
    padding-left: 32px;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    counter-increment: tg-step;
  }
  .tg-steps li::before {
    content: counter(tg-step);
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
  .tg-lead {
    margin: 0;
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .tg-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .tg-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .tg-code-box {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .tg-code-label {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .tg-code {
    color: var(--k-text);
    font: 800 var(--k-text-3xl) / 1.1 var(--k-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.12em;
  }
  @media (max-width: 520px) {
    .tg-code {
      font-size: var(--k-text-2xl);
    }
  }
</style>
