<script>
  // Раздел «Задачи и процессы»: вкладки «Задачи», «Напоминания», «Привычки», «Процессы». У каждого процесса краткая
  // карточка; раскрытая, она показывает цепочку шагов, а править цепочку можно на холсте, который открывается из карточки.
  import { tr } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { IconButton, Tabs } from '../kit';
  import TodosTab from '../routines/TodosTab.svelte';
  import RemindersTab from '../routines/RemindersTab.svelte';
  import HabitsTab from '../routines/HabitsTab.svelte';
  import WorkflowsScreen from '../workflows/WorkflowsScreen.svelte';
  import ProcessList from './ProcessList.svelte';

  /**
   * tab: todos | reminders | habits | processes; canvas — процесс, открытый на холсте ('' — список).
   * reminders — напоминания из общего состояния (у них свой список), onremindersChanged() просит обновить его.
   */
  let { api = realApi, bots = [], models = () => ({}), tab = $bindable('processes'), canvas = $bindable(''), reminders = [], onremindersChanged, onopentask, onopentelegram, onconnect, onsettings, onmenu } = $props();

  const items = [
    { id: 'todos', label: tr('Задачи'), icon: 'todo', panelId: 'tp-pane-todos' },
    { id: 'reminders', label: tr('Напоминания'), icon: 'bell', panelId: 'tp-pane-reminders' },
    { id: 'habits', label: tr('Привычки'), icon: 'flame', panelId: 'tp-pane-habits' },
    { id: 'processes', label: tr('Процессы'), icon: 'repeat', panelId: 'tp-pane-processes' },
  ];
</script>

{#if canvas}
  {#key canvas}
    <WorkflowsScreen id={canvas} {api} {bots} {models} {onconnect} {onsettings} onback={() => (canvas = '')} />
  {/key}
{:else}
  <section class="tp">
    <header class="tp-head">
      <IconButton class="tp-menu" name="wf-list" size="sm" label={tr('Открыть боковую панель')} onclick={() => onmenu?.()} />
      <h2>{tr('Задачи и процессы')}</h2>
    </header>
    <div class="tp-tabs"><Tabs {items} bind:value={tab} label={tr('Разделы')} /></div>
    <div class="tp-body">
      <div class="tp-pane" role="tabpanel" id="tp-pane-{tab}">
        {#if tab === 'todos'}
          <TodosTab {api} {onopentask} />
        {:else if tab === 'reminders'}
          <RemindersTab {api} routines={reminders} onchanged={onremindersChanged} {onopentelegram} />
        {:else if tab === 'habits'}
          <HabitsTab {api} {onopentelegram} />
        {:else}
          <ProcessList {api} {bots} onopen={(id) => (canvas = id)} lead={tr('Процесс сам запускает бота по расписанию или событию. Результат появится в чате бота.')} />
        {/if}
      </div>
    </div>
  </section>
{/if}

<style>
  .tp {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--k-canvas);
  }
  .tp-head {
    display: flex;
    align-items: center;
    gap: var(--k-space-2);
    min-height: 60px;
    padding: var(--k-space-3) var(--k-space-4);
  }
  .tp-head h2 {
    flex: 1;
    margin: 0;
    font: 700 var(--k-text-lg) / 1.2 var(--k-font-display);
  }
  .tp-head :global(.tp-menu) {
    display: none;
  }
  .tp-tabs {
    padding: 0 var(--k-space-4) var(--k-space-3);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tp-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--k-space-4) var(--k-space-5);
  }
  .tp-pane {
    max-width: 820px;
  }
  @media (max-width: 820px) {
    .tp-head :global(.tp-menu) {
      display: inline-flex;
    }
  }
</style>
