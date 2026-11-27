<script>
  // «Новый процесс»: фраза («каждый понедельник собери отчёт») и бот дают процесс «триггер → бот»; «Пустой холст» открывает пустой процесс.
  import { tr } from '../../lib/i18n.js';
  import { Button, Field, Select, TextInput } from '../kit';

  /** bots [{id,name}]; botId — бот уже выбран (панель бота); oncreate(phrase, botId) и onblank() вернут ошибку текстом или ''. */
  let { bots = [], botId = '', oncreate, onblank, oncancel } = $props();

  let phrase = $state('');
  let bot = $state('');
  let error = $state('');
  let busy = $state(false);

  const chosen = $derived(botId || bot || bots[0]?.id || '');
  const options = $derived(bots.map((b) => ({ value: b.id, label: b.name })));

  async function submit(event) {
    event?.preventDefault();
    if (!phrase.trim() || busy) return;
    busy = true;
    error = (await oncreate?.(phrase.trim(), chosen)) || '';
    busy = false;
  }

  async function blank() {
    error = (await onblank?.()) || '';
  }
</script>

<form class="np" onsubmit={submit}>
  <Field label={tr('Что и когда')} hint={tr('Например: «каждый понедельник собери отчёт»')} {error}>
    {#snippet children({ id, describedby, invalid })}
      <TextInput {id} {describedby} {invalid} bind:value={phrase} placeholder={tr('каждый понедельник собери отчёт')} maxlength="500" />
    {/snippet}
  </Field>
  {#if !botId}
    <Field label={tr('Бот')}>
      {#snippet children({ id })}
        <Select {id} {options} value={chosen} placeholder={tr('Ботов пока нет')} onchange={(e) => (bot = e.currentTarget.value)} />
      {/snippet}
    </Field>
  {/if}
  <div class="np-row">
    <Button type="submit" variant="primary" loading={busy} disabled={!phrase.trim() || !chosen}>{tr('Создать')}</Button>
    <Button variant="ghost" iconName="workflow" onclick={blank}>{tr('Пустой холст')}</Button>
    <Button variant="ghost" onclick={() => oncancel?.()}>{tr('Отмена')}</Button>
  </div>
</form>

<style>
  .np {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding: var(--k-space-3) var(--k-space-4);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface);
    box-shadow: 0 0 0 1px var(--k-line);
  }
  .np-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
</style>
