<script>
  // Вкладка «Цели» в ящике бота: самопроверка (вкл/выкл и частота) и список целей с действиями.
  // Данные приходят из app.js (S.goals, S.checkins); после каждого изменения app перечитывает состояние
  // и перерисовывает вкладку через onchanged().
  import { untrack } from 'svelte';
  import Switch from '../kit/Switch.svelte';
  import Field from '../kit/Field.svelte';
  import Select from '../kit/Select.svelte';
  import Button from '../kit/Button.svelte';
  import ListItem from '../kit/ListItem.svelte';
  import EmptyState from '../kit/EmptyState.svelte';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { whenText } from '../../lib/when.js';

  let { bot, goals = [], checkin = null, api, onchanged } = $props();

  const start = untrack(() => checkin) || { enabled: false, everyMinutes: 180 };
  let enabled = $state(Boolean(start.enabled));
  let every = $state(String(start.everyMinutes ?? 180));

  const everyOptions = $derived([
    { value: '60', label: tr('Каждый час') },
    { value: '180', label: tr('Каждые 3 часа') },
    { value: '360', label: tr('Каждые 6 часов') },
    { value: '720', label: tr('Каждые 12 часов') },
  ]);

  function statusText(goal) {
    if (goal.status === 'paused') return tr('на паузе');
    if (goal.status === 'completed') return tr('завершена');
    return goal.pendingRun ? tr('ждёт, пока бот освободится') : tr('идёт');
  }

  function lastLine(goal) {
    const last = (goal.progress || []).at(-1);
    return last ? `${whenText(last.at)}: ${last.text}` : tr('Прогресса ещё нет');
  }

  async function run(fn) {
    try {
      await fn();
      onchanged?.();
    } catch (err) {
      toast(err.message);
    }
  }

  const checkUrl = () => `/api/bots/${encodeURIComponent(bot.id)}/checkin`;
  const goalUrl = (goal) => `/api/goals/${encodeURIComponent(goal.id)}`;

  function setCheck(patch) {
    return run(() => api(checkUrl(), { method: 'PUT', body: patch }));
  }

  function toggleCheck(on) {
    enabled = on;
    setCheck({ enabled: on, everyMinutes: Number(every) });
  }

  function changeEvery(value) {
    every = value;
    setCheck({ everyMinutes: Number(value) });
  }

  function setStatus(goal, status) {
    return run(() => api(goalUrl(goal), { method: 'PATCH', body: { status } }));
  }

  function runNow(goal) {
    return run(() => api(`${goalUrl(goal)}/run`, { method: 'POST' }));
  }

  function remove(goal) {
    if (!confirm(tr('Удалить цель «{name}»?', { name: goal.title }))) return;
    return run(() => api(goalUrl(goal), { method: 'DELETE' }));
  }
</script>

<div class="dg">
  <div class="dg-check">
    <Switch label={tr('Самопроверка')} bind:checked={enabled} onchange={toggleCheck} />
    <p class="dg-note">
      {tr('Бот сам осматривает свои цели и работу и пишет, только если есть что сказать. Не чаще 24 раз в сутки; всё, что меняет что-то, он спрашивает.')}
    </p>
    <Field label={tr('Как часто проверять')}>
      {#snippet children(f)}
        <Select id={f.id} options={everyOptions} value={every} disabled={!enabled} onchange={(e) => changeEvery(e.currentTarget.value)} />
      {/snippet}
    </Field>
  </div>

  {#if !goals.length}
    <EmptyState text={tr('Пока нет целей. Скажи боту в чате, чего добиваться, и он поставит цель.')} />
  {:else}
    <ul class="k-list dg-list">
      {#each goals as goal (goal.id)}
        {#snippet actions()}
          <div class="dg-actions">
            {#if goal.status === 'active'}
              <Button size="sm" variant="ghost" onclick={() => runNow(goal)}>{tr('Запустить')}</Button>
              <Button size="sm" variant="ghost" onclick={() => setStatus(goal, 'paused')}>{tr('Пауза')}</Button>
            {/if}
            {#if goal.status === 'paused'}
              <Button size="sm" variant="ghost" onclick={() => setStatus(goal, 'active')}>{tr('Продолжить')}</Button>
            {/if}
            {#if goal.status !== 'completed'}
              <Button size="sm" variant="ghost" onclick={() => setStatus(goal, 'completed')}>{tr('Завершить')}</Button>
            {/if}
            <Button size="sm" variant="ghost" onclick={() => remove(goal)}>{tr('Удалить')}</Button>
          </div>
        {/snippet}
        <ListItem
          class={goal.status === 'active' ? 'dg-item' : 'dg-item is-off'}
          title={goal.title}
          subtitle={lastLine(goal)}
          meta={`${goal.recurring ? tr('повторяющаяся') : tr('разовая')}, ${statusText(goal)}`}
          context={actions}
        />
      {/each}
    </ul>
  {/if}
</div>

<style>
  .dg { display: flex; flex-direction: column; gap: var(--k-space-4); min-width: 0; }
  .dg-check { display: flex; flex-direction: column; gap: var(--k-space-3); }
  .dg-note { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); }
  .dg-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--k-space-1); }
  .dg :global(.dg-item.is-off) { opacity: 0.55; }
</style>
