<script>
  import { Button, Select, Slider } from './kit';
  import { tr } from '../lib/i18n.js';

  let {
    state = 'listening',
    level = 0,
    lastPhrase = '',
    lastSentence = '',
    notice = '',
    voices = [],
    voiceName = '',
    speed = 1,
    onpause,
    onfinish,
    onvoicechange,
    onspeedchange,
  } = $props();

  const states = $derived({
    listening: tr('Слушаю'),
    recognizing: tr('Распознаю'),
    thinking: tr('Думает'),
    speaking: tr('Говорит'),
    paused: tr('Пауза'),
    approval: tr('Нужно твоё решение в чате'),
  });

  function togglePause() {
    onpause?.();
  }
  function finish() {
    onfinish?.();
  }

  const voiceOptions = $derived([
    { value: '', label: tr('Голос по умолчанию') },
    ...voices.map((v) => ({ value: v.name, label: `${v.name} (${v.lang})` })),
  ]);
</script>

<div class="vc-panel">
  <div class="vc-state">{states[state] || state}</div>
  {#if notice}<div class="vc-notice">{notice}</div>{/if}
  <div class="vc-meter" style="--level:{level}"></div>
  {#if lastPhrase}<div class="vc-phrase">{lastPhrase}</div>{/if}
  {#if lastSentence}<div class="vc-sentence">{lastSentence}</div>{/if}
  <div class="vc-controls">
    <Button variant="secondary" size="sm" onclick={togglePause}>{state === 'paused' ? tr('Продолжить') : tr('Пауза')}</Button>
    <Button variant="primary" size="sm" onclick={finish}>{tr('Закончить')}</Button>
  </div>
  <div class="vc-settings">
    <Select class="vc-select" options={voiceOptions} value={voiceName} onchange={(e) => onvoicechange?.(e.currentTarget.value)} />
    <Slider class="vc-speed" min={0.8} max={1.5} step={0.05} value={speed} label={tr('Скорость')} oninput={(e) => onspeedchange?.(Number(e.currentTarget.value))} />
    <span>{Number(speed).toFixed(2)}</span>
  </div>
</div>

<style>
  .vc-panel { padding: var(--k-space-3); border-radius: var(--k-radius-md); background: var(--k-surface); color: var(--k-text); box-shadow: var(--k-shadow-1); display: flex; flex-direction: column; gap: var(--k-space-2); }
  .vc-state { font-weight: 600; }
  .vc-meter { height: 6px; background: var(--k-surface-2); border-radius: var(--k-radius-pill); position: relative; overflow: hidden; }
  .vc-meter::after { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: calc(var(--level, 0) * 100%); background: var(--k-brand); transition: width 80ms linear; }
  .vc-phrase, .vc-sentence, .vc-notice { font-size: var(--k-text-sm); color: var(--k-text-2); }
  .vc-controls { display: flex; gap: var(--k-space-2); }
  .vc-settings { display: flex; align-items: center; gap: var(--k-space-2); }
  .vc-settings :global(.vc-select) { flex: 1; }
</style>
