<script>
  // Окно «Поручения»: кто → кому · что · состояние · срок · кто начал · токены. Фильтры «Открытые» и «Все за 7 дней»;
  // строка раскрывается итогом, карточками файлов и «Отменить» (для открытых). Данные — GET /api/tasks, их держит app.js.
  import { Modal, Tabs, ListRow, Badge, Button, FileCards, EmptyState } from '../kit';
  import { tr } from '../../lib/i18n.js';
  import { whenText, numberRu } from '../../lib/when.js';
  import { filterTasks, isOpen, TASK_STATES } from '../../lib/tasks.js';

  // focus — задача, которую нужно раскрыть (например, из раздела «Сделано, пока тебя не было»).
  let { open = false, tasks = [], focus = '', oncancel, onopenfile, onclose } = $props();

  const STARTERS = { human: tr('человек'), routine: tr('рутина'), goal: tr('цель'), webhook: tr('вебхук'), bot: tr('бот') };

  let mode = $state('open');
  let expanded = $state({});
  let busy = $state({});

  const shown = $derived(filterTasks(tasks, mode));
  const filters = $derived([
    { id: 'open', label: tr('Открытые') },
    { id: 'week', label: tr('Все за 7 дней') },
  ]);

  $effect(() => {
    if (!open || !focus) return;
    if (!filterTasks(tasks, mode).some((task) => task.id === focus)) mode = 'week';
    expanded[focus] = true;
  });

  const sub = (task) =>
    [
      `${task.fromName} → ${task.toName}`,
      task.deadline ? tr('срок {when}', { when: whenText(task.deadline) }) : '',
      STARTERS[task.startedBy] ? tr('начал: {who}', { who: STARTERS[task.startedBy] }) : '',
      task.tokens ? tr('{n} ток.', { n: numberRu(task.tokens) }) : '',
    ]
      .filter(Boolean)
      .join(' · ');

  async function cancel(task) {
    busy[task.id] = true;
    try {
      await oncancel?.(task);
    } finally {
      busy[task.id] = false;
    }
  }
</script>

<Modal {open} title={tr('Поручения')} size="lg" {onclose}>
  <div class="tp">
    <Tabs items={filters} bind:value={mode} label={tr('Фильтр поручений')} />
    {#if shown.length}
      <ul class="k-lrows tp-list">
        {#each shown as task (task.id)}
          {@const [tone, word] = TASK_STATES[task.state] || TASK_STATES.error}
          <ListRow class="tp-row" data-task={task.id} variant="chevron" bind:expanded={() => Boolean(expanded[task.id]), (value) => (expanded[task.id] = value)} title={task.text} sub={sub(task)}>
            {#snippet trailing()}<Badge {tone} text={word} />{/snippet}
            <div class="tp-body">
              {#if task.summary}<p class="tp-summary">{task.summary}</p>{/if}
              {#if task.resultFiles?.length}<FileCards files={task.resultFiles} onopen={(path) => onopenfile?.(path, task)} />{/if}
              {#if isOpen(task)}
                <div><Button size="sm" variant="danger" loading={busy[task.id]} onclick={() => cancel(task)}>{tr('Отменить поручение')}</Button></div>
              {/if}
            </div>
          </ListRow>
        {/each}
      </ul>
    {:else}
      <EmptyState title={mode === 'open' ? tr('Открытых поручений нет') : tr('За 7 дней поручений нет')} />
    {/if}
  </div>
</Modal>

<style>
  .tp { display: flex; flex-direction: column; gap: var(--k-space-3); }
  .tp-body { display: flex; flex-direction: column; gap: var(--k-space-3); padding-top: var(--k-space-2); }
  .tp-summary { margin: 0; white-space: pre-wrap; color: var(--k-text-2); font: 400 var(--k-text-sm) / 1.45 var(--k-font-body); }
</style>
