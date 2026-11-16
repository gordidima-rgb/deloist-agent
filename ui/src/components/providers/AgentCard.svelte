<script module>
  import { tr } from '../../lib/i18n.js';

  // Тексты, которые есть только у отдельных агентов. Ключ — id провайдера, у остальных берётся запись по умолчанию.
  // caps — что агент умеет на стороне программы: аккаунты, ключ API, проверка обновлений, выход (адреса /api/providers/{id}/…).
  const NONE = { accounts: false, key: false, refresh: false, logout: false, signinLink: false };
  const TEXTS = {
    antigravity: {
      letter: 'G',
      subtitle: () => tr('Google: Gemini, Claude и GPT по подписке Google'),
      install: () => tr('Одна кнопка: Deloist скачает программу Google (около 85 МБ), проверит её подпись и откроет вход через Google в браузере.'),
      installed: () => tr('Программа загружена. Осталось войти через Google.'),
      unsupported: () => tr('Для этого компьютера у Google пока нет сборки.'),
      signin: () => tr('Программа открыла вход через Google в браузере. Войди там, и всё подключится само.'),
      accountKind: () => tr('Вход через Google'),
      keyHint: () => tr('Если нет подписки Google, можно войти по ключу из aistudio.google.com/apikey.'),
      disconnect: () => tr('Выйти из всех аккаунтов Google и забыть ключ. Программа останется на компьютере.'),
      caps: { accounts: true, key: true, refresh: true, logout: true, signinLink: true },
    },
  };
  const DEFAULT_TEXTS = {
    letter: '',
    install: () => tr('Одна кнопка: Deloist скачает агента и подключит его.'),
    installed: () => tr('Агент загружен. Осталось войти.'),
    unsupported: () => tr('Этот агент ставится способом, которого Deloist пока не умеет.'),
    signin: () => tr('Агент открыл окно входа в браузере. Войди там, и всё подключится само.'),
    accountKind: () => tr('Вход выполнен'),
    caps: NONE,
  };
</script>

<script>
  // Карточка агента в окне «Подключить свою ИИ»: одна для Gemini (Antigravity) и для остальных агентов реестра.
  // Состояния: не загружено → загрузка (проценты, проверка подписи, распаковка) → вход в браузере →
  // подключено (аккаунты, версия, ключ API, модели, основной, отключить) → ошибка простыми словами.
  // Блоки без данных не рисуются: нет аккаунтов — нет «Аккаунтов», нет `latest` — нет строки про обновление.
  // Данные и действия даёт web/providers.js: компонент рисует и сообщает о нажатии через onaction.
  import { trServer } from '../../lib/i18n.js';
  import Button from '../kit/Button.svelte';
  import Switch from '../kit/Switch.svelte';
  import TextInput from '../kit/TextInput.svelte';

  // logo: { svg } (логотип кита) или { img } (значок из реестра); иначе первая буква названия.
  let { provider = {}, logo = {}, onaction } = $props();

  let key = $state('');
  let filter = $state('');

  const p = $derived(provider || {});
  const text = $derived({ ...DEFAULT_TEXTS, ...(TEXTS[p.id] || {}) });
  const caps = $derived(text.caps);
  const name = $derived(p.name || p.id || '');
  const letter = $derived(text.letter || name.trim().charAt(0).toUpperCase());
  const accountLabel = $derived.by(() => {
    const a = p.account || {};
    return a.account?.email || a.email || a.label || '';
  });
  // Агенты без своего списка аккаунтов показывают одну строку «кто вошёл», если программа это знает.
  const accounts = $derived(p.accounts || (accountLabel ? [{ id: '', label: accountLabel, status: 'connected', single: true }] : []));
  const phase = $derived(
    ['installing', 'node', 'node_verify', 'npm', 'verifying', 'unpacking', 'checking', 'connecting'].includes(p.status) ? p.status : '',
  );
  const installed = $derived(Boolean(p.installed) || Boolean(p.custom));
  const state = $derived.by(() => {
    if (['installing', 'node', 'node_verify', 'npm', 'verifying', 'unpacking'].includes(phase)) return 'installing';
    if (phase === 'connecting') return 'signing-in';
    if (phase === 'checking') return 'checking';
    if (!installed) return p.error ? 'error' : 'not-installed';
    const connected = p.accounts ? p.accounts.length > 0 : p.status === 'connected';
    if (!connected) return p.error || p.status === 'error' ? 'error' : 'not-connected';
    return 'connected';
  });
  const hidden = $derived(new Set(p.hiddenModels || []));
  const free = (m) => m?.free === true || /:free$/i.test(String(m?.id || ''));
  const allModels = $derived((p.models || []).slice().sort((a, b) => Number(free(b)) - Number(free(a))));
  const models = $derived(allModels.filter((m) => !filter || `${m.name} ${m.id}`.toLowerCase().includes(filter.toLowerCase())));
  const visible = $derived(allModels.filter((m) => !hidden.has(m.id)).length);

  function progressText() {
    if (p.status === 'verifying') return tr('Проверка подписи');
    if (p.status === 'unpacking') return tr('Распаковка');
    if (p.status === 'node') return tr('Скачиваю Node.js (один раз)…');
    if (p.status === 'node_verify') return tr('Проверяю подпись…');
    if (p.status === 'npm') return tr('Ставлю агента…');
    const percent = p.progress?.percent;
    return percent == null ? tr('Скачиваю…') : tr('Скачиваю: {n} %', { n: percent });
  }

  function lost(event) {
    event.currentTarget.replaceWith(document.createTextNode(letter));
  }

  function saveKey() {
    const value = key.trim();
    if (!value) return;
    onaction?.('key-save', { apiKey: value });
    key = '';
  }
