<script>
  // Расписание новой рутины: «По времени» (время + повтор + «Начать с»), «По интервалу» (поля интервала остаются в форме) и «Один раз».
  // Отдаёт состояние { kind, at, date, recurrence, startOn } через onchange; повтор рисует тот же редактор, что у задач.
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { todayIso } from '../../lib/todos.js';
  import { KINDS } from '../../lib/routineSchedule.js';
  import { Select, Field } from '../kit';
  import TodoRecurrence from './TodoRecurrence.svelte';

  let { onchange } = $props();

  let state = $state({
    kind: 'at',
    at: '',
    date: '',
    startOn: '',
    recurrence: untrack(() => ({ freq: 'daily', interval: 1, weekdays: [], monthday: null })),
  });

  const labels = $derived({ at: tr('По времени'), every: tr('По интервалу'), once: tr('Один раз') });
  const kinds = $derived(KINDS.map((value) => ({ value, label: labels[value] })));

  function update(patch) {
    state = { ...state, ...patch };
    onchange?.({ ...state });
  }
  // Дни недели и числа в повторе считаются от даты начала, а без неё — от сегодня.
  const anchor = $derived(state.startOn || todayIso());
  $effect(() => untrack(() => onchange?.({ ...state })));
</script>

<div class="rs">
  <Field label={tr('Когда')}>
    {#snippet children({ id })}
      <Select {id} options={kinds} value={state.kind} onchange={(e) => update({ kind: e.currentTarget.value })} />
    {/snippet}
  </Field>
  {#if state.kind === 'at'}
    <div class="rs-row">
      <Field label={tr('Во сколько')}>
        {#snippet children({ id })}
          <input {id} class="k-input rs-time" type="time" value={state.at} onchange={(e) => update({ at: e.currentTarget.value })} />
        {/snippet}
      </Field>
      <Field label={tr('Начать с (необязательно)')}>
        {#snippet children({ id })}
          <input {id} class="k-input rs-date" type="date" value={state.startOn} onchange={(e) => update({ startOn: e.currentTarget.value })} />
        {/snippet}
      </Field>
    </div>
    <TodoRecurrence rule={state.recurrence} dueDate={anchor} allowNone={false} onchange={(recurrence) => update({ recurrence })} />
  {:else if state.kind === 'once'}
    <div class="rs-row">
      <Field label={tr('Дата')}>
        {#snippet children({ id })}
          <input {id} class="k-input rs-date" type="date" min={todayIso()} value={state.date} onchange={(e) => update({ date: e.currentTarget.value })} />
        {/snippet}
      </Field>
      <Field label={tr('Во сколько')}>
        {#snippet children({ id })}
          <input {id} class="k-input rs-time" type="time" value={state.at} onchange={(e) => update({ at: e.currentTarget.value })} />
        {/snippet}
      </Field>
    </div>
  {/if}
</div>

<style>
  .rs {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
  }
  .rs-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-3, 12px);
  }
  .rs-time {
    width: 130px;
  }
  .rs-date {
    width: 170px;
  }
</style>
