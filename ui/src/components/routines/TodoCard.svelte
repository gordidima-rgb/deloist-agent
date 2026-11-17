<script>
  // Боковая карточка задачи: название, срок, раздел, заметка, чек-лист, повтор, автозакрытие, архив.
  // Каждая правка уходит PATCH-запросом по очереди; ответ сервера целиком заменяет задачу у родителя (onsaved).
  import { untrack } from 'svelte';
  import { api as defaultApi, serverText } from '../../lib/api.js';
  import { tr } from '../../lib/i18n.js';
import { Button, IconButton, Field, TextInput, TextArea, Select, Checkbox, Icon } from '../kit';
  import TodoRecurrence from './TodoRecurrence.svelte';

  let { todo, api = defaultApi, onsaved, onclose, onarchive } = $props();

  let title = $state(untrack(() => todo.title));
  let notes = $state(untrack(() => todo.notes || ''));
  let newItem = $state('');
  let bucket = $state(untrack(() => todo.bucket));
  let autoClose = $state(untrack(() => (todo.auto_close_after_min ? String(todo.auto_close_after_min) : '')));
  let error = $state('');
  let chain = Promise.resolve();

  const bucketOptions = $derived([
    { value: 'key', label: tr('Главное') },
    { value: 'active', label: tr('Актуальное') },
    { value: 'someday', label: tr('Когда-нибудь') },
  ]);
  const autoOptions = $derived([
    { value: '', label: tr('Не закрывать само') },
    { value: '30', label: tr('Через 30 минут после срока') },
    { value: '60', label: tr('Через час после срока') },
    { value: '180', label: tr('Через 3 часа после срока') },
    { value: '1440', label: tr('Через сутки после срока') },
  ]);

  // Запросы идут друг за другом: сервер каждый раз выдаёт пунктам чек-листа новые номера, и ответ должен лечь последним.
  function save(body) {
    chain = chain.then(async () => {
      try {
        const next = await api(`/api/todos/${encodeURIComponent(todo.id)}`, { method: 'PATCH', body });
        error = '';
        if (next?.id) onsaved?.(next);
      } catch (err) {
        error = serverText(err?.data?.message || err?.message) || tr('Не удалось выполнить запрос');
        title = todo.title;
        notes = todo.notes || '';
        bucket = todo.bucket;
        autoClose = todo.auto_close_after_min ? String(todo.auto_close_after_min) : '';
      }
    });
    return chain;
  }

  function commitTitle() {
    const text = title.trim();
    if (!text) {
      title = todo.title;
      return;
    }
    if (text !== todo.title) save({ title: text });
  }

  const commitNotes = () => {
    if (notes.trim() !== (todo.notes || '')) save({ notes });
  };

  const setDate = (event) => save({ due_date: event.currentTarget.value || null });
  const setTime = (event) => save({ due_time: event.currentTarget.value || null });

  $effect(() => {
    const next = bucket;
    untrack(() => {
      if (next !== todo.bucket) save({ bucket: next });
    });
  });

  $effect(() => {
    const next = autoClose;
    untrack(() => {
      const now = todo.auto_close_after_min ? String(todo.auto_close_after_min) : '';
      if (next !== now) save({ auto_close_after_min: next ? Number(next) : null });
    });
  });

  const slim = (items) => items.map((i) => ({ text: i.text, done: i.done }));

  function addItem() {
    const text = newItem.trim();
    if (!text) return;
    newItem = '';
    save({ items: [...slim(todo.items), { text, done: false }] });
  }

  const toggleItem = (index) => save({ items: slim(todo.items).map((i, at) => (at === index ? { ...i, done: !i.done } : i)) });
  const removeItem = (index) => save({ items: slim(todo.items).filter((_, at) => at !== index) });

  function setRecurrence(rule) {
    const body = { recurrence: rule };
    if (!rule) {
      body.auto_close_after_min = null;
      autoClose = '';
    }
    save(body);
  }
</script>

