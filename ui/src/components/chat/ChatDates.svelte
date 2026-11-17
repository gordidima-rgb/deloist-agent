<script>
  // Подсветка сроков в поле ввода чата: то же, что в планировщике (Composer.svelte), но поле остаётся
  // обычным <textarea id="input"> из web/app.js: многострочное, с вложениями и горячими клавишами.
  // Компонент кладёт подсветку позади поля и чипы под ним; текст сообщения не меняется.
  // Разбор делает код на сервере (GET /api/dates/parse), модель не зовётся.
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { api as defaultApi } from '../../lib/api.js';
  import { splitByMarks, hasKind, removeKind, dateLabel } from '../../lib/composer.js';

import { Icon } from '../kit';
  let { input = null, chipsHost = null, api = defaultApi, debounce = 200 } = $props();

  let value = $state('');
  let parsed = $state(null);
  let parsedText = $state('');
  let scroll = $state(0);
  let boxStyle = $state('');
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

  async function run(text, mySeq) {
    try {
      const res = await api(`/api/dates/parse?text=${encodeURIComponent(text)}`);
      if (mySeq !== seq) return; // ответ на уже устаревший текст
      parsed = res && typeof res === 'object' ? res : null;
      parsedText = text;
    } catch {
      if (mySeq === seq) parsed = null;
    }
  }

  function schedule(text) {
    clearTimeout(timer);
    timer = null;
    seq += 1;
    if (!text.trim()) {
      parsed = null;
      parsedText = '';
      return;
    }
    if (parsed && parsedText === text) return;
    const mySeq = seq;
    timer = setTimeout(() => {
      timer = null;
      run(text, mySeq);
    }, debounce);
  }

  // Подсветка должна ложиться на буквы поля: берём у него шрифт, отступы и ширину полосы прокрутки.
  function copyLook() {
    if (!input) return;
    const cs = getComputedStyle(input);
    const bar = input.offsetWidth - input.clientWidth - parseFloat(cs.borderLeftWidth) - parseFloat(cs.borderRightWidth);
    boxStyle =
      `font:${cs.font};letter-spacing:${cs.letterSpacing};line-height:${cs.lineHeight};` +
      `padding:${cs.paddingTop} ${parseFloat(cs.paddingRight) + Math.max(0, bar)}px ${cs.paddingBottom} ${cs.paddingLeft};` +
      `border:${cs.borderTopWidth} solid transparent;`;
  }

  // Поле меняют и кодом (отправка очищает его без события input): app.js зовёт autosize → 'cd-sync'.
  function sync() {
    if (!input) return;
    if (input.value !== value) value = input.value;
    scroll = input.scrollTop;
    copyLook();
  }

  $effect(() => {
    if (!input) return;
    sync();
    const onScroll = () => (scroll = input.scrollTop);
    input.addEventListener('input', sync);
    input.addEventListener('cd-sync', sync);
    input.addEventListener('scroll', onScroll);
    return () => {
      input.removeEventListener('input', sync);
      input.removeEventListener('cd-sync', sync);
      input.removeEventListener('scroll', onScroll);
    };
  });

  // Запрос уходит только после изменения текста; в покое таймеров и запросов нет.
  $effect(() => {
    const text = value;
    untrack(() => schedule(text));
    return () => {
      clearTimeout(timer);
      timer = null;
    };
  });

  function remove(kind) {
    if (!input || !fresh || parsedText !== value) return;
    input.value = removeKind(value, fresh.spans, kind, { keepLines: true });
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.focus();
  }

  const chipLabel = (chip) => tr('Убрать: {what}', { what: chip.text });

  // Ряд чипов живёт в своём месте разметки, а не внутри подсветки.
  function moveTo(host) {
    return (node) => {
      if (!host) return;
      host.appendChild(node);
      return () => node.remove();
    };
  }
  const chipsAttach = $derived(moveTo(chipsHost));
</script>

<div class="cd-back" style={boxStyle} aria-hidden="true"><div class="cd-text" style:transform="translateY({-scroll}px)">{#each parts as part, i (i)}{#if part.kind}<mark class="cd-mark cd-mark--{part.kind}" data-kind={part.kind}>{part.text}</mark>{:else}{part.text}{/if}{/each}&#8203;</div></div>

<div class="cd-chips" aria-live="polite" {@attach chipsAttach}>
  {#each chips as chip (chip.kind)}
    <span class="cd-chip cd-chip--{chip.kind}" data-kind={chip.kind}>
      <span class="cd-chip-text">{chip.text}</span>
      <button type="button" class="cd-chip-x" aria-label={chipLabel(chip)} title={chipLabel(chip)} onclick={() => remove(chip.kind)}>
        <Icon name="close" size={10} stroke={3} />
      </button>
    </span>
  {/each}
</div>

<style>
  .cd-back {
    --cd-date: var(--k-brand-tint, #e7efff);
    --cd-time: var(--k-ai-tint, #eeebff);
    --cd-repeat: color-mix(in srgb, var(--k-bot-mint, #3ddc97) 26%, var(--k-surface, #fff));
    position: absolute;
    inset: 0;
    box-sizing: border-box;
    overflow: hidden;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: normal;
    color: transparent;
    pointer-events: none;
    z-index: 0;
  }
  .cd-mark {
    border-radius: 5px;
    color: transparent;
    background: var(--cd-bg);
    box-shadow: 0 0 0 2px var(--cd-bg);
  }
  .cd-mark--date {
    --cd-bg: var(--cd-date);
  }
  .cd-mark--time {
    --cd-bg: var(--cd-time);
  }
  .cd-mark--repeat {
    --cd-bg: var(--cd-repeat);
  }
  .cd-chips {
    --cd-date: var(--k-brand-tint, #e7efff);
    --cd-time: var(--k-ai-tint, #eeebff);
    --cd-repeat: color-mix(in srgb, var(--k-bot-mint, #3ddc97) 26%, var(--k-surface, #fff));
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
    min-height: 0;
  }
  .cd-chips:empty {
    display: none;
  }
  .cd-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px 4px 3px 10px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--cd-bg);
    color: var(--k-text, #262931);
    font: 600 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    animation: cd-in 140ms var(--k-ease-out, ease) both;
  }
  .cd-chip--date {
    --cd-bg: var(--cd-date);
  }
  .cd-chip--time {
    --cd-bg: var(--cd-time);
  }
  .cd-chip--repeat {
    --cd-bg: var(--cd-repeat);
  }
  .cd-chip-x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: var(--k-text-2, #5c5f66);
    cursor: pointer;
  }
  .cd-chip-x:hover {
    background: color-mix(in srgb, var(--k-text, #262931) 12%, transparent);
    color: var(--k-text, #262931);
  }
  .cd-chip-x:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  @keyframes cd-in {
    from {
      opacity: 0;
      transform: scale(0.92);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cd-chip {
      animation: none;
    }
  }
</style>
