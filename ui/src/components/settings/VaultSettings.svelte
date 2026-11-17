<script>
  // Экран «Менеджер паролей»: Bitwarden, 1Password и проверка сохранённых входов для сайта.
  // Мастер-пароль и токен живут только в поле ввода и в запросе: обратно их не показываем и никуда не пишем.
  import { untrack } from 'svelte';
import { Section, Field, TextInput, Button } from '../kit';
  import { tr } from '../../lib/i18n.js';

  let { api } = $props();

  let status = $state(null);
  let loading = $state(true);
  let loadError = $state('');

  let masterPassword = $state('');
  let unlocking = $state(false);
  let locking = $state(false);
  let bitwardenError = $state('');

  let opToken = $state('');
  let connecting = $state(false);
  let forgetting = $state(false);
  let onepasswordError = $state('');

  let siteAddress = $state('');
  let siteOrigin = $state('');
  let siteChecking = $state(false);
  let siteChecked = $state(false);
  let siteLogins = $state([]);
  let siteError = $state('');

  const bitwarden = $derived(status?.bitwarden ?? null);
  const onepassword = $derived(status?.onepassword ?? null);

  function message(err) {
    return err?.message || tr('Не удалось выполнить запрос');
  }

  async function loadStatus() {
    try {
      status = await api('/api/vault');
      loadError = '';
      return status;
    } catch (err) {
      loadError = message(err);
      return null;
    } finally {
      loading = false;
    }
  }

  function patchBitwarden(next) {
    if (!status) return;
    status = { ...status, bitwarden: { ...status.bitwarden, ...next } };
  }

  function patchOnepassword(next) {
    if (!status) return;
    status = { ...status, onepassword: { ...status.onepassword, ...next } };
  }

  async function unlock() {
    if (unlocking) return;
    unlocking = true;
    bitwardenError = '';
    try {
      const data = await api('/api/vault/bitwarden/unlock', {
        method: 'POST',
        body: { password: masterPassword },
      });
      masterPassword = '';
      patchBitwarden({ status: data?.status || 'unlocked' });
    } catch (err) {
      bitwardenError = message(err);
    } finally {
      masterPassword = '';
      unlocking = false;
    }
  }

  async function lock() {
    if (locking) return;
    locking = true;
    bitwardenError = '';
    try {
      const data = await api('/api/vault/bitwarden/lock', { method: 'POST' });
      patchBitwarden({ status: data?.status || 'locked' });
    } catch (err) {
      bitwardenError = message(err);
    } finally {
      locking = false;
    }
  }

  async function connectOnepassword() {
    if (connecting) return;
    const token = opToken.trim();
    if (!token) return;
    connecting = true;
    onepasswordError = '';
    try {
      const data = await api('/api/vault/onepassword', { method: 'PUT', body: { token } });
      opToken = '';
      patchOnepassword({ connected: data?.connected ?? true });
    } catch (err) {
      onepasswordError = message(err);
    } finally {
      opToken = '';
      connecting = false;
    }
  }

  async function disconnectOnepassword() {
    if (forgetting) return;
    if (!confirm(tr('Отключить 1Password? Бот перестанет входить на сайты с его помощью.'))) return;
    forgetting = true;
    onepasswordError = '';
    try {
      await api('/api/vault/onepassword', { method: 'DELETE' });
      patchOnepassword({ connected: false });
    } catch (err) {
      onepasswordError = message(err);
    } finally {
      forgetting = false;
    }
  }

  // Из адреса с путём или без схемы делаем https://host — только его принимает сервер.
  function originFromAddress(raw) {
    const text = String(raw ?? '').trim();
    if (!text) return '';
    const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `https://${text}`;
    try {
      const url = new URL(withScheme);
      return url.hostname ? `https://${url.host}` : '';
    } catch {
      return '';
    }
  }

  async function checkSite() {
    if (siteChecking) return;
    const origin = originFromAddress(siteAddress);
    siteChecked = false;
    siteLogins = [];
    siteOrigin = '';
    siteError = '';
    if (!origin) {
      siteError = tr('Введи адрес сайта, например example.com');
      return;
    }
    siteOrigin = origin;
    siteChecking = true;
    try {
      const data = await api(`/api/vault/logins?origin=${encodeURIComponent(origin)}`);
      siteLogins = Array.isArray(data?.logins) ? data.logins : [];
      siteChecked = true;
    } catch (err) {
      siteError = message(err);
    } finally {
      siteChecking = false;
    }
  }

  function sourceLabel(source) {
    if (source === 'bitwarden') return tr('Bitwarden');
    if (source === '1password') return tr('1Password');
    return source;
  }

  $effect(() => {
    untrack(() => {
      loadStatus();
    });
  });
