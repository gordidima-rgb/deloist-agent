<script>
  // Карточка шага: триггер, промпт, бот, одобрение, вывод (и «скоро»: модель, условие).
  // Свойства приходят от Svelte Flow; данные меняем через модель холста из контекста.
  import { getContext } from 'svelte';
  import { Handle, Position } from '@xyflow/svelte';
  import { tr } from '../../lib/i18n.js';
  import { KINDS, isEnabled, stepSeconds, outTypeOf } from '../../lib/workflows/graph.js';
  import { WF } from './ctx.js';
  import { kindTitle, kindHint, statusLabel, seconds } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';
  import { Button, IconButton, TextArea, TextInput, ChoiceGrid } from '../kit';
  import BotPicker from './BotPicker.svelte';
  import TriggerFields from './TriggerFields.svelte';
  import OutputFields from './OutputFields.svelte';
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
  const preview = $derived(ctx.assetPreview(id) || (kind === 'asset' && String(fields.mime || '').startsWith('image/') && fields.path ? ctx.fileUrl(fields.path) : ''));
  const imagePath = $derived((Array.isArray(step?.files) && step.files[0]) || outText.trim());
  const imageUrl = $derived(kind === 'image' && step?.status === 'done' && imagePath ? ctx.fileUrl(imagePath) : '');
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
  const outText = $derived(typeof step?.output === 'string' ? step.output : step?.output == null ? '' : JSON.stringify(step.output, null, 2));
  const errText = $derived(step?.error ? String(step.error) : '');
  const elapsed = $derived(step ? stepSeconds(step, ctx.now) : 0);
  const hasResult = $derived(Boolean(step) && (status === 'done' || status === 'failed' || status === 'stopped') && Boolean(outText || errText));

  let resultOpen = $state(false);
  let copied = $state(false);

  const set = (key, value) => ctx.model.setField(id, key, value);

  async function copy() {
    try {
      await navigator.clipboard.writeText(errText || outText);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {}
  }

</script>

<div
  class="wfn"
  class:is-selected={selected}
  class:is-soon={!enabled}
  class:is-invalid={invalid}
  class:is-hit={hit}
  class:is-shake={shaking}
  class:is-leaving={data.leaving}
  class:is-flash={data.flash}
  data-kind={kind}
  data-status={status}
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
      <IconButton class="wfn-lock nodrag" name="wf-lock" size="sm" label={tr('Разблокировать')} onclick={() => ctx.model.toggleLock(id)} />
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
      <TriggerFields {id} {fields} schedule={data.schedule} />
    {:else if kind === 'prompt'}
      <TextArea class="wfn-area nodrag nowheel" rows={3} autosize={false} placeholder={tr('Что сказать следующему шагу')} value={fields.text || ''} oninput={(e) => set('text', e.currentTarget.value)} />
    {:else if kind === 'bot'}
      <BotPicker bots={ctx.bots} value={fields.bot_id || ''} onchange={(v) => ctx.model.setFields(id, { bot_id: v })} />
      <TextArea class="wfn-area nodrag nowheel" rows={3} autosize={false} placeholder={tr('Что поручить боту')} value={fields.task || ''} oninput={(e) => set('task', e.currentTarget.value)} />
    {:else if kind === 'approval'}
      <TextInput class="wfn-input nodrag" placeholder={tr('Что спросить у вас')} value={fields.question || ''} oninput={(e) => set('question', e.currentTarget.value)} />
      {#if status === 'approval' && step}
        <div class="wfn-decide nodrag">
          <Button variant="primary" size="sm" onclick={() => ctx.decide(step, true)}>{tr('Одобрить')}</Button>
          <Button variant="danger" size="sm" onclick={() => ctx.decide(step, false)}>{tr('Отклонить')}</Button>
        </div>
      {/if}
    {:else if kind === 'llm'}
      <Button class="wfn-model nodrag" variant="ghost" iconName="wf-llm" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </Button>
      <TextArea class="wfn-area nodrag nowheel" rows={3} autosize={false} placeholder={tr('Что спросить у модели')} value={fields.prompt || ''} oninput={(e) => set('prompt', e.currentTarget.value)} />
      <Button class="wfn-more nodrag" variant="ghost" size="sm" aria-expanded={systemOpen || Boolean(fields.system)} onclick={() => (systemOpen = !systemOpen)}>
        <WfIcon name="chevron" size={13} />
        <span>{tr('Системная инструкция')}</span>
      </Button>
      {#if systemOpen || fields.system}
        <TextArea class="wfn-area wfn-area--sys nodrag nowheel" rows={2} autosize={false} placeholder={tr('Роль и правила для модели')} value={fields.system || ''} oninput={(e) => set('system', e.currentTarget.value)} />
      {/if}
    {:else if kind === 'condition'}
      <TextArea class="wfn-area nodrag nowheel" rows={2} autosize={false} placeholder={tr('Условие: например, в ответе есть ошибка')} value={fields.rule || ''} oninput={(e) => set('rule', e.currentTarget.value)} />
      <Button class="wfn-model nodrag" variant="ghost" iconName="wf-llm" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </Button>
    {:else if kind === 'asset'}
      {#if fields.name || fields.text || textMode}
        <div class="wfn-file">
          {#if preview}
            <button type="button" class="wfn-file__zoom nodrag" title={tr('Открыть крупно')} aria-label={tr('Открыть крупно')} onclick={() => ctx.zoomImage(preview)}><img class="wfn-file__img" src={preview} alt={fields.name} /></button>
          {:else}
            <span class="wfn-file__ico"><WfIcon name={fields.name ? 'asset' : 'text'} size={20} /></span>
          {/if}
          <span class="wfn-file__meta">
            <b>{fields.name || tr('Текст')}</b>
            <small>{[fields.mime, sizeText].filter(Boolean).join(' · ') || tr('Готовый текст')}</small>
          </span>
          <IconButton class="nodrag" name="close" size="sm" label={tr('Убрать')} onclick={() => { textMode = false; ctx.clearAsset(id); }} />
        </div>
        {#if !fields.name || fields.text}
          <TextArea class="wfn-area nodrag nowheel" rows={3} autosize={false} placeholder={tr('Вставьте текст')} value={fields.text || ''} oninput={(e) => set('text', e.currentTarget.value)} />
        {:else if !fields.path}
          <p class="wfn-hint">{tr('Файл пока только на этом экране: на сервер он попадёт вместе с запуском.')}</p>
        {/if}
      {:else}
        <div class="wfn-drop nodrag" role="presentation">
          <span class="wfn-drop__ico"><WfIcon name="asset" size={22} /></span>
          <b>{tr('Перетащите файл сюда')}</b>
          <span class="wfn-drop__acts">
            <Button variant="secondary" size="sm" onclick={() => fileInput?.click()}>{tr('Выбрать файл')}</Button>
            <Button variant="ghost" size="sm" onclick={() => (textMode = true)}>{tr('Вставить текст')}</Button>
          </span>
        </div>
      {/if}
      <input bind:this={fileInput} class="wfn-hidden" type="file" onchange={pickedFile} tabindex="-1" aria-hidden="true" />
    {:else if kind === 'image'}
      <Button class="wfn-model nodrag" variant="ghost" iconName="wf-image" onclick={(e) => ctx.pickModel(id, e.currentTarget)}>
        <span>{modelText}</span>
        <WfIcon name="chevron" size={14} />
      </Button>
      <TextArea class="wfn-area nodrag nowheel" rows={3} autosize={false} placeholder={tr('Что нарисовать')} value={fields.prompt || ''} oninput={(e) => set('prompt', e.currentTarget.value)} />
      <ChoiceGrid class="nodrag" items={IMAGE_SIZES.map((sz) => ({ id: sz, label: sz.replace('x', '×') }))} value={fields.size || '1024x1024'} label={tr('Размер картинки')} onchange={(sz) => set('size', sz)} />
      {#if imageUrl}
        <button type="button" class="wfn-thumb nodrag" title={tr('Открыть крупно')} onclick={() => ctx.zoomImage(imageUrl)}>
          <img src={imageUrl} alt={fields.prompt || tr('Изображение')} />
        </button>
      {/if}
    {:else if kind === 'output'}
      <OutputFields {id} {fields} />
    {:else}
      <p class="wfn-hint">{kindHint(kind)}</p>
    {/if}

    {#if status === 'running' || status === 'approval'}
      <div class="wfn-live">
        <span>{statusLabel(status)}</span>
        <span class="wfn-sec">{seconds(elapsed)}</span>
        {#if step}
          <Button class="wfn-link nodrag" variant="ghost" size="sm" onclick={() => ctx.stopStep(step)}>{tr('Остановить')}</Button>
        {/if}
      </div>
    {/if}

    {#if hasResult}
      <div class="wfn-res nodrag nowheel" class:is-err={Boolean(errText)} class:is-open={resultOpen}>
        <div class="wfn-res-head">
          {#if attempts.length > 1}
            <span class="wfn-pager nodrag">
              <IconButton class="wfn-prev" name="chevron-left" size="sm" label={tr('Предыдущий результат')} disabled={pickAt <= 0} onclick={() => ctx.pick(id, pickAt - 1)} />
              <span>{pickAt + 1}/{attempts.length}</span>
              <IconButton class="wfn-next" name="chevron-right" size="sm" label={tr('Следующий результат')} disabled={pickAt >= attempts.length - 1} onclick={() => ctx.pick(id, pickAt + 1)} />
            </span>
          {/if}
          <Button class="wfn-res-toggle" variant="ghost" size="sm" aria-expanded={resultOpen} onclick={() => (resultOpen = !resultOpen)}>
            <span>{errText ? tr('Ошибка') : tr('Результат')}</span>
            <span class="wfn-sec">{seconds(elapsed)}</span>
            {#if step?.reused}<span class="wfn-reused" title={tr('Результат взят из прошлого запуска')}>{tr('из прошлого запуска')}</span>{/if}
            <WfIcon name="chevron" size={14} />
          </Button>
          <IconButton name={copied ? 'check' : 'wf-copy'} size="sm" label={tr('Скопировать')} title={copied ? tr('Скопировано') : tr('Скопировать')} onclick={copy} />
          {#if status !== 'running'}
            <IconButton name="wf-retry" size="sm" label={tr('Повторить шаг')} onclick={() => ctx.retry(step)} />
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
    position: relative;
    width: 304px;
    border-radius: var(--k-radius-lg);
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow:
      0 0 0 1px var(--k-line-strong),
      0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
    transition:
      box-shadow var(--k-dur-ui) var(--k-ease-out),
      opacity var(--k-dur-base) var(--k-ease-out);
    animation: wfn-in var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wfn.is-selected {
    box-shadow:
      0 0 0 1.5px var(--k-brand),
      0 0 26px -6px color-mix(in oklab, var(--k-brand) 70%, transparent),
      0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
  }
  .wfn.is-hit {
    box-shadow:
      0 0 0 2px var(--k-brand),
      0 0 0 6px color-mix(in oklab, var(--k-brand) 28%, transparent);
  }
  .wfn.is-invalid {
    box-shadow:
      0 0 0 1.5px var(--k-danger),
      0 0 0 5px color-mix(in oklab, var(--k-danger) 22%, transparent);
  }
  .wfn.is-leaving {
    pointer-events: none;
    animation: wfn-out var(--k-dur-exit) var(--k-ease-in) both;
  }
  .wfn.is-flash {
    animation:
      wfn-in var(--k-dur-appear) var(--k-ease-spring) both,
      wfn-flash 1.5s var(--k-ease-out) both;
  }
  .wfn.is-soon {
    opacity: 0.72;
  }
  .wfn[data-status='running'],
  .wfn[data-status='approval'] {
    box-shadow:
      0 0 0 1.5px var(--k-brand),
      0 0 26px -6px color-mix(in oklab, var(--k-brand) 70%, transparent),
      0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
    animation:
      wfn-in var(--k-dur-appear) var(--k-ease-spring) both,
      wfn-glow 1.8s ease-in-out infinite;
  }
  .wfn[data-status='failed'] {
    box-shadow:
      0 0 0 1.5px var(--k-danger),
      0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
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
    gap: var(--k-space-3);
    padding: var(--k-space-3) var(--k-space-4) var(--k-space-2);
  }
  .wfn-badge {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--k-radius-sm);
    background: var(--k-brand-tint);
    color: var(--k-brand);
    box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--k-brand) 35%, transparent);
  }
  .wfn-title {
    flex: 1;
    min-width: 0;
    font: 700 var(--k-text-md) / 1.2 var(--k-font-display);
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
    animation: wfn-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wfn-st--done {
    background: var(--k-surface-2);
    color: var(--k-brand);
    box-shadow: inset 0 0 0 1px var(--k-line-strong);
  }
  .wfn-st--failed {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .wfn-st--approval {
    background: var(--k-btn-fill);
    color: var(--k-on-brand);
    box-shadow: var(--k-btn-rim);
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
    gap: var(--k-space-2);
    padding: var(--k-space-0) var(--k-space-4) var(--k-space-4);
  }
  .wfn-hint {
    margin: 0;
    font: 400 var(--k-text-xs) / 1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfn-body :global(.wfn-area) {
    min-height: 66px;
    max-height: 220px;
    font-size: var(--k-text-sm);
    field-sizing: content;
  }
  .wfn-body :global(.wfn-area--sys) {
    min-height: 48px;
  }
  .wfn-body :global(.wfn-input) {
    height: 36px;
    font-size: var(--k-text-sm);
  }
  .wfn-body :global(.wfn-model) {
    justify-content: flex-start;
    width: 100%;
    background: var(--k-surface-2);
    border: 1px solid var(--k-line);
    color: var(--k-text);
  }
  .wfn-body :global(.wfn-model span) {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wfn-body :global(.wfn-more) {
    justify-self: start;
    color: var(--k-text-2);
  }
  .wfn-body :global(.wfn-more .wf-ico) {
    transition: transform var(--k-dur-base) var(--k-ease-out);
    transform: rotate(-90deg);
  }
  .wfn-body :global(.wfn-more[aria-expanded='true'] .wf-ico) {
    transform: none;
  }
  .wfn-branch {
    padding: 2px var(--k-space-2);
    border-radius: var(--k-radius-pill);
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
    font: 700 var(--k-text-xs) / 1.5 var(--k-font-body);
    animation: wfn-pop var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .wfn-branch--no {
    background: var(--k-surface-2);
    color: var(--k-text-2);
  }
  .wfn-out {
    position: absolute;
    right: var(--k-space-4);
    translate: 0 -50%;
    font: 700 var(--k-text-xs) / 1 var(--k-font-body);
    pointer-events: none;
  }
  .wfn-out--yes {
    color: var(--k-brand-text);
  }
  .wfn-out--no {
    color: var(--k-text-3);
  }
  .wfn-reused {
    padding: 1px var(--k-space-2);
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-3);
    color: var(--k-text-3);
    font: 600 var(--k-text-micro) / 1.5 var(--k-font-body);
    white-space: nowrap;
  }
  .wfn-pager {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-0);
    padding: 0 var(--k-space-0) 0 6px;
    font: 600 var(--k-text-micro) / 1 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .wfn-hidden {
    position: absolute;
    width: 0;
    height: 0;
    opacity: 0;
    pointer-events: none;
  }
  .wfn-drop {
    display: grid;
    justify-items: center;
    gap: var(--k-space-2);
    padding: var(--k-space-3) var(--k-space-3) var(--k-space-3);
    border: 1.5px dashed var(--k-line-strong);
    border-radius: var(--k-radius-md);
    color: var(--k-text-2);
    font: 600 var(--k-text-sm) / 1.2 var(--k-font-body);
    text-align: center;
    transition:
      border-color var(--k-dur-micro) var(--k-ease-out),
      background var(--k-dur-micro) var(--k-ease-out);
  }
  .wfn-drop:hover {
    border-color: var(--k-brand);
    background: var(--k-brand-tint);
  }
  .wfn-drop__ico {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: var(--k-radius-md);
    background: var(--k-brand-tint);
    color: var(--k-brand);
  }
  .wfn-drop__acts {
    display: flex;
    gap: var(--k-space-2);
  }
  .wfn-file {
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    padding: var(--k-space-2);
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
  }
  .wfn-file__zoom {
    flex: none;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-sm);
    background: none;
    cursor: zoom-in;
    transition: transform var(--k-dur-ui) var(--k-ease-spring);
  }
  .wfn-file__zoom:hover {
    transform: scale(1.06);
  }
  .wfn-file__img,
  .wfn-file__ico {
    flex: none;
    width: 46px;
    height: 46px;
    border-radius: var(--k-radius-sm);
    object-fit: cover;
  }
  .wfn-file__ico {
    display: grid;
    place-items: center;
    background: var(--k-brand-tint);
    color: var(--k-brand);
  }
  .wfn-file__meta {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: var(--k-space-1);
  }
  .wfn-file__meta b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 700 var(--k-text-sm) / 1.2 var(--k-font-body);
  }
  .wfn-file__meta small {
    font: 400 var(--k-text-xs) / 1.2 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfn-thumb {
    display: block;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-md);
    overflow: hidden;
    background: var(--k-surface-2);
    cursor: zoom-in;
    animation: wfn-fold var(--k-dur-appear) var(--k-ease-out) both;
    transition: transform var(--k-dur-ui) var(--k-ease-spring);
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
    gap: var(--k-space-2);
  }
  .wfn-live {
    display: flex;
    align-items: center;
    gap: var(--k-space-2);
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    color: var(--k-brand-text);
  }
  .wfn-sec {
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .wfn-live :global(.wfn-link) {
    margin-left: auto;
  }
  .wfn-res {
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
    overflow: hidden;
    animation: wfn-fold var(--k-dur-appear) var(--k-ease-out) both;
  }
  .wfn-res.is-err {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .wfn-res-head {
    display: flex;
    align-items: center;
    gap: var(--k-space-0);
    padding: var(--k-space-0) var(--k-space-1) 0 var(--k-space-0);
  }
  .wfn-res-head :global(.wfn-res-toggle) {
    flex: 1;
    justify-content: flex-start;
    color: inherit;
  }
  .wfn-res-head :global(.wfn-res-toggle .wf-ico:last-child) {
    margin-left: auto;
    transition: transform var(--k-dur-base) var(--k-ease-out);
  }
  .wfn-res.is-open .wfn-res-head :global(.wfn-res-toggle .wf-ico:last-child) {
    transform: rotate(180deg);
  }
  .wfn-res-text {
    margin: 0;
    padding: 0 var(--k-space-3) var(--k-space-3);
    max-height: 3.3em;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-word;
    font: 400 var(--k-text-xs) / 1.5 var(--k-font-body);
    color: var(--k-text-2);
    transition: max-height var(--k-dur-morph) var(--k-ease-morph);
  }
  .wfn-res.is-err .wfn-res-text {
    color: inherit;
  }
  .wfn-res.is-open .wfn-res-text {
    max-height: 260px;
    overflow-y: auto;
  }
  :global(.wfn-port) {
    width: 12px !important;
    height: 12px !important;
    border: 0 !important;
    border-radius: 50% !important;
    background: var(--k-canvas) !important;
    box-shadow:
      inset 0 0 0 2.5px var(--k-brand),
      0 0 8px color-mix(in oklab, var(--k-brand) 60%, transparent);
    transition: scale var(--k-dur-ui) var(--k-ease-spring);
  }
  :global(.wfn-port:hover),
  :global(.wfn-port.connectingfrom),
  :global(.wfn-port.valid) {
    scale: 1.4;
    background: var(--k-brand) !important;
  }
  @keyframes wfn-out {
    to {
      opacity: 0;
      transform: scale(0.88);
      filter: blur(5px);
    }
  }
  @keyframes wfn-flash {
    0% {
      box-shadow: 0 0 0 0 color-mix(in oklab, var(--k-brand) 60%, transparent);
    }
    30% {
      box-shadow: 0 0 0 7px color-mix(in oklab, var(--k-brand) 35%, transparent);
    }
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
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
        0 0 0 1.5px var(--k-brand),
        0 0 14px -6px color-mix(in oklab, var(--k-brand) 60%, transparent),
        0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
    }
    50% {
      box-shadow:
        0 0 0 2px var(--k-brand),
        0 0 34px -4px color-mix(in oklab, var(--k-brand) 80%, transparent),
        0 16px 34px -18px color-mix(in oklab, var(--k-ink) 45%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfn,
    .wfn.is-leaving,
    .wfn.is-flash,
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
