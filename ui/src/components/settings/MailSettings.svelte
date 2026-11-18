<script>
  // Экран «Почта» окна настроек: ящики ботов (IMAP/SMTP), доступ и автоматическая работа,
  // почтовые триггеры рутин и журнал отправленного. Пароль приложения живёт только в поле ввода
  // и в запросе: обратно сервер его не отдаёт.
  import { untrack } from 'svelte';
  import { Section, Field, TextInput, TextArea, Select, Button, Checkbox, Switch, Badge, EmptyState } from '../kit';
  import { toast } from '../../lib/toast.js';
  import { tr } from '../../lib/i18n.js';

  let { api, bots = [], routines = [], onchanged } = $props();

  const SENT_SHOWN = 20;
  const SECURITY = [
    { value: 'tls', label: 'TLS' },
    { value: 'starttls', label: 'STARTTLS' },
  ];

  let data = $state(null);
  let sent = $state([]);
  let loading = $state(true);
  let error = $state('');

  // Форма добавления ящика.
  let presetId = $state('');
  let addr = $state('');
  let login = $state('');
  let password = $state('');
  let displayName = $state('');
  let imapHost = $state('');
  let imapPort = $state('993');
  let imapSec = $state('tls');
  let smtpHost = $state('');
  let smtpPort = $state('465');
  let smtpSec = $state('tls');
  let adding = $state(false);
  let addError = $state('');

  // Редактирование ящика: открыта одна карточка за раз.
  let editId = $state('');
  let draft = $state({ bots: [], autoBots: [], allow: '', trusted: '', authserv: '', password: '' });
  let saving = $state(false);
  let editError = $state('');
  let busyId = $state('');

  // Форма триггера.
  let trRoutine = $state('');
  let trAccount = $state('');
  let trFolder = $state('INBOX');
  let trFrom = $state('');
  let trSubject = $state('');
  let trSaving = $state(false);
  let trError = $state('');

  const accounts = $derived(data?.accounts ?? []);
  const presets = $derived(data?.presets ?? []);
  const triggers = $derived(data?.triggers ?? []);
  const isCustom = $derived(presetId === '' || presetId === 'custom');
  const preset = $derived(presets.find((p) => p.id === presetId) || null);
  const presetOptions = $derived([
    ...presets.map((p) => ({ value: p.id, label: p.label })),
    { value: 'custom', label: tr('Другой сервер') },
  ]);
  const eligible = $derived(routines.filter((r) => !r.hook && r.kind !== 'reminder'));
  const routineOptions = $derived(eligible.map((r) => ({ value: r.id, label: r.title })));
  const trRoutineObj = $derived(eligible.find((r) => r.id === trRoutine) || null);
  const trAccounts = $derived(
    trRoutineObj ? accounts.filter((a) => (a.bots || []).includes(trRoutineObj.botId)) : [],
  );
  const trAccountOptions = $derived(trAccounts.map((a) => ({ value: a.id, label: a.address })));
  const activeTriggers = $derived(
    triggers
      .map((t) => ({
        ...t,
        routine: routines.find((r) => r.id === t.routineId) || null,
        account: accounts.find((a) => a.id === t.accountId) || null,
      }))
      .filter((t) => t.routine),
  );

  $effect(() => {
    if (presets.length && !presetId) presetId = presets[0].id;
  });
  $effect(() => {
    if (eligible.length && !eligible.some((r) => r.id === trRoutine)) trRoutine = eligible[0].id;
  });
  $effect(() => {
    if (!trAccounts.some((a) => a.id === trAccount)) trAccount = trAccounts[0]?.id || '';
  });

  const botName = (id) => (bots.find((b) => b.id === id) || {}).name || id;
  const errText = (err) => err?.message || tr('Не удалось выполнить запрос');
  const lines = (text) => [...new Set(String(text || '').split(/[\s,;]+/).map((s) => s.trim()).filter(Boolean))];
  const same = (a, b) => JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());

  async function load() {
    try {
      data = await api('/api/mail');
      error = '';
    } catch (err) {
      error = errText(err);
    } finally {
      loading = false;
    }
    try {
      const res = await api('/api/mail/sent');
      sent = Array.isArray(res?.sent) ? res.sent : [];
    } catch {
      sent = [];
    }
  }

  async function refreshAccounts() {
    try {
      data = await api('/api/mail');
    } catch (err) {
      error = errText(err);
    }
  }

  async function addAccount() {
    if (adding) return;
    const address = addr.trim();
    if (!address || !password) return;
    adding = true;
    addError = '';
    const body = { address, password };
    if (login.trim()) body.login = login.trim();
    if (displayName.trim()) body.displayName = displayName.trim();
    if (isCustom) {
      Object.assign(body, {
        preset: 'custom',
        imapHost: imapHost.trim(),
        imapPort: Number(imapPort) || 993,
        imapTls: imapSec,
        smtpHost: smtpHost.trim(),
        smtpPort: Number(smtpPort) || 465,
        smtpTls: smtpSec,
      });
    } else {
      body.preset = presetId;
    }
    try {
      await api('/api/mail/accounts', { method: 'POST', body });
      addr = '';
      login = '';
      password = '';
      displayName = '';
      toast(tr('Ящик подключён'));
      await refreshAccounts();
      onchanged?.();
    } catch (err) {
      addError = errText(err);
    } finally {
      adding = false;
    }
  }

  async function check(account) {
    if (busyId) return;
    busyId = account.id;
    try {
      await api(`/api/mail/accounts/${encodeURIComponent(account.id)}/check`, { method: 'POST' });
      toast(tr('Ящик {address} отвечает', { address: account.address }));
    } catch (err) {
      toast(errText(err));
    } finally {
      busyId = '';
      await refreshAccounts();
    }
  }

  async function remove(account) {
    if (busyId) return;
    if (!confirm(tr('Удалить ящик {address}? Боты потеряют к нему доступ.', { address: account.address }))) return;
    busyId = account.id;
    try {
      await api(`/api/mail/accounts/${encodeURIComponent(account.id)}`, { method: 'DELETE' });
      if (editId === account.id) editId = '';
      await refreshAccounts();
      onchanged?.();
    } catch (err) {
      toast(errText(err));
    } finally {
      busyId = '';
    }
  }

  function openEdit(account) {
    if (editId === account.id) {
      editId = '';
      return;
    }
    editId = account.id;
    editError = '';
    draft = {
      bots: [...(account.bots || [])],
      autoBots: [...(account.autoBots || [])],
      allow: (account.allow || []).join('\n'),
      trusted: (account.trusted || []).join('\n'),
      authserv: (account.authserv || []).join('\n'),
      password: '',
    };
  }

  function toggleBot(id, on) {
    draft.bots = on ? [...new Set([...draft.bots, id])] : draft.bots.filter((x) => x !== id);
    // Авторабота возможна только у бота с доступом.
    if (!on) draft.autoBots = draft.autoBots.filter((x) => x !== id);
  }

  function toggleAuto(id, on) {
    draft.autoBots = on ? [...new Set([...draft.autoBots, id])] : draft.autoBots.filter((x) => x !== id);
  }

  async function save(account) {
    if (saving) return;
    const body = {};
    if (!same(draft.bots, account.bots || [])) body.bots = draft.bots;
    if (!same(draft.autoBots, account.autoBots || [])) body.autoBots = draft.autoBots;
    const allow = lines(draft.allow);
    if (!same(allow, account.allow || [])) body.allow = allow;
    const trusted = lines(draft.trusted);
    if (!same(trusted, account.trusted || [])) body.trusted = trusted;
    const authserv = lines(draft.authserv);
    if (!same(authserv, account.authserv || [])) body.authserv = authserv;
    if (draft.password) body.password = draft.password;
    if (!Object.keys(body).length) {
      editId = '';
      return;
    }
    saving = true;
    editError = '';
    try {
      await api(`/api/mail/accounts/${encodeURIComponent(account.id)}`, { method: 'PATCH', body });
      toast(tr('Сохранено'));
      editId = '';
      await refreshAccounts();
      onchanged?.();
    } catch (err) {
      editError = errText(err);
    } finally {
      saving = false;
    }
  }

  async function saveTrigger() {
    if (trSaving || !trRoutine || !trAccount) return;
    trSaving = true;
    trError = '';
    try {
      await api(`/api/mail/triggers/${encodeURIComponent(trRoutine)}`, {
        method: 'PUT',
        body: { accountId: trAccount, folder: trFolder.trim() || 'INBOX', from: trFrom.trim(), subject: trSubject.trim() },
      });
      trFrom = '';
      trSubject = '';
      toast(tr('Триггер сохранён'));
      await refreshAccounts();
    } catch (err) {
      trError = errText(err);
    } finally {
      trSaving = false;
    }
  }

  async function removeTrigger(routineId) {
    try {
      await api(`/api/mail/triggers/${encodeURIComponent(routineId)}`, { method: 'DELETE' });
      await refreshAccounts();
    } catch (err) {
      toast(errText(err));
    }
  }

  function filtersText(t) {
    const parts = [];
    if (t.from) parts.push(tr('от: {value}', { value: t.from }));
    if (t.subject) parts.push(tr('тема: {value}', { value: t.subject }));
    return parts.join(', ') || tr('все новые письма');
  }

  function when(ms) {
    const n = Number(ms);
    if (!Number.isFinite(n) || n <= 0) return '';
    return new Date(n).toLocaleString();
  }

  $effect(() => {
    untrack(() => {
      load();
    });
  });
