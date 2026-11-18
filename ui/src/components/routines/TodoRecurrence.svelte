<script>
  // Редактор повтора для задач и рутин: готовые варианты, дни недели, число месяца или «N-й день недели», месяц года.
  // Отдаёт правило сервера (или null) через onchange; сам ничего не сохраняет. next — готовая строка «следующий запуск» от родителя.
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import {
    editorFromRecurrence,
    recurrenceFromEditor,
    recurrenceSummary,
    toggleWeekday,
    weekdayOf,
    monthOf,
    monthLabel,
  } from '../../lib/todos.js';
  import { Select, Chip } from '../kit';

  let { rule = null, dueDate = null, allowNone = true, next = '', onchange } = $props();

  let editor = $state(editorFromRecurrence(untrack(() => rule)));
  const locale = typeof window !== 'undefined' ? window.LOCALE || undefined : undefined;

  const options = $derived([
    ...(allowNone ? [{ value: 'none', label: tr('Не повторять') }] : []),
    { value: 'daily', label: tr('Каждый день') },
    { value: 'weekdays', label: tr('По будням') },
    { value: 'weekly', label: tr('Еженедельно') },
    { value: 'biweekly', label: tr('Раз в 2 недели') },
    { value: 'monthly', label: tr('Ежемесячно') },
    { value: 'yearly', label: tr('Ежегодно') },
    ...(editor.preset === 'custom' ? [{ value: 'custom', label: tr('Свой повтор') }] : []),
  ]);
  const dayNames = $derived([tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')]);
  const dayOptions = $derived(
    [tr('понедельник'), tr('вторник'), tr('среду'), tr('четверг'), tr('пятницу'), tr('субботу'), tr('воскресенье')].map((label, i) => ({ value: String(i + 1), label })),
  );
  const byOptions = $derived([
    { value: 'date', label: tr('Числом') },
    { value: 'nth', label: tr('Днём недели') },
  ]);
  const nthOptions = $derived([
    { value: '1', label: tr('первый') },
    { value: '2', label: tr('второй') },
    { value: '3', label: tr('третий') },
    { value: '4', label: tr('четвёртый') },
    { value: '-1', label: tr('последний') },
  ]);
  const monthOptions = $derived(Array.from({ length: 12 }, (_, i) => ({ value: String(i + 1), label: monthLabel(i + 1, locale) })));

  const showDays = $derived(editor.preset === 'weekly' || editor.preset === 'biweekly');
  const showCalendar = $derived(editor.preset === 'monthly' || editor.preset === 'yearly');
  const current = $derived(recurrenceFromEditor(editor, dueDate));
  const summary = $derived(current ? recurrenceSummary(current, tr, locale) : '');

  function update(patch) {
    editor = { ...editor, ...patch };
    onchange?.(recurrenceFromEditor(editor, dueDate));
  }

  function pickPreset(value) {
    if (value === editor.preset) return;
    const needDays = (value === 'weekly' || value === 'biweekly') && !editor.weekdays.length;
    update({ preset: value, weekdays: needDays ? [weekdayOf(dueDate)] : editor.weekdays });
  }

  const toggleDay = (day) => update({ weekdays: toggleWeekday(editor.weekdays, day) });
  const setMonthday = (event) => {
    const raw = event.currentTarget.value.trim();
    update({ monthday: raw === '' ? null : Number(raw) });
  };
</script>

<div class="tr-rec">
  <Select {options} value={editor.preset} onchange={(e) => pickPreset(e.currentTarget.value)} aria-label={tr('Повтор')} />
  {#if showDays}
    <div class="tr-rec-days" role="group" aria-label={tr('Дни недели')}>
      {#each dayNames as name, i (i)}
        <Chip selected={editor.weekdays.includes(i + 1)} onclick={() => toggleDay(i + 1)}>{name}</Chip>
      {/each}
    </div>
  {/if}
  {#if showCalendar}
    <div class="tr-rec-row">
      <div class="tr-rec-cell"><Select options={byOptions} value={editor.by} onchange={(e) => update({ by: e.currentTarget.value })} aria-label={tr('Как считать день')} /></div>
      {#if editor.by === 'nth'}
        <div class="tr-rec-cell"><Select options={nthOptions} value={String(editor.nth)} onchange={(e) => update({ nth: Number(e.currentTarget.value) })} aria-label={tr('Какой по счёту')} /></div>
        <div class="tr-rec-cell"><Select options={dayOptions} value={String(editor.nthDay ?? weekdayOf(dueDate))} onchange={(e) => update({ nthDay: Number(e.currentTarget.value) })} aria-label={tr('День недели')} /></div>
      {:else}
        <input class="k-input tr-rec-num" type="number" min="1" max="31" value={editor.monthday ?? ''} placeholder={tr('как в сроке')} aria-label={tr('Число месяца')} onchange={setMonthday} />
      {/if}
      {#if editor.preset === 'yearly'}
        <div class="tr-rec-cell"><Select options={monthOptions} value={String(editor.month ?? monthOf(dueDate))} onchange={(e) => update({ month: Number(e.currentTarget.value) })} aria-label={tr('Месяц')} /></div>
      {/if}
    </div>
  {/if}
  {#if summary}
    <p class="tr-rec-sum">{summary}{#if next}<span class="tr-rec-next"> · {tr('следующий запуск {when}', { when: next })}</span>{/if}</p>
  {/if}
</div>

<style>
  .tr-rec {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .tr-rec-days,
  .tr-rec-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tr-rec-cell {
    flex: 1 1 150px;
    min-width: 0;
  }
  .tr-rec-num {
    width: 110px;
  }
  .tr-rec-sum {
    margin: 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .tr-rec-next {
    color: var(--k-brand);
  }
</style>
