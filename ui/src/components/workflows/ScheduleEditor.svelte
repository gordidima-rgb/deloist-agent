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
  import { Select, TextInput } from '../kit';
</script>

<div class="wf-sched nodrag nowheel nopan">
  <label class="wf-sched__row">
    <span>{tr('Как часто')}</span>
    <Select options={SCHEDULE_KINDS.map((k) => ({ value: k, label: scheduleKindLabel(k) }))} value={s.kind} onchange={(e) => setKind(e.currentTarget.value)} />
  </label>

  {#if s.kind === 'hourly'}
    <label class="wf-sched__row">
      <span>{tr('Минута')}</span>
      <TextInput type="number" min="0" max="59" value={s.minute ?? 0} onchange={(e) => patch({ minute: Number(e.currentTarget.value) })} />
    </label>
  {/if}

  {#if s.kind === 'weekly'}
    <label class="wf-sched__row">
      <span>{tr('День недели')}</span>
      <Select options={[1, 2, 3, 4, 5, 6, 7].map((d) => ({ value: String(d), label: weekdayName(d) }))} value={String(s.weekday || 1)} onchange={(e) => patch({ weekday: Number(e.currentTarget.value) })} />
    </label>
  {/if}

  {#if s.kind === 'monthly'}
    <label class="wf-sched__row">
      <span>{tr('Число месяца')}</span>
      <TextInput type="number" min="1" max="31" value={s.day || 1} onchange={(e) => patch({ day: Number(e.currentTarget.value) })} />
    </label>
  {/if}

  {#if s.kind === 'once'}
    <label class="wf-sched__row">
      <span>{tr('Дата')}</span>
      <TextInput type="date" value={s.date || ''} onchange={(e) => patch({ date: e.currentTarget.value })} />
    </label>
  {/if}

  {#if needsTime}
    <label class="wf-sched__row">
      <span>{tr('Время')}</span>
      <TextInput type="time" value={s.time || '09:00'} onchange={(e) => patch({ time: e.currentTarget.value || '09:00' })} />
    </label>
  {/if}

  {#if s.kind === 'interval'}
    <div class="wf-sched__row">
      <span>{tr('Каждые')}</span>
      <div class="wf-sched__pair">
        <TextInput type="number" min="1" value={s.amount || 30} onchange={(e) => patch({ amount: Number(e.currentTarget.value) })} />
        <Select options={[{ value: 'minutes', label: tr('минут') }, { value: 'hours', label: tr('часов') }, { value: 'days', label: tr('дней') }]} value={s.unit || 'minutes'} onchange={(e) => patch({ unit: e.currentTarget.value })} />
      </div>
    </div>
    <p class="wf-sched__hint">{tr('Не чаще раза в 3 минуты')}</p>
  {/if}
</div>

<style>
  .wf-sched {
    display: grid;
    gap: 6px;
    padding: var(--k-space-3);
    border-radius: var(--k-radius-md);
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
