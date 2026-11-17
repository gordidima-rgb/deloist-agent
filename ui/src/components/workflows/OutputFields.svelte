<script>
  // Тело узла «Вывод»: куда отдать результат (значки) и настройки места: бот, файл.
  import { getContext } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { OUTPUT_TARGETS, isMessenger, isLinked, targetOf } from '../../lib/workflows/graph.js';
  import { outputParts, badOutputPath } from '../../lib/workflows/assets.js';
  import { WF } from './ctx.js';
  import { targetLabel } from './Labels.svelte';
  import KindChips from './KindChips.svelte';
  import LinkHint from './LinkHint.svelte';
  import BotPicker from './BotPicker.svelte';
  import WfIcon from './WfIcon.svelte';
  import { TextInput } from '../kit';

  let { id, fields = {} } = $props();

  const ctx = getContext(WF);
  const target = $derived(targetOf(fields));
  const parts = $derived(outputParts(fields.path));
  const bad = $derived(badOutputPath(fields.path));
</script>

<KindChips options={OUTPUT_TARGETS} value={target} label={targetLabel} aria={tr('Куда отдать результат')} unavailable={(k) => !isLinked(k, ctx.connections)} onchange={(v) => ctx.model.setFields(id, { target: v })} />

{#if target === 'chat'}
  <BotPicker bots={ctx.bots} value={fields.bot_id || ''} onchange={(v) => ctx.model.setFields(id, { bot_id: v })} />
  <p class="wfo-note">{tr('Текст появится в переписке бота, ход бот не начнёт.')}</p>
{:else if isMessenger(target)}
  {#if isLinked(target, ctx.connections)}
    <p class="wfo-note">{tr('Отправим в привязанный чат. В другие места процесс не пишет.')}</p>
  {:else}
    <LinkHint name={targetLabel(target)} onopen={() => ctx.openSettings(target)} />
  {/if}
{:else if target === 'file'}
  <TextInput class="nodrag" invalid={bad} placeholder={tr('Путь, например отчёты/итог.txt')} value={fields.path || ''} oninput={(e) => ctx.model.setField(id, 'path', e.currentTarget.value)} />
  <div class="wfo-place" class:is-bad={bad} aria-live="polite">
    <WfIcon name="file" size={14} />
    {#if bad}
      <span>{tr('Путь должен быть внутри папки процесса: без диска, «..» и начального слэша.')}</span>
    {:else}
      <span class="wfo-trail">
        <span>{tr('Файлы процесса')}</span>
        {#each parts as p, i (i)}<i>›</i><b>{p}</b>{/each}
        {#if !parts.length}<i>›</i><em>{tr('укажите имя файла')}</em>{/if}
      </span>
    {/if}
  </div>
  <p class="wfo-note">{tr('Файл до 1 МБ. Перезапись при каждом запуске.')}</p>
{:else}
  <p class="wfo-note">{tr('Уведомление в программе с текстом результата.')}</p>
{/if}

<style>
  .wfo-note {
    margin: 0;
    font: 400 12.5px/1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfo-place {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 12px;
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font: 500 12.5px/1.4 var(--k-font-body);
    transition: background 200ms var(--k-ease-out, ease);
  }
  .wfo-place.is-bad {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .wfo-trail {
    display: flex;
    flex-wrap: wrap;
    gap: 3px 6px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .wfo-trail i {
    font-style: normal;
    color: var(--k-text-3);
  }
  .wfo-trail em {
    color: var(--k-text-3);
  }
</style>
