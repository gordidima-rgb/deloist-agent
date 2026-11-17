<script>
  // Окно аккаунта: возврат оплаченных денег, выгрузка своих данных, цены и удаление аккаунта.
  // Удаление — в два шага: сначала «Удалить аккаунт», потом подтверждение. Если на балансе
  // остались оплаченные деньги, сайт откажет, и окно предложит вернуть их или отказаться от них.
import { Button, Checkbox } from './kit';
  import { tr } from '../lib/i18n.js';

  let { api, siteUrl = '', ondeleted = null, onnotice = null } = $props();

  let details = $state(null);
  let busy = $state('');
  let error = $state('');
  let confirming = $state(false);
  let paidLeft = $state('');
  let forfeit = $state(false);

  const prices = $derived(siteUrl ? new URL('/tseny', siteUrl).toString() : '');
  const refundable = $derived(Number(details?.refundableKop) || 0);
  const refundAsked = $derived(Boolean(details?.refundRequest));

  function money(kop) {
    return (kop / 100).toLocaleString(globalThis.LOCALE || undefined, { style: 'currency', currency: 'RUB', minimumFractionDigits: 0, maximumFractionDigits: 2 });
  }

  async function load() {
    try {
      details = await api('/api/account/details');
    } catch {
      details = null; // Сайт ещё без этих адресов или без сети: кнопки возврата просто нет.
    }
  }

  $effect(() => {
    load();
  });

  async function run(name, work) {
    if (busy) return;
    busy = name;
    error = '';
    try {
      await work();
    } catch (err) {
      error = err?.message || tr('Не удалось выполнить запрос');
    } finally {
      busy = '';
    }
  }

  const refund = () =>
    run('refund', async () => {
      const res = await api('/api/account/refund', { method: 'POST' });
      onnotice?.(tr('Заявка на возврат {sum} принята. Деньги вернутся в течение {days} дней', { sum: money(res.requestedKop), days: res.days }));
      await load();
    });

  const exportData = () =>
    run('export', async () => {
      const res = await api('/api/account/export', { method: 'POST' });
      onnotice?.(tr('Данные сохранены: {path}', { path: res.path }));
    });

  const remove = () =>
    run('delete', async () => {
      try {
        await api('/api/account/delete', { method: 'POST', body: { confirm: 'delete', forfeitBalance: Boolean(paidLeft && forfeit) } });
      } catch (err) {
        if (err?.code === 'paid_balance_left') {
          paidLeft = err.message;
          return;
        }
        throw err;
      }
      confirming = false;
      ondeleted?.();
    });

  function cancel() {
    confirming = false;
    paidLeft = '';
    forfeit = false;
    error = '';
  }
</script>

<div class="acr">
  {#if refundable > 0}
    <p class="acr-line">
      {tr('Можно вернуть: {sum}. Подарочные деньги не возвращаются.', { sum: money(refundable) })}
    </p>
  {/if}
  <div class="acr-actions">
    {#if refundable > 0 && !refundAsked}
      <Button size="sm" loading={busy === 'refund'} onclick={refund}>{tr('Вернуть деньги')}</Button>
    {:else if refundAsked}
      <span class="acr-line">{tr('Заявка на возврат в работе')}</span>
    {/if}
    <Button size="sm" loading={busy === 'export'} onclick={exportData}>{tr('Скачать мои данные')}</Button>
    {#if prices}<a class="acr-link" href={prices} target="_blank" rel="noopener">{tr('Цены')}</a>{/if}
  </div>

  {#if !confirming}
    <Button size="sm" variant="ghost" class="acr-danger" onclick={() => (confirming = true)}>{tr('Удалить аккаунт')}</Button>
  {:else}
    <div class="acr-confirm" role="group" aria-label={tr('Удаление аккаунта')}>
      <p class="acr-line acr-strong">{tr('Аккаунт и его данные на deloist.ru удалятся без возврата, программа выйдет из аккаунта. Боты и чаты на этом компьютере останутся.')}</p>
      {#if paidLeft}
        <p class="acr-line">{paidLeft}</p>
        <Checkbox bind:checked={forfeit} label={tr('Отказываюсь от оплаченных денег на балансе')} />
      {/if}
      <div class="acr-actions">
        <Button size="sm" variant="attention" loading={busy === 'delete'} disabled={Boolean(paidLeft) && !forfeit} onclick={remove}>{tr('Удалить навсегда')}</Button>
        <Button size="sm" variant="ghost" onclick={cancel}>{tr('Отмена')}</Button>
      </div>
    </div>
  {/if}
  {#if error}<p class="acr-error" role="alert">{error}</p>{/if}
</div>

<style>
  .acr {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--k-space-2, 8px);
    margin: var(--k-space-3, 12px) 0;
  }
  .acr-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-2, 8px);
  }
  .acr-line {
    margin: 0;
    color: var(--k-text-2, #4a4a4a);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .acr-strong {
    color: var(--k-text, #121212);
  }
  .acr-link {
    color: var(--k-brand, #025bff);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .acr-confirm {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-3, 12px);
    border: 1px solid var(--k-line-strong, #d2d2d2);
    border-radius: var(--k-radius-md, 14px);
  }
  .acr :global(.acr-danger) {
    color: var(--k-danger, #d92d20);
  }
  .acr-error {
    margin: 0;
    color: var(--k-danger, #d92d20);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
</style>
