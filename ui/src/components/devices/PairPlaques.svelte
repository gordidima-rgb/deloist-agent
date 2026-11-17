<script>
  // Плашки «разрешить или запретить доступ с устройства к ботам». Не модальное окно: фокус не забирает,
  // стопкой сверху окна. Решение уходит только после ввода кода с экрана устройства. Имя, браузер и геометка
  // приходят от устройства и показываются только текстом.
  import { onMount } from 'svelte';
import { Button } from '../kit';
  import { tr, trServer } from '../../lib/i18n.js';
  import { enc, groupCode, digitsOnly, kindLabel, errorText } from '../../lib/remote.js';
  import { whenText } from '../../lib/when.js';

  let { api } = $props();

  // Запрос живёт 5 минут (как на ПК); часы устройства и ПК могут расходиться, поэтому отсчёт локальный.
  const TTL_MS = 5 * 60 * 1000;
  let plaques = $state([]);
  const timers = new Map();

  function drop(req) {
    clearTimeout(timers.get(req));
    timers.delete(req);
    plaques = plaques.filter((p) => p.req !== req);
  }

  function add(info) {
    const req = String(info?.req ?? '');
    if (!req || plaques.some((p) => p.req === req)) return;
    const age = Number(info.at) > 0 ? Math.min(TTL_MS - 1000, Math.max(0, Date.now() - Number(info.at))) : 0;
    plaques = [
      ...plaques,
      {
        req,
        name: String(info.name ?? ''),
        kind: String(info.kind ?? ''),
        ua: String(info.ua ?? ''),
        geo: String(info.geo ?? ''),
        at: Number(info.at) || Date.now(),
        code: '',
        busy: false,
        error: '',
        closed: false,
        ack: '',
      },
    ];
    timers.set(req, setTimeout(() => drop(req), TTL_MS - age));
  }

  function patch(req, next) {
    plaques = plaques.map((p) => (p.req === req ? { ...p, ...next } : p));
  }

  // Что ПК считает открытым сейчас: исчезнувшие запросы (отказ с устройства, истёк срок) снимаем.
  async function sync() {
    let list;
    try {
      list = await api('/api/devices');
    } catch {
      return;
    }
    const pending = Array.isArray(list?.pending) ? list.pending : [];
    for (const info of pending) add(info);
    const alive = new Set(pending.map((p) => String(p.req)));
    for (const p of plaques) if (!alive.has(p.req) && !p.ack && !p.closed) drop(p.req);
  }

  function onEvent(event) {
    const ev = event.detail;
    if (ev?.type === 'device_pair_request') add(ev);
    else if (ev?.type === 'devices_changed') sync();
  }

  onMount(() => {
    window.addEventListener('deloist:event', onEvent);
    sync();
    return () => {
      window.removeEventListener('deloist:event', onEvent);
      for (const id of timers.values()) clearTimeout(id);
      timers.clear();
    };
  });

  async function decide(p, allow) {
    if (p.busy) return;
    patch(p.req, { busy: true, error: '' });
    try {
      const answer = await api(`/api/devices/pair/${enc(p.req)}`, {
        method: 'POST',
        body: allow ? { allow: true, code: p.code } : { allow: false },
      });
      if (allow && answer?.allowed) {
        clearTimeout(timers.get(p.req));
        timers.delete(p.req);
        patch(p.req, { busy: false, ack: String(answer.ack ?? ''), code: '' });
      } else {
        drop(p.req);
      }
    } catch (err) {
      // 409: код не совпал, в тексте сервера — сколько попыток осталось; 403/404: запрос закрыт.
      const closed = err?.status === 403 || err?.status === 404;
      patch(p.req, { busy: false, error: trServer(errorText(err)), closed, code: closed ? '' : p.code });
    }
  }

  const when = (ms) => whenText(ms);
</script>

