<script module>
  import { tr } from '../lib/i18n.js';

  // Названия документов сайта. Неизвестный kind показываем как есть: сайт мог добавить новый.
  export function docTitle(kind) {
    switch (kind) {
      case 'pdn':
        return tr('Согласие на обработку персональных данных');
      case 'offer':
        return tr('Оферта: условия использования Deloist');
      case 'privacy':
        return tr('Политика конфиденциальности');
      case 'cross_border':
        return tr('Согласие на передачу данных за границу');
      default:
        return tr('Документ «{kind}»', { kind });
    }
  }

  export function docNote(kind) {
    if (kind === 'pdn') {
      return tr('Почта и платежи для аккаунта. Для моделей на балансе Deloist текст запросов уходит провайдеру модели, в том числе DeepSeek (Китай, КНР).');
    }
    if (kind === 'cross_border') return tr('Модели на балансе Deloist работают на серверах за пределами России, в том числе в Китае (DeepSeek).');
    return '';
  }
</script>

<script>
  // Отдельное согласие с документами Deloist (152-ФЗ, ст. 9): у каждого документа своя галочка,
  // по умолчанию пустая, и «Принимаю» доступна только когда отмечено всё. До согласия
  // не работает только баланс Deloist; свои ключи и подписки работают как раньше.
import { Modal, Checkbox, Button } from './kit';

  let { docs = [], onaccept = null, onlater = null } = $props();

  let open = $state(true);
  // Отметки по документу и редакции: новый список от сайта не наследует старые галочки.
  let checked = $state({});
  const key = (d) => `${d.kind}@${d.version}`;
  let busy = $state(false);
  let error = $state('');

  const all = $derived(docs.length > 0 && docs.every((d) => checked[key(d)]));

  async function accept() {
    if (!all || busy) return;
    busy = true;
    error = '';
    try {
      await onaccept?.(docs.map((d) => ({ kind: d.kind, version: d.version })));
      open = false;
    } catch (err) {
      error = err?.message || tr('Не удалось выполнить запрос');
    } finally {
      busy = false;
    }
  }

  function later() {
    open = false;
    onlater?.();
  }
</script>

<Modal bind:open title={tr('Документы Deloist')} description={tr('Чтобы работать на балансе Deloist, прочитай и прими документы. Каждый открывается в браузере.')} onclose={() => onlater?.()} class="cst-modal">
  <ul class="cst-list">
    {#each docs as doc (key(doc))}
      <li class="cst-doc">
        <Checkbox checked={Boolean(checked[key(doc)])} onchange={(value) => (checked[key(doc)] = value)} label={docTitle(doc.kind)} data-kind={doc.kind} />
        <div class="cst-meta">
          {#if docNote(doc.kind)}<p class="cst-note">{docNote(doc.kind)}</p>{/if}
          <a class="cst-link" href={doc.url} target="_blank" rel="noopener">{tr('Прочитать')}</a>
          <span class="cst-version">{tr('редакция {version}', { version: doc.version })}</span>
        </div>
      </li>
    {/each}
  </ul>
  <p class="cst-hint">{tr('Без согласия баланс Deloist не работает. Свои ключи и подписки работают как раньше.')}</p>
  {#if error}<p class="cst-error" role="alert">{error}</p>{/if}
  {#snippet footer()}
    <Button variant="ghost" onclick={later}>{tr('Позже')}</Button>
    <Button variant="primary" disabled={!all} loading={busy} onclick={accept}>{tr('Принимаю')}</Button>
  {/snippet}
</Modal>

<style>
  .cst-list {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .cst-doc {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-1, 4px);
    padding: var(--k-space-3, 12px);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md, 14px);
    background: var(--k-surface);
  }
  .cst-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--k-space-1, 4px) var(--k-space-3, 12px);
    padding-left: var(--k-space-6);
  }
  .cst-note {
    flex-basis: 100%;
    margin: 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .cst-link {
    color: var(--k-brand);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .cst-version {
    color: var(--k-text-3);
    font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, system-ui);
  }
  .cst-hint {
    margin: var(--k-space-3, 12px) 0 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .cst-error {
    margin: var(--k-space-2, 8px) 0 0;
    color: var(--k-danger);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
</style>