</script>

<div class="agc" data-state={state}>
  <header class="agc__head">
    <span class="agc__logo" aria-hidden="true">
      {#if logo.svg}{@html logo.svg}{:else if logo.img}<img src={logo.img} alt="" onerror={lost} />{:else}{letter}{/if}
    </span>
    <div class="agc__title">
      <h3>{name}</h3>
      {#if text.subtitle}<p>{text.subtitle()}</p>{:else if p.description}<p>{p.description}</p>{/if}
    </div>
  </header>

  {#if state === 'not-installed' || state === 'error' || state === 'not-connected'}
    <section class="agc__box agc__start">
      {#if p.error}<p class="agc__warn" role="alert" data-testid="agc-error">{trServer(p.error)}</p>{/if}
      {#if p.installError && !p.error}<p class="agc__warn" role="alert">{trServer(p.installError)}</p>{/if}
      <p class="agc__hint">{installed ? text.installed() : text.install()}</p>
      <div class="agc__actions">
        {#if p.supported === false}
          <p class="agc__hint">{text.unsupported()}</p>
        {:else if !installed}
          <Button variant="primary" onclick={() => onaction?.('install')}>{tr('Подключить')}</Button>
        {:else}
          <Button variant="primary" onclick={() => onaction?.('connect')}>{tr('Подключить')}</Button>
        {/if}
        {#if p.custom}<Button onclick={() => onaction?.('remove')}>{tr('Удалить')}</Button>{/if}
      </div>
    </section>
  {:else if state === 'installing' || state === 'checking'}
    <section class="agc__box agc__start">
      <p class="agc__hint" data-testid="agc-progress">{state === 'checking' ? tr('Проверяю программу…') : progressText()}</p>
      <div class="agc__bar"><span class:agc__bar--moving={p.progress?.percent == null} style:width={`${p.progress?.percent ?? 30}%`}></span></div>
      {#if state === 'installing'}<div class="agc__actions"><Button onclick={() => onaction?.('cancel')}>{tr('Отмена')}</Button></div>{/if}
    </section>
  {:else if state === 'signing-in'}
    <section class="agc__box agc__start">
      <p class="agc__hint" data-testid="agc-signin">{text.signin()}</p>
      {#if caps.signinLink && p.signinLink}
        <p class="agc__sub" data-testid="agc-nobrowser">{tr('Браузер не открылся? Открой страницу входа сам или скопируй ссылку.')}</p>
      {/if}
      <div class="agc__actions">
        {#if caps.signinLink && p.signinLink}
          <Button variant="primary" onclick={() => onaction?.('signin-open')}>{tr('Открыть страницу входа')}</Button>
          <Button onclick={() => onaction?.('signin-copy')}>{tr('Скопировать ссылку')}</Button>
        {/if}
        <Button onclick={() => onaction?.('cancel')}>{tr('Отмена')}</Button>
      </div>
    </section>
  {:else}
    {#if accounts.length}
      <section class="agc__box">
        <h4>{tr('Аккаунты')}</h4>
        <ul class="agc__list">
          {#each accounts as a (a.id || a.label)}
            <li class="agc__line">
              <div>
                <b>{a.label}</b>
                <span class="agc__sub">
                  {a.kind === 'key' ? tr('Вход по ключу API') : text.accountKind()}
                  {#if a.status && a.status !== 'connected'} · {a.error ? trServer(a.error) : tr('Не подключено')}{/if}
                </span>
              </div>
              {#if caps.accounts && !a.single}<Button size="sm" onclick={() => onaction?.('account-remove', { id: a.id })}>{tr('Выйти')}</Button>{/if}
            </li>
          {/each}
        </ul>
        {#if caps.accounts}
          {#if accounts.length < (p.maxAccounts || 3)}
            <div class="agc__actions"><Button onclick={() => onaction?.('account-add')}>{tr('Добавить аккаунт')}</Button></div>
          {:else}
            <p class="agc__sub agc__pad">{tr('Подключено наибольшее число аккаунтов: {n}.', { n: p.maxAccounts || 3 })}</p>
          {/if}
        {/if}
        {#if p.error}<p class="agc__warn agc__pad" role="alert" data-testid="agc-error">{trServer(p.error)}</p>{/if}
      </section>
    {:else if p.error}
      <section class="agc__box agc__start"><p class="agc__warn" role="alert" data-testid="agc-error">{trServer(p.error)}</p></section>
    {/if}

    <section class="agc__box">
      <h4>{tr('Программа')}</h4>
      <div class="agc__line">
        <div>
          {#if p.custom}
            <b>{tr('Своя команда')}</b>
            <code>{p.command || ''}</code>
          {:else if p.installed}
            <b>{p.update && p.latest ? tr('v{v} · доступна {latest}', { v: p.installed, latest: p.latest }) : tr('v{v} · загружена Deloist', { v: p.installed })}</b>
          {/if}
        </div>
        <div class="agc__row">
          {#if p.update && !p.custom}<Button variant="primary" size="sm" onclick={() => onaction?.('install')}>{tr('Обновить')}</Button>{/if}
          {#if caps.refresh}<Button size="sm" onclick={() => onaction?.('refresh')}>{tr('Проверить обновления')}</Button>{/if}
          <Button size="sm" onclick={() => onaction?.('check')}>{tr('Перезапустить')}</Button>
        </div>
      </div>
    </section>

    {#if caps.key}
      <section class="agc__box">
        <h4>{tr('Ключ API (по желанию)')}</h4>
        <p class="agc__sub agc__pad">
          {#if p.hasKey}
            {tr('Ключ сохранён (…{tail}). Вход по ключу — отдельная запись в списке аккаунтов.', { tail: p.keyTail || '' })}
          {:else}
            {text.keyHint()}
          {/if}
        </p>
        <div class="agc__keyrow">
          <TextInput type="password" bind:value={key} autocomplete="off" placeholder={p.hasKey ? tr('новый ключ') : tr('Ключ API')} aria-label={tr('Ключ API')} />
          <Button size="sm" disabled={!key.trim()} onclick={saveKey}>{tr('Сохранить ключ')}</Button>
          {#if p.hasKey}<Button size="sm" onclick={() => onaction?.('key-remove')}>{tr('Убрать ключ')}</Button>{/if}
        </div>
      </section>
    {/if}

    {#if allModels.length}
      <section class="agc__box">
        <div class="agc__line">
          <h4>{tr('Модели')}</h4>
          <span class="agc__row">
            <span class="agc__sub">{tr('{n} из {m} моделей видно в выборе модели.', { n: visible, m: allModels.length })}</span>
            <Button size="sm" onclick={() => onaction?.('models-all', { hide: visible === allModels.length })}>{visible === allModels.length ? tr('Скрыть все') : tr('Показать все')}</Button>
          </span>
        </div>
        <div class="agc__pad"><TextInput type="search" bind:value={filter} placeholder={tr('Поиск моделей')} aria-label={tr('Поиск моделей')} /></div>
        <ul class="agc__list">
          {#each models as m (m.id)}
            <li class="agc__line" data-model={m.id}>
              <div><b>{m.name}</b> <code>{m.id}</code>{#if free(m)} <span class="agc__done">{tr('Бесплатно')}</span>{/if}</div>
              <Switch checked={!hidden.has(m.id)} label={m.name} onchange={(on) => onaction?.('model', { id: m.id, on })} />
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="agc__box">
      <div class="agc__line">
        <div>
          <b>{tr('По умолчанию для новых ботов')}</b>
          <span class="agc__sub">{p.isDefault ? tr('Новые боты работают через {name}.', { name }) : tr('Новые боты будут работать через {name}.', { name })}</span>
        </div>
        {#if p.isDefault}<span class="agc__done">{tr('Основной')}</span>{:else}<Button size="sm" onclick={() => onaction?.('default')}>{tr('Сделать основным')}</Button>{/if}
      </div>
      {#if typeof p.enabled === 'boolean'}
        <div class="agc__line" data-row="enabled">
          <div>
            <b>{tr('Использовать {name}', { name })}</b>
            <span class="agc__sub">{tr('Показывать в выборе модели у ботов')}</span>
          </div>
          <Switch checked={p.enabled} label={tr('Использовать {name}', { name })} onchange={(on) => onaction?.('enabled', { on })} />
        </div>
      {/if}
      {#if caps.logout}
        <div class="agc__line">
          <div>
            <b>{tr('Отключить')}</b>
            <span class="agc__sub">{text.disconnect()}</span>
          </div>
          <Button size="sm" onclick={() => onaction?.('disconnect')}>{tr('Отключить')}</Button>
        </div>
      {:else if installed}
        <div class="agc__line">
          <div>
            <b>{tr('Удалить')}</b>
            <span class="agc__sub">{p.custom ? tr('Убрать этого агента из списка') : tr('Убрать скачанного агента с компьютера')}</span>
          </div>
          <Button size="sm" onclick={() => onaction?.('remove')}>{tr('Удалить')}</Button>
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .agc { display: flex; flex-direction: column; gap: var(--k-space-4, 16px); }
  .agc__head { display: flex; align-items: center; gap: var(--k-space-3, 12px); }
  .agc__logo :global(svg), .agc__logo img { width: 22px; height: 22px; display: block; }
  .agc__logo {
    display: grid; place-items: center; width: 40px; height: 40px; border-radius: var(--k-radius-md, 10px);
    background: var(--k-surface-3, #eee); font: 700 18px / 1 var(--k-font-body, inherit);
  }
  .agc__title h3 { margin: 0; font: 700 var(--k-text-lg, 18px) / 1.2 var(--k-font-body, inherit); }
  .agc__title p { margin: 2px 0 0; color: var(--k-text-3, #777); font-size: var(--k-text-sm, 13px); }
  .agc__box { border: 1px solid var(--k-border, #ddd); border-radius: var(--k-radius-lg, 12px); background: var(--k-surface-2, transparent); }
  .agc__box h4 { margin: 0; padding: var(--k-space-3, 12px) var(--k-space-4, 16px) var(--k-space-1, 4px); font-size: var(--k-text-sm, 13px); color: var(--k-text-3, #777); }
  .agc__line { display: flex; align-items: center; justify-content: space-between; gap: var(--k-space-4, 16px); padding: var(--k-space-3, 12px) var(--k-space-4, 16px); }
  .agc__line + .agc__line { border-top: 1px solid var(--k-border, #ddd); }
  .agc__line > div:first-child { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 2px; overflow-wrap: anywhere; }
  .agc__list { list-style: none; margin: 0; padding: 0; }
  .agc__sub { color: var(--k-text-3, #777); font-size: var(--k-text-sm, 13px); }
  .agc__pad { padding: var(--k-space-2, 8px) var(--k-space-4, 16px); margin: 0; }
  .agc__start { padding: var(--k-space-4, 16px); display: flex; flex-direction: column; gap: var(--k-space-3, 12px); }
  .agc__hint { margin: 0; color: var(--k-text-2, inherit); }
  .agc__warn { margin: 0; color: var(--k-attention-text, #b3261e); overflow-wrap: anywhere; }
  .agc__actions { display: flex; gap: var(--k-space-2, 8px); padding: var(--k-space-2, 8px) var(--k-space-4, 16px) var(--k-space-3, 12px); }
  .agc__start .agc__actions { padding: 0; }
  .agc__row { display: flex; flex-wrap: wrap; gap: var(--k-space-2, 8px); justify-content: flex-end; }
  .agc__keyrow { display: flex; gap: var(--k-space-2, 8px); padding: var(--k-space-2, 8px) var(--k-space-4, 16px) var(--k-space-3, 12px); align-items: center; }
  .agc__keyrow :global(.k-input) { flex: 1; min-width: 0; }
  .agc__done { color: var(--k-brand-text, inherit); font-weight: 600; }
  .agc__bar { height: 6px; border-radius: 3px; background: var(--k-surface-3, #eee); overflow: hidden; }
  .agc__bar span { display: block; height: 100%; background: var(--k-brand, #3a6); transition: width 0.2s; }
  .agc__bar--moving { animation: agc-move 1.2s ease-in-out infinite alternate; }
  @keyframes agc-move { from { margin-left: 0; } to { margin-left: 70%; } }
  code { color: var(--k-text-3, #777); font-size: 12px; }
</style>