{#if plaques.length}
  <div class="plaques" role="region" aria-label={tr('Запросы доступа с устройств')} aria-live="polite">
    {#each plaques as p (p.req)}
      <section class="plaque" aria-label={tr('Доступ с устройства «{name}»', { name: p.name })} data-req={p.req}>
        {#if p.ack}
          <h3 class="plaque__title">{tr('Устройство «{name}» допущено', { name: p.name })}</h3>
          <p class="plaque__text">{tr('Код для устройства: введите его на устройстве, чтобы завершить подключение.')}</p>
          <div class="plaque__ack" aria-label={tr('Код для устройства')}>{groupCode(p.ack)}</div>
          <div class="plaque__actions"><Button size="sm" onclick={() => drop(p.req)}>{tr('Готово')}</Button></div>
        {:else}
          <h3 class="plaque__title">{tr('Разрешить доступ с устройства «{name}» к ботам?', { name: p.name })}</h3>
          <p class="plaque__meta">
            {[kindLabel(p.kind), p.ua, p.geo, when(p.at)].filter(Boolean).join(' · ')}
          </p>
          {#if !p.closed}
            <label class="plaque__field">
              <span>{tr('Введите код с экрана устройства')}</span>
              <input
                class="k-input plaque__code"
                inputmode="numeric"
                autocomplete="off"
                maxlength="7"
                placeholder="000 000"
                value={groupCode(p.code)}
                aria-invalid={p.error ? 'true' : undefined}
                oninput={(e) => patch(p.req, { code: digitsOnly(e.currentTarget.value), error: '' })}
                onkeydown={(e) => e.key === 'Enter' && p.code.length === 6 && decide(p, true)}
              />
            </label>
          {/if}
          {#if p.error}<p class="plaque__error" role="alert">{p.error}</p>{/if}
          <div class="plaque__actions">
            {#if p.closed}
              <Button size="sm" onclick={() => drop(p.req)}>{tr('Закрыть')}</Button>
            {:else}
              <Button size="sm" variant="primary" disabled={p.busy || p.code.length !== 6} onclick={() => decide(p, true)}>{tr('Разрешить')}</Button>
              <Button size="sm" variant="ghost" disabled={p.busy} onclick={() => decide(p, false)}>{tr('Запретить')}</Button>
            {/if}
          </div>
        {/if}
      </section>
    {/each}
  </div>
{/if}

<style>
  .plaques {
    position: fixed;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 70;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    width: min(440px, calc(100vw - 32px));
    pointer-events: none;
  }
  .plaque {
    pointer-events: auto;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-4, 16px);
    background: var(--k-surface);
    color: var(--k-text);
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-lg, 16px);
    box-shadow: var(--k-shadow-3);
    animation: plaque-in 0.35s cubic-bezier(0.34, 1.36, 0.64, 1);
  }
  @keyframes plaque-in {
    from { opacity: 0; transform: translateY(-12px) scale(0.96); }
    to { opacity: 1; transform: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .plaque { animation: none; }
  }
  h3.plaque__title { margin: 0; font: 600 var(--k-text-md, 15px) / 1.35 var(--k-font-body, inherit); overflow-wrap: anywhere; }
  .plaque__meta, .plaque__text { margin: 0; font: 400 var(--k-text-xs, 12px) / 1.45 var(--k-font-body, inherit); color: var(--k-text-3); overflow-wrap: anywhere; }
  .plaque__field { display: flex; flex-direction: column; gap: 4px; font: 600 var(--k-text-sm, 13px) / 1.3 var(--k-font-body, inherit); color: var(--k-text-2); }
  .plaque__code { font: 600 var(--k-text-xl) / 1.2 var(--k-font-mono, monospace); letter-spacing: 0.12em; text-align: center; }
  .plaque__ack { font: 600 var(--k-text-3xl) / 1.2 var(--k-font-mono, monospace); letter-spacing: 0.18em; text-align: center; padding: var(--k-space-2, 8px); background: var(--k-surface-2); border-radius: var(--k-radius-md, 12px); user-select: all; }
  .plaque__error { margin: 0; font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, inherit); color: var(--k-attention-text); }
  .plaque__actions { display: flex; gap: var(--k-space-2, 8px); justify-content: flex-end; }
</style>
