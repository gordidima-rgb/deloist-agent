<script>
  // Экран «Slack» окна настроек: своё приложение, привязка личных сообщений кодом и отключение моста.
  // Оба токена живут только в полях ввода и в PUT: обратно их не показываем и никуда не пишем.
  import { untrack } from 'svelte';
import { Section, Field, TextInput, Select, Button } from '../kit';
  import { tr } from '../../lib/i18n.js';

  let { api, bots = [], onchanged } = $props();

  const POLL_MS = 3000;
  const CODE_TTL_MS = 10 * 60 * 1000;

  // Готовый манифест: Socket Mode, личные сообщения и четыре права бота.
  const MANIFEST = `display_information:
  name: Deloist
features:
  bot_user:
    display_name: Deloist
    always_online: false
  app_home:
    home_tab_enabled: false
    messages_tab_enabled: true
    messages_tab_read_only_enabled: false
oauth_config:
  scopes:
    bot:
      - chat:write
      - im:history
      - im:read
      - users:read
settings:
  event_subscriptions:
    bot_events:
      - message.im
  socket_mode_enabled: true
  token_rotation_enabled: false`;

  let status = $state(null);
  let loading = $state(true);
  let error = $state('');
  let botToken = $state('');
  let appToken = $state('');
  let selectedBotId = $state('');
  let connecting = $state(false);
  let pairing = $state(false);
  let checking = $state(false);
  let forgetting = $state(false);
  let changingBot = $state(false);
  let code = $state('');
  let expired = $state(false);
  let copied = $state(false);

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
      const next = await api('/api/slack');
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

  async function copyManifest() {
    try {
      await navigator.clipboard?.writeText(MANIFEST);
    } catch {
      // Буфер обмена может быть закрыт: текст манифеста всё равно видно на экране.
    }
    copied = true;
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
    const bot = botToken.trim();
    const app = appToken.trim();
    if (!bot || !app || !selectedBotId) return;
    connecting = true;
    error = '';
    try {
      const data = await api('/api/slack', {
        method: 'PUT',
        body: { botToken: bot, appToken: app, botId: selectedBotId },
      });
      botToken = '';
      appToken = '';
      copied = false;
      status = {
        connected: true,
        enabled: true,
        botId: selectedBotId,
        paired: false,
        team: data?.team || '',
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
      const data = await api('/api/slack/pair', { method: 'POST' });
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

  // Смена отвечающего бота: сервер разрешает PUT без токенов, пока мост подключён.
  async function changeBot(event) {
    const botId = event?.currentTarget?.value || selectedBotId;
    if (changingBot || !botId || botId === status?.botId) return;
    changingBot = true;
    error = '';
    try {
      const data = await api('/api/slack', { method: 'PUT', body: { botId } });
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
    if (!confirm(tr('Отключить Slack? Бот перестанет отвечать в личных сообщениях Slack.'))) return;
    forgetting = true;
    error = '';
    try {
      await api('/api/slack', { method: 'DELETE' });
      stopPolling();
      code = '';
      expired = false;
      copied = false;
      botToken = '';
      appToken = '';
      status = { connected: false, enabled: false, botId: '', paired: false, team: '', username: '' };
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
  <Section title={tr('Свой бот в Slack')}>
    <p class="sl-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !status}
  <Section title={tr('Свой бот в Slack')}>
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="sl-actions"><Button onclick={loadStatus}>{tr('Повторить')}</Button></div>
  </Section>
{:else if !status.connected}
  <Section
    title={tr('Свой бот в Slack')}
    description={tr('Бот команды будет отвечать в Slack от твоего имени. Оба токена хранятся только на этом компьютере.')}
  >
    <ol class="sl-steps">
      <li>{tr('Открой api.slack.com/apps → Create New App → From a manifest, выбери свой workspace и вставь этот текст')}</li>
      <li>{tr('Install to Workspace')}</li>
      <li>{tr('Скопируй Bot User OAuth Token (xoxb-…)')}</li>
      <li>{tr('Basic Information → App-Level Tokens → Generate с правом connections:write (xapp-…)')}</li>
    </ol>
    <div class="sl-manifest-box">
      <div class="sl-manifest-head">
        <span class="sl-manifest-label">{tr('Manifest приложения')}</span>
        <Button size="sm" onclick={copyManifest}>{copied ? tr('Скопировано') : tr('Скопировать')}</Button>
      </div>
      <pre class="sl-manifest"><code>{MANIFEST}</code></pre>
    </div>
    <Field label={tr('Токен бота (xoxb-…)')} hint={tr('Токены показываются один раз: после сохранения их не увидеть.')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={botToken} />
      {/snippet}
    </Field>
    <Field label={tr('Токен приложения (xapp-…)')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={appToken} />
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
    <div class="sl-actions">
      <Button variant="primary" loading={connecting} disabled={!botToken.trim() || !appToken.trim() || !selectedBotId} onclick={connect}>
        {tr('Подключить')}
      </Button>
    </div>
  </Section>
{:else if !status.paired}
  <Section title={tr('Осталось привязать личку')}>
    <p class="sl-lead">
      {status.username ? tr('Бот {username} подключён.', { username: status.username }) : tr('Бот подключён.')}
    </p>
    {#if status.team}<p class="sl-note">{tr('Рабочее пространство: {team}', { team: status.team })}</p>{/if}
    {#if botName}<p class="sl-note">{tr('Отвечает: {bot}', { bot: botName })}</p>{/if}
    {#if code && !expired}
      <div class="sl-code-box">
        <span class="sl-code-label">{tr('Код для привязки')}</span>
        <code class="sl-code">{code}</code>
        <p class="sl-note">{tr('Напиши боту в Slack в разделе Messages: start {code}. Код действует 10 минут.', { code })}</p>
        <div class="sl-actions">
          <Button variant="primary" loading={checking} onclick={checkPair}>{tr('Проверить')}</Button>
        </div>
      </div>
    {:else}
      {#if expired}<p class="k-hint is-error">{tr('Код истёк. Получи новый.')}</p>{/if}
      <div class="sl-actions">
        <Button variant="primary" loading={pairing} onclick={getCode}>{tr('Получить код')}</Button>
      </div>
    {/if}
    {#if error}
      <p class="k-hint is-error" role="alert">{error}</p>
    {/if}
  </Section>
{:else}
  <Section title={tr('Личка привязана')}>
    <p class="sl-lead">
      {status.username
        ? tr('Личка привязана. Пиши боту {username} в Slack — отвечает {bot}.', { username: status.username, bot: botName })
        : tr('Личка привязана. Отвечает {bot}.', { bot: botName })}
    </p>
    {#if status.team}<p class="sl-note">{tr('Рабочее пространство: {team}', { team: status.team })}</p>{/if}
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
    <div class="sl-actions">
      <Button variant="attention" loading={forgetting} onclick={forget}>{tr('Отключить')}</Button>
    </div>
  </Section>
{/if}

<style>
  /* Тело раздела — как у разделов настроек с классом s-pad: колонка с отступами сверху и снизу. */
  :global(#sub-slack .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .sl-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: sl-step;
  }
  .sl-steps li {
    position: relative;
    padding-left: 32px;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    counter-increment: sl-step;
  }
  .sl-steps li::before {
    content: counter(sl-step);
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
  .sl-lead {
    margin: 0;
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .sl-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .sl-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .sl-manifest-box {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-3);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .sl-manifest-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2);
  }
  .sl-manifest-label {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .sl-manifest {
    max-height: 280px;
    margin: 0;
    overflow: auto;
    color: var(--k-text);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-mono);
    white-space: pre;
  }
  .sl-code-box {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .sl-code-label {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .sl-code {
    color: var(--k-text);
    font: 800 var(--k-text-3xl) / 1.1 var(--k-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.12em;
  }
  @media (max-width: 520px) {
    .sl-code {
      font-size: var(--k-text-2xl);
    }
  }
</style>
