<script>
  // "Connect" for an app: consent card from the manifest, then the key form or the OAuth button,
  // then a check that lists the tools. Every step goes to /api/catalog/{id}/connect, which hands it
  // to the connectors module. Secrets are sent once and never kept here.
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { api, serverText, lang } from '../../lib/api.js';

  let { item, onchanged } = $props();

  const card = $derived(item.card || {});
  const fields = $derived(card.auth?.fields || []);
  const oauth = $derived(card.auth?.kind === 'oauth' || card.auth?.kind === 'mcp-oauth');
  // consent, then auth (key or OAuth), then done. A half-finished connect resumes at auth.
  let step = $state(untrack(() => (item.state?.status === 'pending' ? 'auth' : 'consent')));
  let busy = $state(false);
  let error = $state('');
  let unavailable = $state(false);
  let values = $state({});
  let saved = $state({});
  let oauthUrl = $state('');
  // The sign-in server has no client registration: ask for a client ID (and optional secret) once.
  let needClient = $state(false);
  let authHost = $state('');
  let client = $state({ id: '', secret: '', port: '' });
  let tools = $state.raw([]);

  const base = $derived(`/api/catalog/${encodeURIComponent(item.id)}`);
  const allSaved = $derived(fields.every((f) => saved[f.name]));

  async function run(fn) {
    busy = true;
    error = '';
    try {
      await fn();
    } catch (err) {
      if (err.status === 503) unavailable = true;
      error = serverText(err.message);
    } finally {
      busy = false;
    }
  }

  const allow = () =>
    run(async () => {
      await api(`${base}/install`, { method: 'POST', body: { lang: lang() } });
      await api(`${base}/connect`, { method: 'POST', body: { step: 'confirm' } });
      step = 'auth';
      onchanged?.();
    });

  const sendSecret = (field) =>
    run(async () => {
      await api(`${base}/connect`, { method: 'POST', body: { step: 'secret', name: field.name, value: values[field.name] || '' } });
      saved = { ...saved, [field.name]: true };
      values = { ...values, [field.name]: '' };
    });

  const startOauth = () =>
    run(async () => {
      const body = { step: 'oauth' };
      if (needClient && client.id.trim()) {
        body.clientId = client.id.trim();
        if (client.secret) body.clientSecret = client.secret;
        if (client.port) body.redirectPort = Number(client.port) || 0;
      }
      try {
        const data = await api(`${base}/connect`, { method: 'POST', body });
        oauthUrl = data.url;
        needClient = false;
        client = { id: '', secret: '', port: '' };
        window.open(data.url, '_blank', 'noopener');
      } catch (err) {
        if (err.data?.needsClientId) {
          needClient = true;
          authHost = err.data.authHost || '';
          return;
        }
        throw err;
      }
    });

  const check = () =>
    run(async () => {
      const data = await api(`${base}/connect`, { method: 'POST', body: { step: 'check' } });
      tools = data.tools || [];
      step = 'done';
      onchanged?.();
    });
</script>