</script>

{#if loading && !data}
  <Section title={tr('Почта для ботов')}>
    <p class="ml-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !data}
  <Section title={tr('Почта для ботов')}>
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="ml-actions"><Button onclick={load}>{tr('Повторить')}</Button></div>
  </Section>
{:else}
  <Section
    title={tr('Почтовые ящики')}
    description={tr('Бот читает и пишет письма от вашего имени. Пароль хранится только на этом компьютере.')}
  >
    {#if !accounts.length}
      <p class="ml-note">{tr('Ящиков пока нет. Добавьте первый ниже.')}</p>
    {/if}
    {#each accounts as account (account.id)}
      <div class="ml-card" data-account={account.id}>
        <div class="ml-card-head">
          <div class="ml-card-title">
            <span class="ml-lead">{account.label || account.address}</span>
            {#if account.label && account.label !== account.address}
              <span class="ml-note">{account.address}</span>
            {/if}
          </div>
          <div class="ml-badges">
            {#if !account.hasPassword}<Badge tone="attention" text={tr('нет пароля')} />{/if}
          </div>
        </div>
        {#if account.lastError}
          <p class="k-hint is-error">{account.lastError}</p>
        {/if}
        <div class="ml-actions">
          <Button loading={busyId === account.id} onclick={() => check(account)}>{tr('Проверить')}</Button>
          <Button onclick={() => openEdit(account)}>
            {editId === account.id ? tr('Свернуть') : tr('Настроить')}
          </Button>
          <Button variant="attention" disabled={busyId === account.id} onclick={() => remove(account)}>
            {tr('Удалить')}
          </Button>
        </div>

        {#if editId === account.id}
          <div class="ml-edit">
            <div class="ml-group">
              <span class="ml-group-title">{tr('Боты с доступом')}</span>
              {#if !bots.length}
                <p class="ml-note">{tr('Сначала создайте хотя бы одного бота')}</p>
              {/if}
              {#each bots as bot (bot.id)}
                <Checkbox
                  label={bot.name}
                  checked={draft.bots.includes(bot.id)}
                  onchange={(on) => toggleBot(bot.id, on)}
                />
              {/each}
            </div>

            {#if draft.bots.length}
              <div class="ml-group">
                <span class="ml-group-title">{tr('Автоматическая работа')}</span>
                <p class="ml-note">
                  {tr('Выключено: каждое письмо бот отправляет только после вашего согласия. Включено: без вопроса — только адресатам из белого списка и доверенным отправителям, чьё подписанное письмо бот прочитал в этом же запуске; новому адресату — всё равно карточка. Не больше 20 писем в час.')}
                </p>
                {#each draft.bots as id (id)}
                  <Switch
                    label={botName(id)}
                    checked={draft.autoBots.includes(id)}
                    onchange={(on) => toggleAuto(id, on)}
                  />
                {/each}
              </div>
            {/if}

            <Field
              label={tr('Белый список')}
              hint={tr('Адреса или целые домены вида @domain.ru, по одному в строке.')}
            >
              {#snippet children(f)}
                <TextArea id={f.id} describedby={f.describedby} rows={3} bind:value={draft.allow} />
              {/snippet}
            </Field>
            <Field
              label={tr('Доверенные отправители')}
              hint={tr('Письмо от них бот выполняет как поручение, только если почтовый сервер подтвердил адрес подписью (DKIM или DMARC). Остальные письма — просто данные. Остаток риска: если подтверждается только домен, любой ящик этого домена может писать от имени доверенного адреса. Доверие работает, только если почтовый сервер сам добавляет свой заголовок Authentication-Results при доставке.')}
            >
              {#snippet children(f)}
                <TextArea id={f.id} describedby={f.describedby} rows={3} bind:value={draft.trusted} />
              {/snippet}
            </Field>
            <Field
              label={tr('Серверы, которым верим')}
              hint={tr('Имена почтовых серверов получателя (например yandex.net), по одному в строке. Подпись письма засчитывается, только если её записал такой сервер. У Яндекса, Mail и Gmail заполнено само; у своего сервера пусто — письма не считаются подписанными, пока вы не впишете имя из заголовка Authentication-Results.')}
            >
              {#snippet children(f)}
                <TextArea id={f.id} describedby={f.describedby} rows={2} bind:value={draft.authserv} />
              {/snippet}
            </Field>
            <Field label={tr('Новый пароль приложения')} hint={tr('Оставьте пустым, чтобы не менять.')}>
              {#snippet children(f)}
                <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={draft.password} />
              {/snippet}
            </Field>
            {#if editError}
              <p class="k-hint is-error" role="alert">{editError}</p>
            {/if}
            <div class="ml-actions">
              <Button variant="primary" loading={saving} onclick={() => save(account)}>{tr('Сохранить')}</Button>
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </Section>

  <Section
    title={tr('Добавить ящик')}
    description={tr('Обычный пароль от почты не подойдёт: нужен пароль приложения, который почтовый сервис создаёт отдельно.')}
  >
    <Field label={tr('Почтовый сервис')}>
      {#snippet children(f)}
        <Select id={f.id} options={presetOptions} bind:value={presetId} />
      {/snippet}
    </Field>
    {#if preset?.help && !isCustom}
      <p class="ml-note">
        <a class="ml-link" href={preset.help} target="_blank" rel="noopener noreferrer">{tr('Создать пароль приложения')}</a>
      </p>
    {/if}
    <Field label={tr('Адрес почты')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="email" autocomplete="off" spellcheck="false" bind:value={addr} />
      {/snippet}
    </Field>
    <Field label={tr('Логин')} hint={tr('Необязательно: по умолчанию совпадает с адресом.')}>
      {#snippet children(f)}
        <TextInput id={f.id} describedby={f.describedby} autocomplete="off" spellcheck="false" bind:value={login} />
      {/snippet}
    </Field>
    <Field label={tr('Пароль приложения')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="password" autocomplete="off" spellcheck="false" bind:value={password} />
      {/snippet}
    </Field>
    <Field label={tr('Имя отправителя')} hint={tr('Необязательно: так подпишутся письма.')}>
      {#snippet children(f)}
        <TextInput id={f.id} describedby={f.describedby} bind:value={displayName} />
      {/snippet}
    </Field>
    {#if isCustom}
      <Field label={tr('IMAP: сервер')}>
        {#snippet children(f)}
          <TextInput id={f.id} placeholder="imap.example.com" spellcheck="false" bind:value={imapHost} />
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
          <TextInput id={f.id} placeholder="smtp.example.com" spellcheck="false" bind:value={smtpHost} />
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
    {/if}
    {#if adding}
      <p class="ml-note">{tr('Проверяю вход на сервере, это может занять до минуты…')}</p>
    {/if}
    {#if addError}
      <p class="k-hint is-error" role="alert">{addError}</p>
    {/if}
    <div class="ml-actions">
      <Button variant="primary" loading={adding} disabled={!addr.trim() || !password} onclick={addAccount}>
        {tr('Подключить ящик')}
      </Button>
    </div>
  </Section>

  <Section
    title={tr('Письма будят рутину')}
    description={tr('Рутина по своему расписанию смотрит ящик и будит бота, только если пришли новые подходящие письма. Без писем модель не вызывается. Старые письма не обрабатываются, каждое — один раз.')}
  >
    {#each activeTriggers as t (t.routineId)}
      <div class="ml-card" data-trigger={t.routineId}>
        <div class="ml-card-head">
          <div class="ml-card-title">
            <span class="ml-lead">{t.routine.title}</span>
            <span class="ml-note">
              {t.account?.address || t.accountId} · {t.folder || 'INBOX'} · {filtersText(t)}
            </span>
          </div>
        </div>
        {#if t.lastError}<p class="k-hint is-error">{t.lastError}</p>{/if}
        <div class="ml-actions">
          <Button variant="attention" onclick={() => removeTrigger(t.routineId)}>{tr('Убрать')}</Button>
        </div>
      </div>
    {/each}
    {#if !eligible.length}
      <p class="ml-note">{tr('Нет рутин, которые можно привязать к почте. Создайте рутину по расписанию.')}</p>
    {:else}
      <Field label={tr('Рутина')}>
        {#snippet children(f)}
          <Select id={f.id} options={routineOptions} bind:value={trRoutine} />
        {/snippet}
      </Field>
      {#if trAccounts.length}
        <Field label={tr('Ящик')}>
          {#snippet children(f)}
            <Select id={f.id} options={trAccountOptions} bind:value={trAccount} />
          {/snippet}
        </Field>
        <Field label={tr('Папка')}>
          {#snippet children(f)}
            <TextInput id={f.id} bind:value={trFolder} />
          {/snippet}
        </Field>
        <Field label={tr('От кого содержит')}>
          {#snippet children(f)}
            <TextInput id={f.id} spellcheck="false" bind:value={trFrom} />
          {/snippet}
        </Field>
        <Field label={tr('Тема содержит')}>
          {#snippet children(f)}
            <TextInput id={f.id} bind:value={trSubject} />
          {/snippet}
        </Field>
        {#if trError}<p class="k-hint is-error" role="alert">{trError}</p>{/if}
        <div class="ml-actions">
          <Button variant="primary" loading={trSaving} onclick={saveTrigger}>{tr('Сохранить триггер')}</Button>
        </div>
      {:else}
        <p class="k-hint is-error">{tr('Сначала откройте ящик боту этой рутины')}</p>
      {/if}
    {/if}
  </Section>

  <Section title={tr('Отправленные письма')}>
    {#if sent.length}
      <ul class="ml-sent">
        {#each sent.slice(0, SENT_SHOWN) as row, i (`${row.at}-${i}`)}
          <li class="ml-sent-row">
            <span class="ml-lead">{row.subject || tr('(без темы)')}</span>
            <span class="ml-note">
              {when(row.at)} · {row.bot} → {row.label} · {row.how === 'auto' ? tr('авто') : tr('по согласию')}
            </span>
          </li>
        {/each}
      </ul>
    {:else}
      <EmptyState title={tr('Бот пока ничего не отправлял')} />
    {/if}
  </Section>
{/if}

<style>
  :global(#sub-mail .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .ml-lead {
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .ml-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .ml-link {
    color: var(--k-brand-text);
  }
  .ml-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .ml-card {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
  .ml-card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--k-space-3);
  }
  .ml-card-title {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-1);
    min-width: 0;
  }
  .ml-badges {
    display: flex;
    gap: var(--k-space-2);
  }
  .ml-edit {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-3);
    border-top: 1px solid var(--k-line);
  }
  .ml-group {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
  }
  .ml-group-title {
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
  }
  .ml-sent {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .ml-sent-row {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-1);
  }
</style>
