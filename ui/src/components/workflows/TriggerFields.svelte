<script>
  // Тело узла «Триггер»: вид запуска (значки) и настройки вида: расписание, адрес вебхука, фильтр сообщений.
  import { getContext } from 'svelte';
  import { tr, trServer } from '../../lib/i18n.js';
  import { TRIGGER_KINDS, isMessenger, isLinked, triggerOf } from '../../lib/workflows/graph.js';
  import { toast } from '../../lib/workflows/notify.js';
  import { WF } from './ctx.js';
  import { triggerLabel, scheduleText, nextRunText } from './Labels.svelte';
  import KindChips from './KindChips.svelte';
  import LinkHint from './LinkHint.svelte';
  import ScheduleEditor from './ScheduleEditor.svelte';
  import WfIcon from './WfIcon.svelte';

  let { id, fields = {}, schedule } = $props();

  const ctx = getContext(WF);
  const kind = $derived(triggerOf(fields));
  const trig = $derived(ctx.triggers.get(id));
  const linked = $derived(isLinked(kind, ctx.connections));
  let schedOpen = $state(false);
  let hookUrl = $state('');
  let hookBusy = $state(false);
  let copied = $state(false);

  function pick(value) {
    if (value === kind) return;
    ctx.model.setFields(id, { trigger: value });
    if (value === 'schedule' && !schedule) ctx.model.setSchedule(id, { kind: 'daily', time: '09:00' });
    schedOpen = value === 'schedule';
    hookUrl = '';
  }

  async function makeHook() {
    if (hookBusy) return;
    hookBusy = true;
    try {
      hookUrl = await ctx.makeWebhook(id);
      copied = false;
    } catch (err) {
      toast(trServer(err?.message || tr('Не получилось')));
    } finally {
      hookBusy = false;
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(hookUrl);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch {}
  }
</script>

<KindChips options={TRIGGER_KINDS} value={kind} label={triggerLabel} aria={tr('Как запускать')} unavailable={(k) => !isLinked(k, ctx.connections)} onchange={pick} />

{#if kind === 'schedule'}
  <button type="button" class="wft-sched nodrag" aria-expanded={schedOpen} onclick={() => (schedOpen = !schedOpen)}>
    <WfIcon name="clock" size={14} />
    <span>{scheduleText(schedule)}</span>
    <WfIcon name="chevron" size={14} />
  </button>
  {#if schedOpen}
    <ScheduleEditor {schedule} onchange={(s) => ctx.model.setSchedule(id, s)} />
  {/if}
  <p class="wft-note">
    <span>{tr('Следующий запуск')}</span>
    <b>{nextRunText(trig?.next_run) || tr('после сохранения')}</b>
  </p>
{:else if kind === 'webhook'}
  {#if hookUrl}
    <div class="wft-url nodrag nowheel">
      <input type="text" readonly value={hookUrl} aria-label={tr('Адрес вебхука')} onfocus={(e) => e.currentTarget.select()} />
      <button type="button" class="wft-copy" class:done={copied} title={tr('Скопировать адрес')} aria-label={tr('Скопировать адрес')} onclick={copy}>
        <WfIcon name={copied ? 'check' : 'copy'} size={14} />
      </button>
    </div>
    <p class="wft-warn">{tr('Адрес показан один раз: скопируйте его сейчас. Потом его не открыть, можно только выпустить новый.')}</p>
  {:else if trig?.hook}
    <p class="wft-note"><span>{tr('Адрес создан и хранится скрыто.')}</span></p>
  {:else}
    <p class="wft-note"><span>{tr('Любой POST на адрес запускает процесс, а тело запроса попадёт в первый шаг как данные.')}</span></p>
  {/if}
  <button type="button" class="wft-btn nodrag" disabled={hookBusy} onclick={makeHook}>
    <WfIcon name="webhook" size={14} />
    <span>{trig?.hook || hookUrl ? tr('Новый адрес') : tr('Создать адрес')}</span>
  </button>
  {#if trig?.hook || hookUrl}
    <p class="wft-warn wft-warn--soft">{tr('Старый адрес после этого перестанет работать.')}</p>
  {/if}
{:else if isMessenger(kind)}
  <input class="wft-input nodrag" type="text" placeholder={tr('Только сообщения с этим текстом (необязательно)')} value={fields.text || ''} oninput={(e) => ctx.model.setField(id, 'text', e.currentTarget.value)} />
  {#if !linked}
    <LinkHint name={triggerLabel(kind)} onopen={() => ctx.openSettings(kind)} />
  {:else}
    <p class="wft-note"><span>{tr('Срабатывает на сообщения из привязанного чата.')}</span></p>
  {/if}
{:else}
  <p class="wft-note"><span>{tr('Запускается кнопкой «Запустить» наверху')}</span></p>
{/if}
{#if kind !== 'manual' && !ctx.enabled}
  <p class="wft-off"><WfIcon name="pause" size={12} /><span>{tr('Процесс выключен: такой запуск не сработает')}</span></p>
{/if}

<style>
  .wft-note,
  .wft-warn,
  .wft-off {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 8px;
    margin: 0;
    font: 400 12.5px/1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wft-note b {
    font-weight: 600;
    color: var(--k-text-2);
  }
  .wft-warn {
    padding: 7px 10px;
    border-radius: 11px;
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
    font-weight: 500;
    animation: wft-in 0.4s var(--k-ease-spring, ease) both;
  }
  .wft-warn--soft {
    background: transparent;
    padding: 0 2px;
    color: var(--k-text-3);
    animation: none;
  }
  .wft-off {
    align-items: center;
    padding: 6px 10px;
    border-radius: 11px;
    background: var(--k-surface-2);
    color: var(--k-text-2);
  }
  .wft-sched {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 36px;
    padding: 0 10px;
    border: 1px solid var(--k-line);
    border-radius: 12px;
    background: var(--k-surface);
    color: var(--k-text);
    font: 600 13px/1 var(--k-font-body);
    cursor: pointer;
  }
  .wft-sched span {
    flex: 1;
    text-align: left;
  }
  .wft-url {
    display: flex;
    gap: 6px;
    animation: wft-in 0.45s var(--k-ease-spring, ease) both;
  }
  .wft-url input,
  .wft-input {
    flex: 1;
    min-width: 0;
    height: 36px;
    padding: 0 10px;
    border: 1px solid var(--k-line);
    border-radius: 12px;
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 500 12.5px/1 var(--k-font-body);
  }
  .wft-input {
    width: 100%;
    box-sizing: border-box;
    background: var(--k-surface);
  }
  .wft-input:focus,
  .wft-url input:focus {
    outline: 2px solid color-mix(in oklab, var(--k-brand) 45%, transparent);
    outline-offset: 1px;
  }
  .wft-copy,
  .wft-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border: 0;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wft-copy {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background: var(--k-surface-3);
    color: var(--k-text);
  }
  .wft-copy.done {
    background: color-mix(in oklab, var(--k-bot-mint) 40%, var(--k-surface));
  }
  .wft-btn {
    height: 36px;
    padding: 0 14px;
    border-radius: 12px;
    background: var(--k-surface-3);
    color: var(--k-text);
    font: 700 13px/1 var(--k-font-body);
  }
  .wft-btn:hover,
  .wft-copy:hover {
    background: var(--k-line-strong, var(--k-surface-3));
  }
  .wft-btn:active,
  .wft-copy:active {
    transform: scale(0.95);
  }
  .wft-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .wft-sched:focus-visible,
  .wft-btn:focus-visible,
  .wft-copy:focus-visible {
    outline: 2px solid var(--k-brand);
    outline-offset: 2px;
  }
  @keyframes wft-in {
    from {
      opacity: 0;
      transform: translateY(-5px) scale(0.97);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wft-warn,
    .wft-url,
    .wft-copy,
    .wft-btn {
      animation: none;
      transition: none;
    }
  }
</style>
