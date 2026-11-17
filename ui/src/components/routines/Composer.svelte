<script>
  // Поле ввода фразы с подсветкой даты, времени и повтора на лету и чипами под полем.
  // Разбор делает код на сервере (GET /api/dates/parse), модель не зовётся.
  // Свойства общие: value, onsubmit(parsed), placeholder. Список задач возьмёт тот же компонент.
  // bots: если список дан, «+Имя» и «@Имя» открывают выбор исполнителя (MentionPopover, как в чате).
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { api as defaultApi } from '../../lib/api.js';
  import { splitByMarks, hasKind, removeKind, dateLabel } from '../../lib/composer.js';
  import MentionPopover from '../MentionPopover.svelte';

import { Icon } from '../kit';
  let {
    value = $bindable(''),
    placeholder = '',
    api = defaultApi,
    maxlength = 500,
    bots = [],
    disabled = false,
    id = '',
    describedby = '',
    debounce = 150,
    onsubmit,
    onparse,
  } = $props();

  let parsed = $state(null);
  let parsedText = $state('');
  let focused = $state(false);
  let field = $state(null);

  let timer = null;
  let seq = 0;

  const locale = () => (typeof window !== 'undefined' && window.LOCALE) || undefined;

  // Куски разбора годятся, пока текст только дописывают: иначе подсветка съехала бы, и мы её снимаем.
  const fresh = $derived(parsed && value.startsWith(parsedText) ? parsed : null);
  const parts = $derived(fresh ? splitByMarks(value, fresh.spans) : [{ text: value, kind: null }]);
  const chips = $derived.by(() => {
    if (!fresh || parsedText !== value) return [];
    const list = [];
    if (fresh.due_date && hasKind(fresh.spans, 'date')) list.push({ kind: 'date', text: dateLabel(fresh.due_date, locale()) });
    if (fresh.due_time && hasKind(fresh.spans, 'time')) list.push({ kind: 'time', text: fresh.due_time });
    if (fresh.label && hasKind(fresh.spans, 'repeat')) list.push({ kind: 'repeat', text: fresh.label });
    return list;
  });

  async function request(text) {
    const res = await api(`/api/dates/parse?text=${encodeURIComponent(text)}`);
    return res && typeof res === 'object' ? res : null;
  }

  async function run(text, mySeq) {
    try {
      const res = await request(text);
      if (mySeq !== seq) return; // пришёл ответ на уже устаревший текст
      parsed = res;
      parsedText = text;
    } catch {
      if (mySeq === seq) parsed = null;
    }
    if (mySeq === seq) onparse?.(parsed ? { text, ...parsed } : null);
  }

  function schedule(text) {
    clearTimeout(timer);
    timer = null;
    seq += 1;
    if (!text.trim()) {
      parsed = null;
      parsedText = '';
      onparse?.(null);
      return;
    }
    if (parsed && parsedText === text) return;
    const mySeq = seq;
    timer = setTimeout(() => {
      timer = null;
      run(text, mySeq);
    }, debounce);
  }

  // Запрос уходит только после изменения текста; в покое таймеров и запросов нет.
  $effect(() => {
    const text = value;
    untrack(() => schedule(text));
    return () => {
      clearTimeout(timer);
      timer = null;
    };
  });

  // Отправка: берёт свежий разбор (если он ещё не пришёл, дожидается) и отдаёт родителю.
  export async function submit() {
    const text = value.trim();
    if (!text || disabled) return;
    clearTimeout(timer);
    timer = null;
    let result = parsed && parsedText === value ? parsed : null;
    if (!result) {
      seq += 1;
      try {
        result = await request(value);
      } catch {
        result = null;
      }
    }
    const empty = { title: text, due_date: null, due_time: null, recurrence: null, label: null, spans: [], complete: false };
    onsubmit?.({ ...empty, ...(result || {}), text });
  }

  function oninput(event) {
    const clean = event.currentTarget.value.replace(/\s*[\r\n]+\s*/g, ' ');
    if (clean !== event.currentTarget.value) event.currentTarget.value = clean;
    value = clean;
  }

  function onkeydown(event) {
    if (event.defaultPrevented) return; // список «+Имя» уже забрал Enter
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    submit();
  }

  function remove(kind) {
    if (!fresh || parsedText !== value) return;
    value = removeKind(value, fresh.spans, kind);
  }

  // Дата из календаря: слова прежней даты уходят, новая дописывается как ДД.ММ.ГГГГ и подсвечивается сама.
  export function setDate(iso) {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(iso ?? ''));
    if (!m) return;
    const base = fresh && parsedText === value && hasKind(fresh.spans, 'date') ? removeKind(value, fresh.spans, 'date') : value.trim();
    value = `${base} ${m[3]}.${m[2]}.${m[1]}`.trim().slice(0, maxlength);
    field?.focus();
  }

  // Дописать текст (голос) через пробел.
  export function append(text) {
    const add = String(text ?? '').replace(/\s+/g, ' ').trim();
    if (!add) return;
    value = `${value.trim()} ${add}`.trim().slice(0, maxlength);
    field?.focus();
  }

  export function focus() {
    field?.focus();
  }

  function chipLabel(chip) {
    return tr('Убрать: {what}', { what: chip.text });
  }
