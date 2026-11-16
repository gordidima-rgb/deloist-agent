<script>
  // Карточка шага: триггер, промпт, бот, одобрение, вывод (и «скоро»: модель, условие).
  // Свойства приходят от Svelte Flow; данные меняем через модель холста из контекста.
  import { getContext } from 'svelte';
  import { Handle, Position } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { KINDS, isEnabled, stepSeconds, outTypeOf } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { kindTitle, kindHint, statusLabel, scheduleText, nextRunText, seconds } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';
  import BotPicker from './BotPicker.svelte';
  import ScheduleEditor from './ScheduleEditor.svelte';
  import { buildGroups, labelOf } from '../../lib/modelPicker.js';

  let { id, data, selected = false } = $props();

  const ctx = getContext(WF);
  const kind = $derived(data.kind);
  const meta = $derived(KINDS[kind] || {});
  const fields = $derived(data.fields || {});
  const step = $derived(ctx.steps.get(id));
  const status = $derived(step?.status || (ctx.runOpen ? 'waiting' : ''));
  const enabled = $derived(isEnabled(kind));
  const invalid = $derived(ctx.model.invalid.node_id === id);
  const hit = $derived(ctx.hits.has(id));
  const locked = $derived(Boolean(data.locked));
  const attempts = $derived(ctx.stepList(id));
  const pickAt = $derived(ctx.pickIndex(id));
  const modelText = $derived.by(() => {
    const value = { driver: fields.provider || '', model: fields.model || '' };
    const m = ctx.models();
    return labelOf(buildGroups({ providers: m.providers || [], plan: m.plan || null, settings: m.settings || {}, value }), value).text;
  });
  let systemOpen = $state(false);
  let textMode = $state(false);
  let fileInput = $state();
  const outType = $derived(outTypeOf(kind, fields));
  const portType = $derived(outType === 'image' || outType === 'file' ? outType : 'text');
  const shaking = $derived(ctx.shakeId() === id);
  const preview = $derived(ctx.assetPreview(id));
  const imageUrl = $derived(kind === 'image' && step?.status === 'done' && outText ? ctx.fileUrl(outText.trim()) : '');
  const sizeText = $derived.by(() => {
    const n = Number(fields.size) || 0;
    if (n >= 1048576) return `${(n / 1048576).toFixed(1)} MB`;
    if (n >= 1024) return `${Math.round(n / 1024)} KB`;
    return n ? `${n} B` : '';
  });
  const IMAGE_SIZES = ['1024x1024', '1536x1024', '1024x1536'];
  const pickedFile = (e) => {
    const f = e.currentTarget.files?.[0];
    if (f) ctx.assetFromFile(id, f);
    e.currentTarget.value = '';
  };
  const trig = $derived(ctx.triggers.get(id));
  const outText = $derived(typeof step?.output === 'string' ? step.output : step?.output == null ? '' : JSON.stringify(step.output, null, 2));
  const errText = $derived(step?.error ? String(step.error) : '');
  const elapsed = $derived(step ? stepSeconds(step, ctx.now) : 0);
  const hasResult = $derived(Boolean(step) && (status === 'done' || status === 'failed' || status === 'stopped') && Boolean(outText || errText));

  let resultOpen = $state(false);
  let schedOpen = $state(false);
  let copied = $state(false);

  const set = (key, value) => ctx.model.setField(id, key, value);

  async function copy() {
    try {
      await navigator.clipboard.writeText(errText || outText);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {}
  }

  function setTrigger(value) {
    if (value === fields.trigger) return;
    ctx.model.setFields(id, { trigger: value });
    if (value === 'schedule' && !data.schedule) ctx.model.setSchedule(id, { kind: 'daily', time: '09:00' });
    schedOpen = value === 'schedule';
  }
</script>

<div
  class="wfn"
  class:is-selected={selected}
  class:is-soon={!enabled}
  class:is-invalid={invalid}
  class:is-hit={hit}
  class:is-shake={shaking}
  data-kind={kind}
  data-status={status}
  style:--tone="var(--k-bot-{meta.tone || 'sky'})"
>
  {#if meta.inputs}
    <Handle type="target" position={Position.Left} class="wfn-port" />
  {/if}

  <header class="wfn-head">
    <span class="wfn-badge"><WfIcon name={kind} size={16} /></span>
    <span class="wfn-title">{kindTitle(kind)}</span>
    {#if step?.branch && kind === 'condition'}
      <span class="wfn-branch wfn-branch--{step.branch}">{step.branch === 'yes' ? tr('Да') : tr('Нет')}</span>
    {/if}
    {#if locked}
      <button type="button" class="wfn-lock nodrag" title={tr('Разблокировать')} aria-label={tr('Разблокировать')} onclick={() => ctx.model.toggleLock(id)}><WfIcon name="lock" size={13} /></button>
    {/if}
    <span class="wfn-state" aria-label={statusLabel(status)} title={statusLabel(status)}>
      {#if status === 'running'}
        <span class="wfn-spin"></span>
      {:else if status === 'done'}
        <span class="wfn-st wfn-st--done"><WfIcon name="check" size={13} /></span>
      {:else if status === 'failed'}
        <span class="wfn-st wfn-st--failed"><WfIcon name="x" size={13} /></span>
      {:else if status === 'approval'}
        <span class="wfn-st wfn-st--approval"><WfIcon name="pause" size={13} /></span>
      {:else if status === 'stopped'}
        <span class="wfn-st wfn-st--stopped"><WfIcon name="stop" size={11} /></span>
      {:else if status === 'skipped'}
        <span class="wfn-st wfn-st--skipped"><WfIcon name="skip" size={13} /></span>
      {:else if status === 'waiting'}
        <span class="wfn-st wfn-st--waiting"><WfIcon name="clock" size={13} /></span>
      {/if}
    </span>
  </header>

  <div class="wfn-body">
    {#if kind === 'trigger'}
      <div class="wfn-seg nodrag" role="radiogroup" aria-label={tr('Как запускать')}>
        <button type="button" role="radio" aria-checked={fields.trigger !== 'schedule'} class:on={fields.trigger !== 'schedule'} onclick={() => setTrigger('manual')}>{tr('Вручную')}</button>
        <button type="button" role="radio" aria-checked={fields.trigger === 'schedule'} class:on={fields.trigger === 'schedule'} onclick={() => setTrigger('schedule')}>{tr('По расписанию')}</button>
      </div>
      {#if fields.trigger === 'schedule'}
        <button type="button" class="wfn-sched nodrag" aria-expanded={schedOpen} onclick={() => (schedOpen = !schedOpen)}>
          <WfIcon name="clock" size={14} />
          <span>{scheduleText(data.schedule)}</span>
          <WfIcon name="chevron" size={14} />
        </button>
        {#if schedOpen}
          <ScheduleEditor schedule={data.schedule} onchange={(s) => ctx.model.setSchedule(id, s)} />
        {/if}
        <p class="wfn-next">
          <span>{tr('Следующий запуск')}</span>
          <b>{nextRunText(trig?.next_run) || tr('после сохранения')}</b>
        </p>
      {:else}
        <p class="wfn-hint">{tr('Запускается кнопкой «Запустить» наверху')}</p>
      {/if}
    {:else if kind === 'prompt'}
      <textarea class="wfn-area nodrag nowheel" rows="3" placeholder={tr('Что сказать следующему шагу')} value={fields.text || ''} oninput={(e) => set('text', e.currentTarget.value)}></textarea>
    {:else if kind === 'bot'}
      <BotPicker bots={ctx.bots} value={fields.bot_id || ''} onchange={(v) => ctx.model.setFields(id, { bot_id: v })} />
      <textarea class="wfn-area nodrag nowheel" rows="3" placeholder={tr('Что поручить боту')} value={fields.task || ''} oninput={(e) => set('task', e.currentTarget.value)}></textarea>
    {:else if kind === 'approval'}
      <input class="wfn-input nodrag" type="text" placeholder={tr('Что спросить у вас')} value={fields.question || ''} oninput={(e) => set('question', e.currentTarget.value)} />
      {#if status === 'approval' && step}
        <div class="wfn-decide nodrag">
          <button type="button" class="wfn-btn wfn-btn--ok" onclick={() => ctx.decide(step, true)}>{tr('Одобрить')}</button>
          <button type="button" class="wfn-btn wfn-btn--no" onclick={() => ctx.decide(step, false)}>{tr('Отклонить')}</button>
        </div>
      {/if}
    {:else if kind === 'llm'}
      <button type="button" class="wfn-model nodrag" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <WfIcon name="llm" size={14} />
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </button>
      <textarea class="wfn-area nodrag nowheel" rows="3" placeholder={tr('Что спросить у модели')} value={fields.prompt || ''} oninput={(e) => set('prompt', e.currentTarget.value)}></textarea>
      <button type="button" class="wfn-more nodrag" aria-expanded={systemOpen || Boolean(fields.system)} onclick={() => (systemOpen = !systemOpen)}>
        <WfIcon name="chevron" size={13} />
        <span>{tr('Системная инструкция')}</span>
      </button>
      {#if systemOpen || fields.system}
        <textarea class="wfn-area wfn-area--sys nodrag nowheel" rows="2" placeholder={tr('Роль и правила для модели')} value={fields.system || ''} oninput={(e) => set('system', e.currentTarget.value)}></textarea>
      {/if}
    {:else if kind === 'condition'}
      <textarea class="wfn-area nodrag nowheel" rows="2" placeholder={tr('Условие: например, в ответе есть ошибка')} value={fields.rule || ''} oninput={(e) => set('rule', e.currentTarget.value)}></textarea>
      <button type="button" class="wfn-model nodrag" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <WfIcon name="llm" size={14} />
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </button>
    {:else if kind === 'asset'}
      {#if fields.name || fields.text || textMode}
        <div class="wfn-file">
          {#if preview}
            <img class="wfn-file__img" src={preview} alt={fields.name} />
          {:else}
            <span class="wfn-file__ico"><WfIcon name={fields.name ? 'asset' : 'text'} size={20} /></span>
          {/if}
          <span class="wfn-file__meta">
            <b>{fields.name || tr('Текст')}</b>
            <small>{[fields.mime, sizeText].filter(Boolean).join(' · ') || tr('Готовый текст')}</small>
          </span>
          <button type="button" class="wfn-icon nodrag" title={tr('Убрать')} aria-label={tr('Убрать')} onclick={() => { textMode = false; ctx.clearAsset(id); }}><WfIcon name="x" size={14} /></button>
        </div>
        {#if !fields.name || fields.text}
          <textarea class="wfn-area nodrag nowheel" rows="3" placeholder={tr('Вставьте текст')} value={fields.text || ''} oninput={(e) => set('text', e.currentTarget.value)}></textarea>
        {:else if !fields.path}
          <p class="wfn-hint">{tr('Файл пока только на этом экране: на сервер он попадёт вместе с запуском.')}</p>
        {/if}
      {:else}
        <div class="wfn-drop nodrag" role="presentation">
          <span class="wfn-drop__ico"><WfIcon name="asset" size={22} /></span>
          <b>{tr('Перетащите файл сюда')}</b>
          <span class="wfn-drop__acts">
            <button type="button" class="wfn-btn wfn-btn--ok" onclick={() => fileInput?.click()}>{tr('Выбрать файл')}</button>
            <button type="button" class="wfn-btn wfn-btn--no" onclick={() => (textMode = true)}>{tr('Вставить текст')}</button>
          </span>
        </div>
      {/if}
      <input bind:this={fileInput} class="wfn-hidden" type="file" onchange={pickedFile} tabindex="-1" aria-hidden="true" />
    {:else if kind === 'image'}
      <button type="button" class="wfn-model nodrag" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <WfIcon name="image" size={14} />
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </button>
      <textarea class="wfn-area nodrag nowheel" rows="3" placeholder={tr('Что нарисовать')} value={fields.prompt || ''} oninput={(e) => set('prompt', e.currentTarget.value)}></textarea>
      <div class="wfn-seg wfn-seg--3 nodrag" role="radiogroup" aria-label={tr('Размер картинки')}>
        {#each IMAGE_SIZES as sz (sz)}
          <button type="button" role="radio" aria-checked={(fields.size || '1024x1024') === sz} class:on={(fields.size || '1024x1024') === sz} onclick={() => set('size', sz)}>{sz.replace('x', '×')}</button>
        {/each}
      </div>
      {#if imageUrl}
        <button type="button" class="wfn-thumb nodrag" title={tr('Открыть крупно')} onclick={() => ctx.zoomImage(imageUrl)}>
          <img src={imageUrl} alt={fields.prompt || tr('Изображение')} />
        </button>
      {/if}
    {:else if kind === 'output'}
      <div class="wfn-target">
        <WfIcon name="bell" size={15} />
        <span>{tr('Уведомление в программе')}</span>
      </div>
    {:else}
      <p class="wfn-hint">{kindHint(kind)}</p>
    {/if}

    {#if status === 'running' || status === 'approval'}
      <div class="wfn-live">
        <span>{statusLabel(status)}</span>
        <span class="wfn-sec">{seconds(elapsed)}</span>
        {#if step}
          <button type="button" class="wfn-link nodrag" onclick={() => ctx.stopStep(step)}>{tr('Остановить')}</button>
        {/if}
      </div>
    {/if}

    {#if hasResult}
      <div class="wfn-res nodrag nowheel" class:is-err={Boolean(errText)} class:is-open={resultOpen}>
        <div class="wfn-res-head">
          {#if attempts.length > 1}
            <span class="wfn-pager nodrag">
              <button type="button" aria-label={tr('Предыдущий результат')} disabled={pickAt <= 0} onclick={() => ctx.pick(id, pickAt - 1)}><WfIcon name="chevron" size={12} /></button>
              <span>{pickAt + 1}/{attempts.length}</span>
              <button type="button" aria-label={tr('Следующий результат')} disabled={pickAt >= attempts.length - 1} onclick={() => ctx.pick(id, pickAt + 1)}><WfIcon name="chevron" size={12} /></button>
            </span>
          {/if}
          <button type="button" class="wfn-res-toggle" aria-expanded={resultOpen} onclick={() => (resultOpen = !resultOpen)}>
            <span>{errText ? tr('Ошибка') : tr('Результат')}</span>
            <span class="wfn-sec">{seconds(elapsed)}</span>
            {#if step?.reused}<span class="wfn-reused" title={tr('Результат взят из прошлого запуска')}>{tr('из прошлого запуска')}</span>{/if}
            <WfIcon name="chevron" size={14} />
          </button>
          <button type="button" class="wfn-icon" title={copied ? tr('Скопировано') : tr('Скопировать')} aria-label={tr('Скопировать')} onclick={copy}>
            <WfIcon name={copied ? 'check' : 'copy'} size={14} />
          </button>
          {#if status !== 'running'}
            <button type="button" class="wfn-icon" title={tr('Повторить шаг')} aria-label={tr('Повторить шаг')} onclick={() => ctx.retry(step)}>
              <WfIcon name="retry" size={14} />
            </button>
          {/if}
        </div>
        {#if !(kind === 'image' && imageUrl)}<pre class="wfn-res-text">{errText || outText}</pre>{/if}
      </div>
    {/if}
  </div>

  {#if meta.outputs && kind !== 'condition'}
    <Handle type="source" position={Position.Right} id="out" class="wfn-port wfn-port--t-{portType}" />
  {/if}
  {#if kind === 'condition'}
    <Handle type="source" position={Position.Right} id="yes" style="top: 34%" class="wfn-port wfn-port--yes" />
    <Handle type="source" position={Position.Right} id="no" style="top: 66%" class="wfn-port wfn-port--no" />
    <span class="wfn-out wfn-out--yes" style="top: 34%">{tr('Да')}</span>
    <span class="wfn-out wfn-out--no" style="top: 66%">{tr('Нет')}</span>
  {/if}
</div>

<style>
  .wfn {
    --tone: var(--k-bot-sky);
    position: relative;
    width: 304px;
    border-radius: 20px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 14px 30px -18px rgba(0, 0, 0, 0.35);
    transition:
      border-color 150ms var(--k-ease-out, ease),
      box-shadow 220ms var(--k-ease-out, ease),
      opacity 220ms var(--k-ease-out, ease);
    animation: wfn-in 0.5s var(--k-ease-spring, ease) both;
  }
  .wfn.is-selected {
    border-color: var(--k-brand);
    box-shadow:
      0 0 0 3px color-mix(in oklab, var(--k-brand) 28%, transparent),
      0 18px 36px -16px color-mix(in oklab, var(--k-brand) 55%, transparent);
  }
  .wfn.is-hit {
    border-color: var(--k-bot-lemon);
    box-shadow: 0 0 0 4px color-mix(in oklab, var(--k-bot-lemon) 38%, transparent);
  }
  .wfn.is-invalid {
    border-color: var(--k-attention);
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--k-attention) 30%, transparent);
  }
  .wfn.is-soon {
    opacity: 0.72;
    border-style: dashed;
  }
  .wfn[data-status='running'] {
    border-color: var(--k-brand);
    animation:
      wfn-in 0.5s var(--k-ease-spring, ease) both,
      wfn-glow 1.8s ease-in-out infinite;
  }
  .wfn[data-status='approval'] {
    border-color: var(--k-attention);
    animation:
      wfn-in 0.5s var(--k-ease-spring, ease) both,
      wfn-glow-warn 1.8s ease-in-out infinite;
  }
  .wfn[data-status='done'] {
    border-color: color-mix(in oklab, var(--k-bot-mint) 70%, var(--k-line));
  }
  .wfn[data-status='failed'] {
    border-color: var(--k-attention);
  }
  .wfn[data-status='skipped'] {
    opacity: 0.5;
  }
  .wfn[data-status='waiting'] {
    opacity: 0.92;
  }

  .wfn-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px 8px;
  }
  .wfn-badge {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 10px;
    background: color-mix(in oklab, var(--tone) 24%, var(--k-surface));
    color: color-mix(in oklab, var(--tone) 70%, var(--k-text));
  }
  .wfn-title {
    flex: 1;
    min-width: 0;
    font: 800 16px/1.2 var(--k-font-display);
    letter-spacing: -0.01em;
  }
  .wfn-soon {
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font: 600 11px/1.4 var(--k-font-body);
  }
  .wfn-state {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
  }
  .wfn-st {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    animation: wfn-pop 0.45s var(--k-ease-spring, ease) both;
  }
  .wfn-st--done {
    background: color-mix(in oklab, var(--k-bot-mint) 26%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-mint) 80%, var(--k-text));
  }
  .wfn-st--failed {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfn-st--approval {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfn-st--stopped,
  .wfn-st--skipped,
  .wfn-st--waiting {
    background: var(--k-surface-2);
    color: var(--k-text-3);
  }
  .wfn-spin {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2.5px solid color-mix(in oklab, var(--k-brand) 25%, transparent);
    border-top-color: var(--k-brand);
    animation: wfn-rot 0.8s linear infinite;
  }

  .wfn-body {
    display: grid;
    gap: 8px;
    padding: 2px 14px 14px;
  }
  .wfn-hint,
  .wfn-next {
    margin: 0;
    font: 400 12.5px/1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfn-next {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .wfn-next b {
    font-weight: 600;
    color: var(--k-text-2);
  }
  .wfn-seg {
    display: grid;
    grid-template-columns: 1fr 1fr;
    padding: 3px;
    border-radius: 12px;
    background: var(--k-surface-2);
  }
  .wfn-seg button {
    height: 28px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: var(--k-text-2);
    font: 600 12px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfn-seg button:active {
    transform: scale(0.95);
  }
  .wfn-seg button.on {
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.18);
  }
  .wfn-sched {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--k-line);
    border-radius: 11px;
    background: transparent;
    color: var(--k-text);
    font: 600 12.5px/1 var(--k-font-body);
    cursor: pointer;
    transition: background 150ms var(--k-ease-out, ease);
  }
  .wfn-sched:hover {
    background: var(--k-surface-2);
  }
  .wfn-sched span {
    flex: 1;
    text-align: left;
  }
  .wfn-area,
  .wfn-input {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--k-line);
    border-radius: 12px;
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 400 13.5px/1.45 var(--k-font-body);
    resize: none;
    padding: 9px 11px;
    transition:
      border-color 150ms var(--k-ease-out, ease),
      box-shadow 150ms var(--k-ease-out, ease),
      background 150ms var(--k-ease-out, ease);
  }
  .wfn-area {
    field-sizing: content;
    min-height: 66px;
    max-height: 220px;
  }
  .wfn-input {
    height: 36px;
  }
  .wfn-area:focus,
  .wfn-input:focus {
    outline: none;
    border-color: var(--k-brand);
    background: var(--k-surface);
    box-shadow: var(--k-focus);
  }
  .wfn-area::placeholder,
  .wfn-input::placeholder {
    color: var(--k-text-3);
  }
  .wfn-target {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 36px;
    padding: 0 11px;
    border-radius: 12px;
    background: var(--k-surface-2);
    font: 600 13.5px/1 var(--k-font-body);
    color: var(--k-text-2);
  }
  .wfn-model {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 10px;
    border: 1px solid var(--k-line);
    border-radius: 12px;
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 600 13px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfn-model:hover {
    background: var(--k-surface-3);
  }
  .wfn-model:active {
    transform: scale(0.97);
  }
  .wfn-model span {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wfn-more {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px;
    border: 0;
    background: transparent;
    color: var(--k-text-2);
    font: 600 12px/1 var(--k-font-body);
    cursor: pointer;
  }
  .wfn-more :global(.wf-ico) {
    transition: transform 220ms var(--k-ease-out, ease);
    transform: rotate(-90deg);
  }
  .wfn-more[aria-expanded='true'] :global(.wf-ico) {
    transform: none;
  }
  .wfn-area--sys {
    min-height: 48px;
  }
  .wfn-hint--branches {
    padding-right: 36px;
    text-align: right;
  }
  .wfn-branch {
    padding: 2px 9px;
    border-radius: 999px;
    font: 700 11.5px/1.5 var(--k-font-body);
    animation: wfn-pop 0.45s var(--k-ease-spring, ease) both;
  }
  .wfn-branch--yes {
    background: color-mix(in oklab, var(--k-bot-mint) 24%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-mint) 80%, var(--k-text));
  }
  .wfn-branch--no {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfn-lock {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--k-text-3);
    cursor: pointer;
  }
  .wfn-lock:hover {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .wfn-out {
    position: absolute;
    right: 14px;
    translate: 0 -50%;
    font: 700 11.5px/1 var(--k-font-body);
    pointer-events: none;
  }
  .wfn-out--yes {
    color: color-mix(in oklab, var(--k-bot-mint) 80%, var(--k-text));
  }
  .wfn-out--no {
    color: var(--k-attention-text);
  }
  .wfn-reused {
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--k-surface-3);
    color: var(--k-text-3);
    font: 500 10.5px/1.5 var(--k-font-body);
    white-space: nowrap;
  }
  .wfn-pager {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 2px 0 6px;
    font: 600 11px/1 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .wfn-pager button {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .wfn-pager button:first-child :global(.wf-ico) {
    transform: rotate(90deg);
  }
  .wfn-pager button:last-child :global(.wf-ico) {
    transform: rotate(-90deg);
  }
  .wfn-pager button:hover:not(:disabled) {
    background: color-mix(in oklab, currentColor 14%, transparent);
  }
  .wfn-pager button:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .wfn-hidden {
    position: absolute;
    width: 0;
    height: 0;
    opacity: 0;
    pointer-events: none;
  }
  .wfn-seg--3 {
    grid-template-columns: repeat(3, 1fr);
  }
  .wfn-seg--3 button {
    font-size: 11.5px;
  }
  .wfn-drop {
    display: grid;
    justify-items: center;
    gap: 8px;
    padding: 14px 10px 12px;
    border: 1.5px dashed var(--k-line-strong);
    border-radius: 14px;
    color: var(--k-text-2);
    font: 600 13px/1.2 var(--k-font-body);
    text-align: center;
    transition:
      border-color 150ms var(--k-ease-out, ease),
      background 150ms var(--k-ease-out, ease);
  }
  .wfn-drop:hover {
    border-color: var(--k-bot-tangerine);
    background: color-mix(in oklab, var(--k-bot-tangerine) 8%, transparent);
  }
  .wfn-drop__ico {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 13px;
    background: color-mix(in oklab, var(--k-bot-tangerine) 24%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-tangerine) 78%, var(--k-text));
  }
  .wfn-drop__acts {
    display: flex;
    gap: 6px;
  }
  .wfn-drop__acts .wfn-btn {
    height: 30px;
    padding: 0 12px;
    font-size: 12.5px;
  }
  .wfn-file {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 8px;
    border-radius: 14px;
    background: var(--k-surface-2);
  }
  .wfn-file__img,
  .wfn-file__ico {
    flex: none;
    width: 46px;
    height: 46px;
    border-radius: 11px;
    object-fit: cover;
  }
  .wfn-file__ico {
    display: grid;
    place-items: center;
    background: color-mix(in oklab, var(--k-bot-tangerine) 24%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-tangerine) 78%, var(--k-text));
  }
  .wfn-file__meta {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 3px;
  }
  .wfn-file__meta b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 700 13px/1.2 var(--k-font-body);
  }
  .wfn-file__meta small {
    font: 400 11.5px/1.2 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfn-thumb {
    display: block;
    padding: 0;
    border: 0;
    border-radius: 14px;
    overflow: hidden;
    background: var(--k-surface-2);
    cursor: zoom-in;
    animation: wfn-fold 0.4s var(--k-ease-out, ease) both;
    transition: transform 320ms var(--k-ease-spring, ease);
  }
  .wfn-thumb:hover {
    transform: scale(1.02);
  }
  .wfn-thumb img {
    display: block;
    width: 100%;
    max-height: 220px;
    object-fit: cover;
  }
  .wfn.is-shake {
    animation: wfn-shake 0.42s cubic-bezier(0.36, 0.07, 0.19, 0.97) both;
  }
  :global(.wfn-port--t-image) {
    background: var(--k-bot-rose) !important;
  }
  :global(.wfn-port--t-file) {
    background: var(--k-bot-tangerine) !important;
  }
  @keyframes wfn-shake {
    10%,
    90% {
      transform: translateX(-2px);
    }
    20%,
    80% {
      transform: translateX(4px);
    }
    30%,
    50%,
    70% {
      transform: translateX(-7px);
    }
    40%,
    60% {
      transform: translateX(7px);
    }
  }
  .wfn-decide {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .wfn-btn {
    height: 34px;
    border: 0;
    border-radius: 11px;
    font: 700 13px/1 var(--k-font-body);
    cursor: pointer;
    transition:
      transform 300ms var(--k-ease-spring, ease),
      background 150ms var(--k-ease-out, ease);
  }
  .wfn-btn:active {
    transform: scale(0.95);
  }
  .wfn-btn--ok {
    background: var(--k-ink, #000);
    color: var(--k-on-ink, #fff);
  }
  .wfn-btn--ok:hover {
    background: var(--k-ink-3, #24262c);
  }
  .wfn-btn--no {
    background: var(--k-surface-2);
    color: var(--k-attention-text);
  }
  .wfn-btn--no:hover {
    background: var(--k-attention-tint);
  }
  .wfn-live {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 600 12px/1 var(--k-font-body);
    color: var(--k-brand-text);
  }
  .wfn-sec {
    font: 500 11.5px/1 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .wfn-link {
    margin-left: auto;
    padding: 4px 8px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--k-text-2);
    font: 600 12px/1 var(--k-font-body);
    cursor: pointer;
  }
  .wfn-link:hover {
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .wfn-res {
    border-radius: 12px;
    background: var(--k-surface-2);
    overflow: hidden;
    animation: wfn-fold 0.35s var(--k-ease-out, ease) both;
  }
  .wfn-res.is-err {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfn-res-head {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px 0 2px;
  }
  .wfn-res-toggle {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border: 0;
    background: transparent;
    color: inherit;
    font: 700 12px/1 var(--k-font-body);
    cursor: pointer;
  }
  .wfn-res-toggle :global(.wf-ico:last-child) {
    margin-left: auto;
    transition: transform 220ms var(--k-ease-out, ease);
  }
  .wfn-res.is-open .wfn-res-toggle :global(.wf-ico:last-child) {
    transform: rotate(180deg);
  }
  .wfn-icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: inherit;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfn-icon:hover {
    background: color-mix(in oklab, currentColor 12%, transparent);
  }
  .wfn-icon:active {
    transform: scale(0.9);
  }
  .wfn-res-text {
    margin: 0;
    padding: 0 12px 10px;
    max-height: 3.3em;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-word;
    font: 400 12.5px/1.5 var(--k-font-body);
    color: var(--k-text-2);
    transition: max-height 320ms var(--k-ease-morph, ease);
  }
  .wfn-res.is-err .wfn-res-text {
    color: inherit;
  }
  .wfn-res.is-open .wfn-res-text {
    max-height: 260px;
    overflow-y: auto;
  }

  :global(.wfn-port) {
    width: 14px !important;
    height: 14px !important;
    border-radius: 50% !important;
    border: 3px solid var(--k-surface) !important;
    background: var(--k-text-3) !important;
    box-shadow: 0 0 0 1px var(--k-line-strong);
    transition:
      scale 300ms var(--k-ease-spring, ease),
      background 150ms var(--k-ease-out, ease);
  }
  :global(.wfn-port:hover),
  :global(.wfn-port.connectingfrom) {
    scale: 1.35;
    background: var(--k-brand) !important;
  }
  :global(.wfn-port.valid) {
    background: var(--k-bot-mint) !important;
    scale: 1.5;
  }
  :global(.wfn-port--yes) {
    background: var(--k-bot-mint) !important;
  }
  :global(.wfn-port--no) {
    background: var(--k-attention) !important;
  }

  @keyframes wfn-in {
    from {
      opacity: 0;
      transform: scale(0.88) translateY(8px);
      filter: blur(6px);
    }
  }
  @keyframes wfn-pop {
    from {
      transform: scale(0.3);
      opacity: 0;
    }
  }
  @keyframes wfn-rot {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes wfn-fold {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @keyframes wfn-glow {
    0%,
    100% {
      box-shadow:
        0 0 0 3px color-mix(in oklab, var(--k-brand) 14%, transparent),
        0 14px 30px -18px rgba(0, 0, 0, 0.35);
    }
    50% {
      box-shadow:
        0 0 0 6px color-mix(in oklab, var(--k-brand) 26%, transparent),
        0 18px 36px -14px color-mix(in oklab, var(--k-brand) 55%, transparent);
    }
  }
  @keyframes wfn-glow-warn {
    0%,
    100% {
      box-shadow: 0 0 0 3px color-mix(in oklab, var(--k-attention) 14%, transparent);
    }
    50% {
      box-shadow: 0 0 0 6px color-mix(in oklab, var(--k-attention) 28%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfn,
    .wfn[data-status='running'],
    .wfn[data-status='approval'],
    .wfn-st,
    .wfn-res {
      animation: none;
    }
    .wfn-spin {
      animation-duration: 2.4s;
    }
  }
</style>
