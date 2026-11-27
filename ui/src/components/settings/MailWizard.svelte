<script>
  // Мастер «Подключить почту»: человек вводит адрес, служба определяется по домену, дальше шаги с кнопками на нужные
  // страницы и пароль приложения; для Яндекса — вход одной кнопкой (OAuth), если она включена в сборке.
  // Серверы и порты спрятаны в «Дополнительно». Пароль живёт только в поле и в запросе, обратно сервер его не отдаёт.
  import { Field, TextInput, Select, Button } from '../kit';
  import { toast } from '../../lib/toast.js';
  import { tr } from '../../lib/i18n.js';

  let { api, presets = [], yandexOauth = false, pollMs = 1500, onadded } = $props();

  const SECURITY = [
    { value: 'tls', label: 'TLS' },
    { value: 'starttls', label: 'STARTTLS' },
  ];
  // Шаги до ввода пароля. `link` — кнопка открывает страницу `preset.help`.
  const mailSteps = () => [
    {
      title: tr('Откройте настройки паролей'),
      text: tr('Настройки → Все настройки → Безопасность → «Пароли для внешних приложений». Нужен привязанный телефон.'),
      link: tr('Открыть справку Mail'),
    },
    { title: tr('Создайте пароль'), text: tr('Нажмите «Добавить», выберите «Полный доступ к Почте» и скопируйте пароль: он виден один раз.') },
  ];
  const stepsFor = (id) => {
    switch (id) {
      case 'yandex':
        return [
          {
            title: tr('Включите доступ по IMAP'),
            text: tr('В Яндекс Почте: Настройки → Почтовые программы → разрешите доступ с помощью почтовых клиентов и отметьте «Пароли приложений и OAuth-токены».'),
          },
          { title: tr('Создайте пароль приложения'), text: tr('Выберите тип «Почта» и скопируйте пароль: он виден один раз.'), link: tr('Открыть страницу паролей') },
        ];
      case 'mailru':
      case 'vk':
        return mailSteps();
      case 'rambler':
        return [
          {
            title: tr('Разрешите почтовые программы'),
            text: tr('В настройках почты откройте «Безопасность» и включите доступ для почтовых клиентов.'),
            link: tr('Открыть справку Рамблера'),
          },
          { title: tr('Создайте пароль'), text: tr('Там же создайте специальный пароль для почтовой программы и скопируйте его.') },
        ];
      case 'gmail':
        return [
          { title: tr('Включите двухэтапную защиту'), text: tr('Пароли приложений Google доступны, когда в аккаунте включена двухэтапная аутентификация.') },
          { title: tr('Создайте пароль приложения'), text: tr('Назовите его «Deloist» и скопируйте пароль.'), link: tr('Открыть страницу паролей') },
        ];
      default:
        return [];
    }
  };
  const errText = (err) => (err?.message ? tr(err.message) : tr('Не удалось выполнить запрос'));

  let addr = $state('');
  let chosen = $state('');
  let password = $state('');
  let displayName = $state('');
  let login = $state('');
  let imapHost = $state('');
  let imapPort = $state('993');
  let imapSec = $state('tls');
  let smtpHost = $state('');
  let smtpPort = $state('465');
  let smtpSec = $state('tls');
  let advanced = $state(false);
  let adding = $state(false);
  let error = $state('');
  let signing = $state(false);
  let alive = true;

  const domain = $derived(addr.trim().toLowerCase().split('@')[1] || '');
  const detected = $derived(presets.find((p) => (p.domains || []).includes(domain)) || null);
  const presetId = $derived(chosen || detected?.id || '');
  const preset = $derived(presets.find((p) => p.id === presetId) || null);
  const isCustom = $derived(presetId === 'custom');
  const askService = $derived(addr.includes('@') && domain.includes('.') && !detected);
  const steps = $derived(stepsFor(presetId));
  const serviceOptions = $derived([
    ...presets.map((p) => ({ value: p.id, label: p.label })),
    { value: 'custom', label: tr('Другой сервер') },
  ]);
  const ready = $derived(presetId !== '' && addr.trim() && password && !adding);

  $effect(() => () => {
    alive = false;
  });

  function open(url) {
    window.open(url, '_blank', 'noopener');
  }

  async function connect() {
    if (!ready) return;
    adding = true;
    error = '';
    const body = { address: addr.trim(), password, preset: presetId };
    if (login.trim()) body.login = login.trim();
    if (displayName.trim()) body.displayName = displayName.trim();
    if (isCustom || imapHost.trim()) Object.assign(body, { imapHost: imapHost.trim(), imapPort: Number(imapPort) || 993, imapTls: imapSec });
    if (isCustom || smtpHost.trim()) Object.assign(body, { smtpHost: smtpHost.trim(), smtpPort: Number(smtpPort) || 465, smtpTls: smtpSec });
    try {
      await api('/api/mail/accounts', { method: 'POST', body });
      addr = '';
      login = '';
      password = '';
      displayName = '';
      chosen = '';
      toast(tr('Ящик подключён'));
      onadded?.();
    } catch (err) {
      error = errText(err);
    } finally {
      adding = false;
    }
  }

  // Вход через Яндекс: браузер открывает сервер, мы ждём возврата и завершаем подключение.
  async function yandexSignIn() {
    if (signing) return;
    signing = true;
    error = '';
    try {
      const { flow } = await api('/api/mail/oauth/yandex/start', { method: 'POST' });
      for (;;) {
        await new Promise((resolve) => setTimeout(resolve, pollMs));
        if (!alive) return;
        const state = await api(`/api/mail/oauth/yandex/${encodeURIComponent(flow)}`);
        if (state.status === 'failed') throw new Error(state.error);
        if (state.status === 'ready') break;
      }
      const body = displayName.trim() ? { displayName: displayName.trim() } : {};
      await api(`/api/mail/oauth/yandex/${encodeURIComponent(flow)}/finish`, { method: 'POST', body });
      addr = '';
      displayName = '';
      toast(tr('Ящик подключён'));
      onadded?.();
    } catch (err) {
      error = errText(err);
    } finally {
      signing = false;
    }
  }