</script>

<div class="cmp" class:cmp--focus={focused} class:cmp--off={disabled}>
  <div class="cmp-field">
    <div class="cmp-back" aria-hidden="true">{#each parts as part, i (i)}{#if part.kind}<mark class="cmp-mark cmp-mark--{part.kind}" data-kind={part.kind}>{part.text}</mark>{:else}{part.text}{/if}{/each}&#8203;</div>
    <textarea
      class="cmp-input"
      rows="1"
      {id}
      {placeholder}
      {disabled}
      {maxlength}
      aria-describedby={describedby || undefined}
      spellcheck="true"
      bind:this={field}
      {value}
      {oninput}
      {onkeydown}
      onfocus={() => (focused = true)}
      onblur={() => (focused = false)}
    ></textarea>
  </div>
  {#if bots.length && field}<MentionPopover {bots} input={field} />{/if}
  <div class="cmp-chips" aria-live="polite">
    {#each chips as chip (chip.kind)}
      <span class="cmp-chip cmp-chip--{chip.kind}" data-kind={chip.kind}>
        <span class="cmp-chip-text">{chip.text}</span>
        <button type="button" class="cmp-chip-x" aria-label={chipLabel(chip)} title={chipLabel(chip)} onclick={() => remove(chip.kind)}>
          <Icon name="close" size={10} stroke={3} />
        </button>
      </span>
    {/each}
  </div>
</div>

<style>
  .cmp {
    --cmp-date: var(--k-brand-tint);
    --cmp-time: var(--k-ai-tint);
    --cmp-repeat: color-mix(in srgb, var(--k-brand) 14%, var(--k-surface));
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .cmp-field {
    position: relative;
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-surface);
    transition: border-color 140ms var(--k-ease-out, ease), box-shadow 140ms var(--k-ease-out, ease);
  }
  .cmp--focus .cmp-field {
    border-color: var(--k-brand);
    box-shadow: var(--k-focus);
  }
  .cmp--off .cmp-field {
    opacity: 0.6;
  }
  .cmp-back,
  .cmp-input {
    box-sizing: border-box;
    margin: 0;
    padding: 10px 14px;
    border: 0;
    font: 400 var(--k-text-md, 15px) / 1.45 var(--k-font-body, system-ui);
    letter-spacing: normal;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: normal;
  }
  .cmp-back {
    min-height: 2.9em;
    color: transparent;
    pointer-events: none;
  }
  .cmp-input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    resize: none;
    overflow: hidden;
    outline: none;
    background: transparent;
    color: var(--k-text);
  }
  .cmp-input::placeholder {
    color: var(--k-text-3);
  }
  .cmp-mark {
    border-radius: 5px;
    color: transparent;
    box-shadow: 0 0 0 2px var(--cmp-bg);
    background: var(--cmp-bg);
  }
  .cmp-mark--date {
    --cmp-bg: var(--cmp-date);
  }
  .cmp-mark--time {
    --cmp-bg: var(--cmp-time);
  }
  .cmp-mark--repeat {
    --cmp-bg: var(--cmp-repeat);
  }
  .cmp-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
    min-height: 0;
  }
  .cmp-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px 4px 3px 10px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--cmp-bg);
    color: var(--k-text);
    font: 600 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    animation: cmp-in 140ms var(--k-ease-out, ease) both;
  }
  .cmp-chip--date {
    --cmp-bg: var(--cmp-date);
  }
  .cmp-chip--time {
    --cmp-bg: var(--cmp-time);
  }
  .cmp-chip--repeat {
    --cmp-bg: var(--cmp-repeat);
  }
  .cmp-chip-x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
  }
  .cmp-chip-x:hover {
    background: color-mix(in srgb, var(--k-text) 12%, transparent);
    color: var(--k-text);
  }
  .cmp-chip-x:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  @keyframes cmp-in {
    from {
      opacity: 0;
      transform: scale(0.92);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cmp-chip {
      animation: none;
    }
    .cmp-field {
      transition: none;
    }
  }
</style>
