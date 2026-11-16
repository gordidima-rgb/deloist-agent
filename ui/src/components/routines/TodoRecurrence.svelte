<script>
  // Редактор повтора задачи: готовые варианты, дни недели и число месяца.
  // Отдаёт правило сервера (или null) через onchange; сам ничего не сохраняет.
  import { untrack } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { editorFromRecurrence, recurrenceFromEditor, toggleWeekday, weekdayOf } from '../../lib/todos.js';
  import Select from '../kit/Select.svelte';
  import Chip from '../kit/Chip.svelte';

  let { rule = null, dueDate = null, onchange } = $props();

  let editor = $state(editorFromRecurrence(untrack(() => rule)));
  let preset = $state(untrack(() => editor.preset));

  const options = $derived([
    { value: 'none', label: tr('Не повторять') },
    { value: 'daily', label: tr('Каждый день') },
    { value: 'weekdays', label: tr('По будням') },
    { value: 'weekly', label: tr('Еженедельно') },
    { value: 'biweekly', label: tr('Раз в 2 недели') },
    { value: 'monthly', label: tr('Ежемесячно') },
    { value: 'yearly', label: tr('Ежегодно') },
    ...(editor.preset === 'custom' ? [{ value: 'custom', label: tr('Свой повтор') }] : []),
  ]);
  const dayNames = $derived([tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')]);
  const showDays = $derived(editor.preset === 'weekly' || editor.preset === 'biweekly');

  function emit() {
    onchange?.(recurrenceFromEditor(editor, dueDate));
  }

  function pickPreset(value) {
    if (value === editor.preset) return;
    const needDays = (value === 'weekly' || value === 'biweekly') && !editor.weekdays.length;
    editor = { ...editor, preset: value, weekdays: needDays ? [weekdayOf(dueDate)] : editor.weekdays };
    emit();
  }

  $effect(() => {
    // Select меняет preset через bind: передаём выбор наверх один раз на изменение.
    const next = preset;
    untrack(() => pickPreset(next));
  });

  function toggleDay(day) {
    editor = { ...editor, weekdays: toggleWeekday(editor.weekdays, day) };
    emit();
  }

  function setMonthday(event) {
    const raw = event.currentTarget.value.trim();
    editor = { ...editor, monthday: raw === '' ? null : Number(raw) };
    emit();
  }
</script>

<div class="tr-rec">
  <Select {options} bind:value={preset} aria-label={tr('Повтор')} />
  {#if showDays}
    <div class="tr-rec-days" role="group" aria-label={tr('Дни недели')}>
      {#each dayNames as name, i (i)}
        <Chip selected={editor.weekdays.includes(i + 1)} onclick={() => toggleDay(i + 1)}>{name}</Chip>
      {/each}
    </div>
  {/if}
  {#if editor.preset === 'monthly'}
    <label class="tr-rec-month">
      <span>{tr('Число месяца')}</span>
      <input class="k-input tr-rec-num" type="number" min="1" max="31" value={editor.monthday ?? ''} placeholder={tr('как в сроке')} onchange={setMonthday} />
    </label>
  {/if}
</div>

<style>
  .tr-rec {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .tr-rec-days {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tr-rec-month {
    display: flex;
    align-items: center;
    gap: var(--k-space-2, 8px);
    color: var(--k-text-2, #5c5f66);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .tr-rec-num {
    width: 110px;
  }
</style>
