<script>
  // Кнопка голоса в лотке дока. Вид задаёт state (docs/design/kit.md, «Голос»), стили — классы .k-mic из /kit.css.
  // Полоски берут уровень из событий записи: нет событий — нет движения. Бесконечных анимаций нет.
  import { tr } from '../lib/i18n.js';

  import Icon from './kit/Icon.svelte';
  let { state = 'idle', level = 0, progress = 0, onpress } = $props();

  // Средние полоски выше крайних; лёгкий разброс без случайности, чтобы кадр зависел только от уровня.
  const SHAPE = [0.55, 0.85, 1, 0.8, 0.6];
  const bands = $derived(SHAPE.map((s, i) => (level <= 0.02 ? 0 : Math.min(1, level * s * (1.15 + 0.3 * Math.sin((i + 1) * level * 17))))));
  const label = $derived(
    {
      idle: tr('Голос: начать запись'),
      downloading: tr('Скачиваю модель речи'),
      loading: tr('Загружаю модель в память'),
      recording: tr('Идёт запись: нажми, чтобы закончить'),
      transcribing: tr('Распознаю речь'),
      sending: tr('Отправляю боту'),
      done: tr('Отправлено'),
      error: tr('Голос: ошибка'),
    }[state] ?? tr('Голос: начать запись'),
  );
</script>

<button type="button" class="k-mic" data-state={state} aria-label={label} title={label} style:--lvl={level} style:--progress={progress} onclick={() => onpress?.()}>
  <Icon name="mic" size={null} stroke={1.6} class="k-mic__icon" />
  <svg class="k-mic__ring" viewBox="0 0 40 40" aria-hidden="true"
    ><circle class="k-mic__ring-track" cx="20" cy="20" r="18.5" /><circle class="k-mic__ring-arc" cx="20" cy="20" r="18.5" pathLength="100" /></svg
  >
  <span class="k-mic__halo"></span>
  <span class="k-mic__disc"></span>
  <span class="k-mic__bars">
    {#each bands as b, i (i)}<i style:--b={b}></i>{/each}
  </span>
  <Icon name="tick" size={null} stroke={3.3} class="k-mic__check" />
  <Icon name="exclamation" size={null} stroke={3.6} class="k-mic__alert" />
  <span class="k-mic__rec"></span>
</button>