<div class="flow">
  {#if step === 'consent'}
    <div class="k-approval">
      <div class="k-approval__head"><span class="k-approval__title">{tr('Подключить {name}?', { name: card.title })}</span></div>
      <p class="say">{tr('Вот что произойдёт. Ничего не включится без вашего «Разрешить».')}</p>
      <div class="k-approval__what">
        {#if card.transport === 'http'}
          <div>{tr('Адрес сервера: {url}', { url: card.url })}</div>
        {:else}
          <div>{tr('Команда: {command}', { command: [card.command, ...(card.args || [])].join(' ') })}</div>
          {#if card.install?.package}
            <div>{tr('Пакет: {package}, версия закреплена: {pin}', { package: card.install.package, pin: card.install.pin })}</div>
          {/if}
        {/if}
        {#if card.egress?.length}
          <div>{tr('Ключ уходит только на: {hosts}', { hosts: card.egress.join(', ') })}</div>
        {/if}
        {#if fields.length}
          <div>{tr('Спросим: {fields}', { fields: fields.map((f) => f.label).join(', ') })}</div>
        {/if}
        {#if oauth}<div>{tr('Вход через сайт сервиса в вашем браузере')}</div>{/if}
      </div>
      <p class="say">{tr('Каждый вызов инструментов этого приложения бот сначала покажет вам.')}</p>
      <div class="k-approval__actions">
        <button type="button" class="k-btn k-btn--primary" disabled={busy || unavailable} onclick={allow}>{tr('Разрешить и подключить')}</button>
      </div>
    </div>
  {/if}

  {#if step === 'auth' || (step === 'consent' && unavailable)}
    <div class="auth">
      {#each fields as field (field.name)}
        <label class="k-field">
          <span class="k-label">{field.label}</span>
          <span class="cat-row">
            <input
              class="k-input"
              type={field.secret ? 'password' : 'text'}
              autocomplete="off"
              spellcheck="false"
              disabled={busy || unavailable || step !== 'auth'}
              placeholder={saved[field.name] ? tr('Сохранено. Вставьте новый, чтобы заменить.') : ''}
              bind:value={values[field.name]}
            />
            <button
              type="button"
              class="k-btn"
              disabled={busy || unavailable || step !== 'auth' || !values[field.name]}
              onclick={() => sendSecret(field)}>{tr('Сохранить')}</button
            >
          </span>
          {#if field.help}<span class="k-hint">{field.help}</span>{/if}
          {#if field.helpUrl}<a class="k-hint cat-link" href={field.helpUrl} target="_blank" rel="noopener noreferrer">{tr('Где взять')}</a>{/if}
        </label>
      {/each}
      {#if oauth && needClient}
        <div class="k-field" data-testid="client-form">
          <span class="k-hint">{tr('Сервер входа {host} не умеет регистрировать приложения сам. Создайте приложение в его настройках и впишите client ID.', { host: authHost })}</span>
          <label class="k-field"><span class="k-label">{tr('Client ID')}</span><input class="k-input" autocomplete="off" spellcheck="false" bind:value={client.id} /></label>
          <label class="k-field"><span class="k-label">{tr('Client secret (если есть)')}</span><input class="k-input" type="password" autocomplete="off" spellcheck="false" bind:value={client.secret} /></label>
          <label class="k-field"><span class="k-label">{tr('Порт возврата (если сервер просит точный адрес)')}</span><input class="k-input" inputmode="numeric" placeholder="http://127.0.0.1:PORT/cb" bind:value={client.port} /></label>
        </div>
      {/if}
      {#if oauth}
        <button type="button" class="k-btn k-btn--ink" disabled={busy || unavailable || step !== 'auth' || (needClient && !client.id.trim())} onclick={startOauth}>
          {needClient ? tr('Продолжить вход') : tr('Войти через {name}', { name: card.title })}
        </button>
        {#if oauthUrl}<p class="k-hint">{tr('Окно входа открылось в браузере. Вернитесь сюда и нажмите «Проверить».')}</p>{/if}
      {/if}
      <button type="button" class="k-btn k-btn--primary" disabled={busy || unavailable || step !== 'auth' || (!oauth && !allSaved)} onclick={check}>
        {tr('Проверить')}
      </button>
    </div>
  {/if}

  {#if step === 'done'}
    <div class="done" role="status">
      <p>{tr('Подключено. Инструментов: {count}.', { count: tools.length })}</p>
      {#if tools.length}
        <ul>
          {#each tools.slice(0, 12) as tool (tool.name || tool)}<li><code>{tool.name || tool}</code></li>{/each}
        </ul>
      {/if}
    </div>
  {/if}

  {#if unavailable}
    <p class="cat-note" role="status">{tr('В этой сборке подключение приложений ещё не работает: оно появится вместе с модулем коннекторов. Карточка выше показывает, что будет спрошено.')}</p>
  {:else if error}
    <p class="k-hint is-error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .flow {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
  }
  .say {
    margin: 0;
    color: var(--k-text-2, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  .auth {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--k-space-3, 12px);
  }
  .auth .k-field {
    align-self: stretch;
  }
  .cat-row {
    display: flex;
    gap: var(--k-space-2, 8px);
  }
  .cat-row .k-input {
    flex: 1;
    min-width: 0;
  }
  .cat-link {
    color: var(--k-brand-text, #0250e0);
  }
  .cat-note {
    margin: 0;
    padding: var(--k-space-2, 8px) var(--k-space-3, 12px);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface-2, #f0f0f0);
    color: var(--k-text-2, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  .done p {
    margin: 0;
  }
  .done ul {
    margin: 6px 0 0;
    padding-left: 18px;
    font-size: var(--k-text-sm, 13px);
  }
</style>
