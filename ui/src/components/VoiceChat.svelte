<script>
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
</script>

<div class="vc-panel">
  <div class="vc-state">{states[state] || state}</div>
  {#if notice}<div class="vc-notice">{notice}</div>{/if}
  <div class="vc-meter" style="--level:{level}"></div>
  {#if lastPhrase}<div class="vc-phrase">{lastPhrase}</div>{/if}
  {#if lastSentence}<div class="vc-sentence">{lastSentence}</div>{/if}
  <div class="vc-controls">
    <button class="vc-btn" onclick={togglePause}>{state === 'paused' ? tr('Продолжить') : tr('Пауза')}</button>
    <button class="vc-btn" onclick={finish}>{tr('Закончить')}</button>
  </div>
  <div class="vc-settings">
    <select value={voiceName} onchange={(e) => onvoicechange?.(e.target.value)} class="vc-select">
      <option value="">{tr('Голос по умолчанию')}</option>
      {#each voices as v}
        <option value={v.name}>{v.name} ({v.lang})</option>
      {/each}
    </select>
    <input type="range" min="0.8" max="1.5" step="0.05" value={speed}
      oninput={(e) => onspeedchange?.(Number(e.target.value))} />
    <span>{Number(speed).toFixed(2)}</span>
  </div>
</div>

<style>
  .vc-panel { padding: 12px; border-radius: 12px; background: var(--color-bg-surface, #fff); color: var(--color-text-primary, #111); display: flex; flex-direction: column; gap: 8px; }
  .vc-state { font-weight: 600; }
  .vc-meter { height: 6px; background: var(--color-bg-surface-soft, #eee); border-radius: 3px; position: relative; overflow: hidden; }
  .vc-meter::after { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: calc(var(--level, 0) * 100%); background: var(--color-accent, #4f46e5); transition: width 80ms linear; }
  .vc-phrase, .vc-sentence, .vc-notice { font-size: 13px; color: var(--color-text-secondary, #555); }
  .vc-controls { display: flex; gap: 8px; }
  .vc-btn { padding: 6px 10px; border-radius: 8px; border: 1px solid var(--color-border-default, #ccc); background: transparent; cursor: pointer; }
  .vc-settings { display: flex; align-items: center; gap: 8px; }
  .vc-select { flex: 1; }
</style>