<aside class="tc" aria-label={tr('Карточка задачи')}>
  <div class="tc-head">
    <h4 class="tc-h">{tr('Задача')}</h4>
    <IconButton label={tr('Закрыть карточку')} size="sm" onclick={() => onclose?.()}>
      {#snippet icon()}
        <Icon name="close" size={16} />
      {/snippet}
    </IconButton>
  </div>

  {#if error}<p class="tc-error" role="alert">{error}</p>{/if}

  <Field label={tr('Название')}>
    {#snippet children({ id, describedby, invalid })}
      <TextInput {id} {describedby} {invalid} bind:value={title} maxlength={200} onchange={commitTitle} onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()} />
    {/snippet}
  </Field>

  <div class="tc-row">
    <label class="tc-col">
      <span class="tc-label">{tr('Дата')}</span>
      <input class="k-input tc-date" type="date" value={todo.due_date ?? ''} onchange={setDate} aria-label={tr('Дата')} />
    </label>
    <label class="tc-col">
      <span class="tc-label">{tr('Время')}</span>
      <input class="k-input tc-time" type="time" value={todo.due_time ?? ''} onchange={setTime} aria-label={tr('Время')} />
    </label>
  </div>

  <label class="tc-col">
    <span class="tc-label">{tr('Раздел')}</span>
    <Select options={bucketOptions} bind:value={bucket} aria-label={tr('Раздел')} />
  </label>

  <Field label={tr('Заметка')}>
    {#snippet children({ id, describedby, invalid })}
      <TextArea {id} {describedby} {invalid} rows={3} bind:value={notes} maxlength={4000} onblur={commitNotes} />
    {/snippet}
  </Field>

  <div class="tc-col">
    <span class="tc-label">{tr('Чек-лист')}</span>
    {#if todo.items.length}
      <ul class="tc-items">
        {#each todo.items as item, i (item.id)}
          <li class="tc-item">
            <Checkbox checked={item.done} label={item.text} onchange={() => toggleItem(i)} />
            <IconButton label={tr('Убрать: {what}', { what: item.text })} size="sm" onclick={() => removeItem(i)}>
              {#snippet icon()}
                <Icon name="close" size={14} />
              {/snippet}
            </IconButton>
          </li>
        {/each}
      </ul>
    {/if}
    <form class="tc-add" onsubmit={(e) => { e.preventDefault(); addItem(); }}>
      <input class="k-input tc-add-input" type="text" maxlength="200" bind:value={newItem} placeholder={tr('Добавить пункт')} aria-label={tr('Добавить пункт')} />
      <Button size="sm" type="submit" disabled={!newItem.trim()}>{tr('Добавить')}</Button>
    </form>
  </div>

  {#if todo.files?.length}
    <div class="tc-col">
      <span class="tc-label">{tr('Файлы')}</span>
      <ul class="tc-items">
        {#each todo.files as file (file.path)}
          <li class="tc-item"><span class="tc-hint">{file.name}</span></li>
        {/each}
      </ul>
    </div>
  {/if}

  <div class="tc-col">
    <span class="tc-label">{tr('Повтор')}</span>
    <TodoRecurrence rule={todo.recurrence} dueDate={todo.due_date} onchange={setRecurrence} />
  </div>

  {#if todo.recurrence}
    <label class="tc-col">
      <span class="tc-label">{tr('Автозакрытие')}</span>
      <Select options={autoOptions} bind:value={autoClose} aria-label={tr('Автозакрытие')} />
      <span class="tc-hint">{tr('Повторяющаяся задача без отметки закроется сама.')}</span>
    </label>
  {/if}

  <div class="tc-foot">
    <Button variant="ghost" onclick={() => onarchive?.(todo)}>{tr('В архив')}</Button>
  </div>
</aside>

<style>
  .tc {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface);
    align-self: start;
  }
  .tc-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .tc-h {
    margin: 0;
    color: var(--k-text);
    font: 700 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
  }
  .tc-error {
    margin: 0;
    color: var(--k-attention-text);
    font: 600 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .tc-row {
    display: flex;
    gap: var(--k-space-3, 12px);
  }
  .tc-col {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .tc-label {
    color: var(--k-text-2);
    font: 600 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, system-ui);
    letter-spacing: 0.04em;
  }
  .tc-hint {
    color: var(--k-text-3);
    font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, system-ui);
  }
  .tc-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tc-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2, 8px);
  }
  .tc-add {
    display: flex;
    gap: var(--k-space-2, 8px);
  }
  .tc-add-input {
    flex: 1;
    min-width: 0;
  }
  .tc-foot {
    display: flex;
    justify-content: flex-end;
  }
</style>
