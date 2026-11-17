<script>
  // Экран «Устройства»: связь этого ПК с deloist.ru, доверенные устройства (отзыв и права на действия),
  // и «Другие ПК» — знакомство с другим ПК аккаунта, чьих ботов видно в боковой панели.
  // Имена и всё прочее с устройств и других ПК показываются только текстом.
  import { onMount } from 'svelte';
import { Section, Row, Field, TextInput, Button, Switch } from '../kit';
  import { tr, trServer } from '../../lib/i18n.js';
  import { enc, groupCode, digitsOnly, kindLabel, agoText, errorText, linkText } from '../../lib/remote.js';

  let { api } = $props();

  let info = $state(null);
  let remote = $state({ connected: false, pcs: [], candidates: [], pairing: null });
  let loadError = $state('');
  let notice = $state('');

  // Что сейчас спрашивается: отзыв, включение действий, отключение ПК.
  let revoking = $state('');
  let actionsAsk = $state('');
  let actionsName = $state('');
  let actionsError = $state('');
  let switchKey = $state({});
  let forgetting = $state('');
  let busy = $state('');

  // Знакомство с другим ПК.
  let pair = $state(null); // { req, code, pc, name }
  let ackCode = $state('');
  let pairError = $state('');
  let pairBusy = $state(false);

  async function loadDevices() {
    try {
      info = await api('/api/devices');
      loadError = '';
    } catch (err) {
      loadError = errorText(err);
    }
  }

  async function loadRemote() {
    try {
      remote = await api('/api/remote/pcs');
    } catch {
      /* связи нет: список остаётся прежним */
    }
  }

  // Режим сервера: старый ноутбук как всегда включённый сервер (автозапуск без окна и запрет сна).
  let server = $state({ supported: false, enabled: false, awake: false });
  let serverKey = $state(0);
  let serverError = $state('');

  async function loadServer() {
    try {
      server = await api('/api/server-mode');
    } catch {
      /* адреса нет: раздел остаётся скрытым */
    }
  }

  async function setServer(enabled) {
    busy = 'server';
    serverError = '';
    try {
      server = await api('/api/server-mode', { method: 'PUT', body: { enabled } });
    } catch (err) {
      serverError = trServer(errorText(err));
      serverKey += 1;
    } finally {
      busy = '';
    }
  }

  const loadAll = () => Promise.all([loadDevices(), loadRemote(), loadServer()]);

  function onEvent(event) {
    if (event.detail?.type === 'devices_changed') loadAll();
  }

  onMount(() => {
    loadAll();
    window.addEventListener('deloist:event', onEvent);
    return () => window.removeEventListener('deloist:event', onEvent);
  });

  // Пока идёт знакомство, спрашиваем у ПК, пришло ли приветствие другого ПК.
  $effect(() => {
    if (!remote.pairing && !pair) return;
    const timer = setInterval(loadRemote, 3000);
    return () => clearInterval(timer);
  });

  const changed = () => window.dispatchEvent(new CustomEvent('deloist:remote-changed'));

  async function revoke(device) {
    busy = device.id;
    try {
      await api(`/api/devices/${enc(device.id)}`, { method: 'DELETE' });
      revoking = '';
      notice = tr('Устройство «{name}» отозвано', { name: device.name });
      await loadDevices();
    } catch (err) {
      notice = trServer(errorText(err));
    } finally {
      busy = '';
    }
  }

  function askActions(device) {
    actionsAsk = device.id;
    actionsName = '';
    actionsError = '';
  }

  function cancelActions(device) {
    actionsAsk = '';
    switchKey = { ...switchKey, [device.id]: (switchKey[device.id] ?? 0) + 1 };
  }

  async function setActions(device, enabled) {
    busy = device.id;
    actionsError = '';
    try {
      await api(`/api/devices/${enc(device.id)}/actions`, {
        method: 'POST',
        body: enabled ? { enabled: true, confirm: actionsName } : { enabled: false },
      });
      actionsAsk = '';
      await loadDevices();
    } catch (err) {
      actionsError = trServer(errorText(err));
      if (!enabled) switchKey = { ...switchKey, [device.id]: (switchKey[device.id] ?? 0) + 1 };
    } finally {
      busy = '';
    }
  }

  async function startPair(candidate) {
    pairBusy = true;
    pairError = '';
    try {
      const started = await api('/api/remote/pair/start', { method: 'POST', body: { pc: candidate.id } });
      pair = { req: String(started.req), code: String(started.code), pc: candidate.id, name: String(started.name ?? candidate.name) };
      ackCode = '';
      await loadRemote();
    } catch (err) {
      pairError = trServer(errorText(err));
    } finally {
      pairBusy = false;
    }
  }

  async function confirmPair() {
    const req = pair?.req || remote.pairing?.req;
    if (!req) return;
    pairBusy = true;
    pairError = '';
    try {
      await api(`/api/remote/pair/${enc(req)}/ack`, { method: 'POST', body: { code: ackCode } });
      notice = tr('ПК «{name}» подключён', { name: pair?.name || remote.pairing?.name || '' });
      pair = null;
      ackCode = '';
      await loadRemote();
      changed();
    } catch (err) {
      pairError = trServer(errorText(err));
      if (err?.status === 403) pair = null;
    } finally {
      pairBusy = false;
    }
  }

  async function forget(pc) {
    busy = pc.id;
    try {
      await api(`/api/remote/${enc(pc.id)}`, { method: 'DELETE' });
      forgetting = '';
      await loadRemote();
      changed();
    } catch (err) {
      notice = trServer(errorText(err));
    } finally {
      busy = '';
    }
  }

  const devices = $derived(Array.isArray(info?.devices) ? info.devices : []);
  const shownPair = $derived(pair ?? (remote.pairing ? { req: remote.pairing.req, code: String(remote.pairing.code ?? ''), pc: remote.pairing.pc, name: String(remote.pairing.name ?? '') } : null));
  const nameOf = (d) => d.name || tr('Без имени');