</script>

<Field label={tr('Адрес почты')}>
  {#snippet children(f)}
    <TextInput id={f.id} type="email" autocomplete="off" spellcheck="false" bind:value={addr} />
  {/snippet}
</Field>

{#if askService}
  <Field label={tr('Почтовый сервис')} hint={tr('Службу по адресу определить не вышло: выберите её.')}>
    {#snippet children(f)}
      <Select id={f.id} options={[{ value: '', label: '—' }, ...serviceOptions]} bind:value={chosen} />
    {/snippet}
  </Field>
{:else if detected}
  <p class="mw-note" data-service={detected.id}>{tr('Почтовый сервис')}: {detected.label}</p>
{/if}

{#if presetId === 'yandex' && yandexOauth}
  <div class="mw-actions">
    <Button variant="primary" loading={signing} onclick={yandexSignIn}>{tr('Войти через Яндекс')}</Button>
  </div>
  <p class="mw-note">{tr('Откроется браузер: войдите в Яндекс и разрешите доступ к почте. Пароль приложения не нужен.')}</p>
  <p class="mw-note">{tr('Или подключите по паролю приложения:')}</p>
{/if}

{#if steps.length}
  <ol class="mw-steps">
    {#each steps as step, i (step.title)}
      <li class="mw-step" data-step={i + 1}>
        <span class="mw-num">{i + 1}</span>
        <div class="mw-body">
          <span class="mw-title">{step.title}</span>
          <span class="mw-note">{step.text}</span>
          {#if step.link && preset?.help}
            <div class="mw-actions"><Button size="sm" onclick={() => open(preset.help)}>{step.link}</Button></div>
          {/if}
        </div>
      </li>
    {/each}
  </ol>
{/if}

{#if presetId}
  <Field label={steps.length ? tr('Шаг {n}: вставьте пароль приложения', { n: steps.length + 1 }) : tr('Пароль приложения')}>
    {#snippet children(f)}
      <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={password} />
    {/snippet}
  </Field>
  <Field label={tr('Имя отправителя')} hint={tr('Необязательно: так подпишутся письма.')}>
    {#snippet children(f)}
      <TextInput id={f.id} describedby={f.describedby} bind:value={displayName} />
    {/snippet}
  </Field>

  <div class="mw-actions">
    <Button variant="ghost" size="sm" aria-expanded={advanced || isCustom} onclick={() => (advanced = !advanced)}>
      {tr('Дополнительно')}
    </Button>
  </div>
  {#if advanced || isCustom}
    <div class="mw-advanced" data-advanced>
      <Field label={tr('Логин')} hint={tr('Необязательно: по умолчанию совпадает с адресом.')}>
        {#snippet children(f)}
          <TextInput id={f.id} describedby={f.describedby} autocomplete="off" spellcheck="false" bind:value={login} />
        {/snippet}
      </Field>
      <Field label={tr('IMAP: сервер')}>
        {#snippet children(f)}
          <TextInput id={f.id} placeholder={preset?.imapHost || 'imap.example.com'} spellcheck="false" bind:value={imapHost} />
        {/snippet}
      </Field>
      <Field label={tr('IMAP: порт')}>
        {#snippet children(f)}
          <TextInput id={f.id} inputmode="numeric" bind:value={imapPort} />
        {/snippet}
      </Field>
      <Field label={tr('IMAP: защита')}>
        {#snippet children(f)}
          <Select id={f.id} options={SECURITY} bind:value={imapSec} />
        {/snippet}
      </Field>
      <Field label={tr('SMTP: сервер')}>
        {#snippet children(f)}
          <TextInput id={f.id} placeholder={preset?.smtpHost || 'smtp.example.com'} spellcheck="false" bind:value={smtpHost} />
        {/snippet}
      </Field>
      <Field label={tr('SMTP: порт')}>
        {#snippet children(f)}
          <TextInput id={f.id} inputmode="numeric" bind:value={smtpPort} />
        {/snippet}
      </Field>
      <Field label={tr('SMTP: защита')}>
        {#snippet children(f)}
          <Select id={f.id} options={SECURITY} bind:value={smtpSec} />
        {/snippet}
      </Field>
    </div>
  {/if}
{/if}

{#if adding}
  <p class="mw-note">{tr('Проверяю вход на сервере, это может занять до минуты…')}</p>
{/if}
{#if signing}
  <p class="mw-note">{tr('Жду входа в браузере…')}</p>
{/if}
{#if error}
  <p class="k-hint is-error" role="alert">{error}</p>
{/if}
<div class="mw-actions">
  <Button variant={presetId === 'yandex' && yandexOauth ? 'secondary' : 'primary'} loading={adding} disabled={!ready} onclick={connect}>
    {tr('Подключить')}
  </Button>
</div>

<style>
  .mw-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .mw-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .mw-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .mw-step {
    display: flex;
    gap: var(--k-space-3);
    padding: var(--k-space-3);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .mw-num {
    flex: none;
    color: var(--k-brand-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .mw-body {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    min-width: 0;
  }
  .mw-title {
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
  }
  .mw-advanced {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
  }
</style>