</script>

<p class="pv-note">{tr('Пароли не попадают в чат и к модели: программа сама вставляет их в поле пароля на сайте, для которого вход сохранён. Первый вход бота на каждый сайт ты разрешаешь карточкой.')}</p>

{#if loading && !status}
  <Section title={tr('Bitwarden')}>
    <p class="pv-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !status}
  <Section title={tr('Менеджер паролей')}>
    <p class="k-hint is-error" role="alert">{loadError}</p>
    <div class="pv-actions"><Button onclick={loadStatus}>{tr('Повторить')}</Button></div>
  </Section>
{:else}
  <Section title={tr('Bitwarden')}>
    {#if !bitwarden.cli}
      <p class="pv-text">{tr('Нужна программа Bitwarden CLI (bw). Установи её с bitwarden.com/help/cli, выполни в терминале bw login и вернись сюда.')}</p>
      <div class="pv-actions">
        <Button loading={loading} onclick={loadStatus}>{tr('Проверить снова')}</Button>
      </div>
    {:else if bitwarden.status === 'unauthenticated'}
      <p class="pv-text">{tr('Выполни в терминале bw login, потом нажми «Проверить снова»')}</p>
      <div class="pv-actions">
        <Button loading={loading} onclick={loadStatus}>{tr('Проверить снова')}</Button>
      </div>
    {:else if bitwarden.status === 'locked'}
      <Field label={tr('Мастер-пароль')} error={bitwardenError}>
        {#snippet children(f)}
          <TextInput
            id={f.id}
            type="password"
            autocomplete="off"
            spellcheck="false"
            bind:value={masterPassword}
          />
        {/snippet}
      </Field>
      <div class="pv-actions">
        <Button variant="primary" loading={unlocking} disabled={!masterPassword} onclick={unlock}>
          {tr('Разблокировать')}
        </Button>
      </div>
    {:else}
      <p class="pv-lead">
        {tr('Открыт для {email}. Бот вводит пароли сам, ты их не видишь; сам замок закроется через 30 минут без входов.', { email: bitwarden.email })}
      </p>
      {#if bitwardenError}
        <p class="k-hint is-error" role="alert">{bitwardenError}</p>
      {/if}
      <div class="pv-actions">
        <Button variant="attention" loading={locking} onclick={lock}>{tr('Заблокировать')}</Button>
      </div>
    {/if}
  </Section>

  <Section title={tr('1Password')}>
    {#if !onepassword.cli}
      <p class="pv-text">{tr('Нужна программа 1Password CLI (op): developer.1password.com/docs/cli')}</p>
      <div class="pv-actions">
        <Button loading={loading} onclick={loadStatus}>{tr('Проверить снова')}</Button>
      </div>
    {:else if !onepassword.connected}
      <ol class="pv-steps">
        <li>{tr('Создай service account в 1Password (Developer → Service Accounts) с доступом только к нужному хранилищу')}</li>
        <li>{tr('Скопируй токен (начинается с ops_)')}</li>
      </ol>
      <Field label={tr('Токен 1Password')} error={onepasswordError}>
        {#snippet children(f)}
          <TextInput
            id={f.id}
            type="password"
            autocomplete="off"
            spellcheck="false"
            bind:value={opToken}
          />
        {/snippet}
      </Field>
      <div class="pv-actions">
        <Button
          variant="primary"
          loading={connecting}
          disabled={!opToken.trim()}
          onclick={connectOnepassword}
        >
          {tr('Подключить')}
        </Button>
      </div>
    {:else}
      <p class="pv-lead">{tr('Подключено')}</p>
      {#if onepasswordError}
        <p class="k-hint is-error" role="alert">{onepasswordError}</p>
      {/if}
      <div class="pv-actions">
        <Button variant="attention" loading={forgetting} onclick={disconnectOnepassword}>
          {tr('Отключить')}
        </Button>
      </div>
    {/if}
  </Section>

  <section class="k-section pv-site">
    <header class="k-section__head">
      <div class="k-section__text">
        <h2 class="k-section__title">{tr('Проверить сайт')}</h2>
      </div>
    </header>
    <div class="k-section__body">
      <Field label={tr('Адрес сайта')} error={siteError}>
        {#snippet children(f)}
          <div class="pv-site-row">
            <TextInput
              id={f.id}
              type="text"
              autocomplete="off"
              spellcheck="false"
              placeholder={tr('example.com/login')}
              bind:value={siteAddress}
            />
            <Button variant="primary" loading={siteChecking} onclick={checkSite}>
              {tr('Проверить')}
            </Button>
          </div>
        {/snippet}
      </Field>
      {#if siteChecked}
        {#if siteLogins.length}
          <ul class="pv-logins">
            {#each siteLogins as login (login.id)}
              <li class="pv-login">
                <span class="pv-login__title">{login.title || tr('Без названия')}</span>
                {#if login.username}
                  <span class="pv-login__user">{login.username}</span>
                {/if}
                <span class="pv-login__meta">
                  <span class="pv-badge">{sourceLabel(login.source)}</span>
                  {#if login.hasTotp}
                    <span class="pv-badge pv-badge--code">{tr('есть код')}</span>
                  {/if}
                </span>
              </li>
            {/each}
          </ul>
          <p class="pv-note">{tr('Входы для {origin}', { origin: siteOrigin })}</p>
        {:else}
          <p class="pv-text">{tr('Для этого сайта входов нет')}</p>
        {/if}
      {/if}
    </div>
  </section>
{/if}

<style>
  /* Тело раздела — как у соседних экранов настроек: колонка с отступами сверху и снизу. */
  :global(#sub-vault .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .pv-text,
  .pv-lead,
  .pv-note {
    margin: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
  }
  .pv-text {
    color: var(--k-text-2);
  }
  .pv-lead {
    color: var(--k-text);
    font-weight: 600;
    font-size: var(--k-text-md);
  }
  .pv-note {
    color: var(--k-text-3);
  }
  .pv-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .pv-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: pv-step;
  }
  .pv-steps li {
    position: relative;
    padding-left: 32px;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    counter-increment: pv-step;
  }
  .pv-steps li::before {
    content: counter(pv-step);
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
  .pv-site-row {
    display: flex;
    align-items: flex-end;
    gap: var(--k-space-2);
  }
  .pv-site-row :global(.k-input) {
    flex: 1;
    min-width: 0;
  }
  .pv-logins {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .pv-login {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--k-space-2);
    padding: var(--k-space-3);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
  }
  .pv-login__title {
    color: var(--k-text);
    font: 600 var(--k-text-sm) / 1.4 var(--k-font-body);
  }
  .pv-login__user {
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / 1.4 var(--k-font-body);
  }
  .pv-login__meta {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--k-space-1);
    margin-left: auto;
  }
  .pv-badge {
    padding: 2px 8px;
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font: 600 var(--k-text-xs) / 1.4 var(--k-font-body);
  }
  .pv-badge--code {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
</style>