</script>

<div class="devices">
  {#if loadError}<p class="note is-error" role="alert">{loadError}</p>{/if}
  {#if notice}<p class="note" role="status">{notice}</p>{/if}

  <Section title={tr('Связь с сайтом')} description={tr('Через deloist.ru ходят только зашифрованные сообщения между вашими устройствами: сайт прочитать их не может.')}>
    <Row label={info?.pcName ? tr('Этот ПК: {name}', { name: info.pcName }) : tr('Этот ПК')} sublabel={linkText(info)}>
      <span class="dot" class:is-on={info?.connected} role="img" aria-label={info?.connected ? tr('в сети') : tr('не в сети')}></span>
    </Row>
  </Section>

  {#if server.supported}
    <Section title={tr('Режим сервера')} description={tr('Для старого ноутбука, который стоит дома или на работе: Deloist стартует вместе с Windows без окна и не даёт компьютеру уснуть. Боты и рутины работают всегда, а управлять ими можно с телефона или другого ПК.')}>
      {#key serverKey}
        <Row label={tr('Режим сервера')} sublabel={tr('Запуск с Windows без окна, значок остаётся в трее. Настройки питания не меняются, экран гаснуть может. Закрытая крышка ноутбука сон не отменяет.')}>
          <Switch checked={server.enabled} aria-label={tr('Режим сервера')} disabled={busy === 'server'} onchange={setServer} />
        </Row>
      {/key}
      {#if server.enabled}
        <Row label={server.awake ? tr('Режим сервера: включён · не засыпает') : tr('Режим сервера: включён · сон не отключён')}>
          <span class="dot" class:is-on={server.awake} role="img" aria-label={server.awake ? tr('в сети') : tr('не в сети')} data-server-status></span>
        </Row>
      {/if}
      {#if serverError}<p class="note is-error" role="alert">{serverError}</p>{/if}
    </Section>
  {/if}

  <Section title={tr('Доверенные устройства')} description={tr('Телефон или браузер, которым вы разрешили писать ботам. Новое устройство подтверждается на плашке с кодом.')}>
    {#if !devices.length}
      <p class="empty">{tr('Пока ни одного устройства. Откройте deloist.ru на телефоне и войдите в аккаунт: здесь появится запрос.')}</p>
    {/if}
    {#each devices as device (device.id)}
      <div class="device" data-device={device.id}>
        <Row label={nameOf(device)} sublabel={`${kindLabel(device.kind)} · ${device.online ? tr('в сети') : agoText(device.lastSeen)}`}>
          {#if revoking === device.id}
            <span class="inline">
              <Button size="sm" variant="attention" loading={busy === device.id} onclick={() => revoke(device)}>{tr('Отозвать')}</Button>
              <Button size="sm" variant="ghost" onclick={() => (revoking = '')}>{tr('Отмена')}</Button>
            </span>
          {:else}
            <Button size="sm" variant="ghost" onclick={() => (revoking = device.id)}>{tr('Отозвать…')}</Button>
          {/if}
        </Row>
        {#if revoking === device.id}
          <p class="hint">{tr('Устройство «{name}» сразу потеряет доступ к ботам. Чтобы вернуть его, нужно знакомство заново.', { name: nameOf(device) })}</p>
        {/if}
        {#key switchKey[device.id] ?? 0}
          <Row label={tr('Разрешать действия в рабочей папке')} sublabel={tr('По умолчанию устройство может только читать и писать боту. Здесь вы разрешаете ему запускать команды и скрипты бота.')}>
            <Switch
              checked={Boolean(device.actions)}
              aria-label={tr('Разрешать действия в рабочей папке')}
              disabled={busy === device.id}
              onchange={(on) => (on ? askActions(device) : setActions(device, false))}
            />
          </Row>
        {/key}
        {#if actionsAsk === device.id}
          <div class="ask">
            <p class="hint">
              {tr('С этим правом бот по просьбе устройства сможет запускать команды, скрипты и работать в браузере внутри своей рабочей папки. Секреты, оплата и удаление по-прежнему подтверждаются только на ПК.')}
            </p>
            <Field label={tr('Введите имя устройства точно: {name}', { name: nameOf(device) })} error={actionsError}>
              {#snippet children({ id, describedby, invalid })}
                <TextInput {id} {describedby} {invalid} bind:value={actionsName} autocomplete="off" />
              {/snippet}
            </Field>
            <span class="inline">
              <Button size="sm" variant="primary" disabled={actionsName !== device.name} loading={busy === device.id} onclick={() => setActions(device, true)}>{tr('Включить')}</Button>
              <Button size="sm" variant="ghost" onclick={() => cancelActions(device)}>{tr('Отмена')}</Button>
            </span>
          </div>
        {/if}
      </div>
    {/each}
  </Section>

  <Section title={tr('Другие ПК')} description={tr('Боты других ваших ПК появятся в боковой панели отдельным разделом. Чтобы они увидели ботов этого ПК, подключение нужно сделать и с их стороны.')}>
    {#each remote.pcs as pc (pc.id)}
      <Row label={pc.name || tr('Без имени')} sublabel={pc.online ? tr('в сети') : pc.status === 'no_link' ? tr('нет связи с сайтом') : tr('не в сети')}>
        {#if forgetting === pc.id}
          <span class="inline">
            <Button size="sm" variant="attention" loading={busy === pc.id} onclick={() => forget(pc)}>{tr('Отключить')}</Button>
            <Button size="sm" variant="ghost" onclick={() => (forgetting = '')}>{tr('Отмена')}</Button>
          </span>
        {:else}
          <Button size="sm" variant="ghost" onclick={() => (forgetting = pc.id)}>{tr('Отключить…')}</Button>
        {/if}
      </Row>
    {/each}

    {#if shownPair}
      <div class="ask pair" data-pairing>
        <p class="hint">{tr('Введите этот код на ПК «{name}» в плашке запроса:', { name: shownPair.name })}</p>
        <div class="code" aria-label={tr('Код для другого ПК')}>{groupCode(shownPair.code)}</div>
        <Field label={tr('Код с экрана того ПК')} error={pairError} hint={remote.pairing && !remote.pairing.welcomed ? tr('Ждём, пока другой ПК подтвердит запрос…') : ''}>
          {#snippet children({ id, describedby, invalid })}
            <TextInput
              {id}
              {describedby}
              {invalid}
              inputmode="numeric"
              autocomplete="off"
              placeholder="000 000"
              value={groupCode(ackCode)}
              oninput={(e) => (ackCode = digitsOnly(e.currentTarget.value))}
              onkeydown={(e) => e.key === 'Enter' && ackCode.length === 6 && confirmPair()}
            />
          {/snippet}
        </Field>
        <span class="inline">
          <Button size="sm" variant="primary" disabled={ackCode.length !== 6} loading={pairBusy} onclick={confirmPair}>{tr('Подключить')}</Button>
          <Button size="sm" variant="ghost" onclick={() => { pair = null; pairError = ''; }}>{tr('Скрыть')}</Button>
        </span>
      </div>
    {:else}
      {#each remote.candidates as candidate (candidate.id)}
        <Row label={candidate.name || tr('Без имени')} sublabel={candidate.online ? tr('в сети, не подключён') : tr('не в сети')}>
          <Button size="sm" disabled={!candidate.online || pairBusy} onclick={() => startPair(candidate)}>{tr('Подключить ПК')}</Button>
        </Row>
      {/each}
      {#if !remote.pcs.length && !remote.candidates.length}
        <p class="empty">{tr('Других ПК в вашем аккаунте сейчас нет. Войдите в тот же аккаунт Deloist на втором ПК.')}</p>
      {/if}
      {#if pairError}<p class="note is-error" role="alert">{pairError}</p>{/if}
    {/if}
  </Section>
</div>

<style>
  .devices { display: flex; flex-direction: column; gap: var(--k-space-4, 16px); }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--k-text-3); display: inline-block; transition: background 0.15s cubic-bezier(0.22, 1, 0.36, 1); }
  .dot.is-on { background: var(--k-brand); }
  .inline { display: inline-flex; gap: var(--k-space-2, 8px); align-items: center; flex-wrap: wrap; }
  .hint, .empty, .note { margin: 0; padding: 0 var(--k-space-3, 12px); font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, inherit); color: var(--k-text-3); overflow-wrap: anywhere; }
  .note.is-error { color: var(--k-attention-text); }
  .ask { display: flex; flex-direction: column; gap: var(--k-space-2, 8px); padding: var(--k-space-3, 12px); border-radius: var(--k-radius-md, 12px); background: var(--k-surface-2); animation: ask-in 0.3s cubic-bezier(0.34, 1.36, 0.64, 1); }
  .ask .hint { padding: 0; }
  .code { font: 600 28px / 1.2 var(--k-font-mono, monospace); letter-spacing: 0.18em; text-align: center; user-select: all; }
  @keyframes ask-in { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { .ask { animation: none; } .dot { transition: none; } }
</style>
