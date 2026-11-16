<script>
  // Расписание триггера: вид, время и дополнительные поля под вид. Каждое изменение — onchange(новое расписание).
  import { tr } from '../../lib/i18n.js';
  import { SCHEDULE_KINDS, scheduleDefaults, normalizeSchedule } from '../../lib/workflows/graph.js';
  import { scheduleKindLabel, weekdayName } from './Labels.svelte';

  let { schedule, onchange } = $props();

  const s = $derived(schedule || scheduleDefaults('daily'));
  const needsTime = $derived(['daily', 'weekdays', 'weekly', 'monthly', 'once'].includes(s.kind));

  function patch(next) {
    onchange?.(normalizeSchedule({ ...s, ...next }));
  }

  function setKind(kind) {
    const base = scheduleDefaults(kind);
    // Время и день сохраняем, если новый вид их тоже знает.
    if (base.time && s.time) base.time = s.time;
    onchange?.(base);
  }
</script>

<div class="wf-sched nodrag nowheel nopan">
  <label class="wf-sched__row">
    <span>{tr('Как часто')}</span>
    <select value={s.kind} onchange={(e) => setKind(e.currentTarget.value)}>
      {#each SCHEDULE_KINDS as k (k)}
        <option value={k}>{scheduleKindLabel(k)}</option>
      {/each}
    </select>
  </label>

  {#if s.kind === 'hourly'}
    <label class="wf-sched__row">
      <span>{tr('Минута')}</span>
      <input type="number" min="0" max="59" value={s.minute ?? 0} onchange={(e) => patch({ minute: Number(e.currentTarget.value) })} />
    </label>
  {/if}

  {#if s.kind === 'weekly'}
    <label class="wf-sched__row">
      <span>{tr('День недели')}</span>
      <select value={String(s.weekday || 1)} onchange={(e) => patch({ weekday: Number(e.currentTarget.value) })}>
        {#each [1, 2, 3, 4, 5, 6, 7] as d (d)}
          <option value={String(d)}>{weekdayName(d)}</option>
        {/each}
      </select>
    </label>
  {/if}

  {#if s.kind === 'monthly'}
    <label class="wf-sched__row">
      <span>{tr('Число месяца')}</span>
      <input type="number" min="1" max="31" value={s.day || 1} onchange={(e) => patch({ day: Number(e.currentTarget.value) })} />
    </label>
  {/if}

  {#if s.kind === 'once'}
    <label class="wf-sched__row">
      <span>{tr('Дата')}</span>
      <input type="date" value={s.date || ''} onchange={(e) => patch({ date: e.currentTarget.value })} />
    </label>
  {/if}

  {#if needsTime}
    <label class="wf-sched__row">
      <span>{tr('Время')}</span>
      <input type="time" value={s.time || '09:00'} onchange={(e) => patch({ time: e.currentTarget.value || '09:00' })} />
    </label>
  {/if}

  {#if s.kind === 'interval'}
    <div class="wf-sched__row">
      <span>{tr('Каждые')}</span>
      <div class="wf-sched__pair">
        <input type="number" min="1" value={s.amount || 30} onchange={(e) => patch({ amount: Number(e.currentTarget.value) })} />
        <select value={s.unit || 'minutes'} onchange={(e) => patch({ unit: e.currentTarget.value })}>
          <option value="minutes">{tr('минут')}</option>
          <option value="hours">{tr('часов')}</option>
          <option value="days">{tr('дней')}</option>
        </select>
      </div>
    </div>
    <p class="wf-sched__hint">{tr('Не чаще раза в 3 минуты')}</p>
  {/if}
</div>

<style>
  .wf-sched {
    display: grid;
    gap: 6px;
    padding: 10px;
    border-radius: 12px;
    background: var(--k-surface-2);
    animation: wf-fold 0.3s var(--k-ease-out, ease) both;
  }
  .wf-sched__row {
    display: grid;
    grid-template-columns: 92px 1fr;
    align-items: center;
    gap: 8px;
    font: 500 12px/1.3 var(--k-font-body);
    color: var(--k-text-2);
  }
  .wf-sched__pair {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 6px;
  }
  select,
  input {
    min-width: 0;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--k-line);
    border-radius: 10px;
    background: var(--k-surface);
    color: var(--k-text);
    font: 500 13px/1 var(--k-font-body);
    transition: border-color 150ms var(--k-ease-out, ease), box-shadow 150ms var(--k-ease-out, ease);
  }
  select:focus,
  input:focus {
    outline: none;
    border-color: var(--k-brand);
    box-shadow: var(--k-focus);
  }
  .wf-sched__hint {
    margin: 0;
    font: 400 11px/1.3 var(--k-font-body);
    color: var(--k-text-3);
  }
  @keyframes wf-fold {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.98);
      filter: blur(4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-sched {
      animation: none;
    }
  }
</style>
