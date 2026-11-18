<script>
  // Вкладка «Задачи» окна «Рутины»: доска из трёх разделов, фильтры, поиск, перенос просроченных,
  // карточка задачи сбоку и архив. Данные берёт сама у /api/todos и обновляется по событию todos_changed.
  // Задача создаётся фразой: код сервера находит срок и повтор (create() — единственное место запроса).
  import { api as defaultApi, serverText } from '../../lib/api.js';
  import { tr } from '../../lib/i18n.js';
  import { dateLabel } from '../../lib/composer.js';
  import { toast } from '../../lib/toast.js';
  import {
    BUCKETS,
    FILTERS,
    groupBoard,
    filterCounts,
    carryOverCandidates,
    nextBucket,
    checklistProgress,
    dueInfo,
    recurrenceSummary,
    matchesQuery,
    todayIso,
    OWNERS,
    filterByOwner,
  } from '../../lib/todos.js';
  import { filterTasks, TASK_STATES } from '../../lib/tasks.js';
  import { whenText } from '../../lib/when.js';
  import Composer from './Composer.svelte';
  import TodoCard from './TodoCard.svelte';
import { Button, IconButton, EmptyState, Icon, Chip, TextInput, ListRow, Menu, Tabs, Select, Badge } from '../kit';

  // onopentask(id) открывает окно «Поручения» на этом поручении между ботами.
  let { api = defaultApi, onopentask } = $props();

  let todos = $state([]);
  let archived = $state([]);
  let loaded = $state(false);
  let error = $state('');
  let today = $state(todayIso());
  let filter = $state('all');
  const OWNER_KEY = 'deloist.todos.owner';
  const storedOwner = () => {
    try {
      const saved = localStorage.getItem(OWNER_KEY);
      return OWNERS.includes(saved) ? saved : 'mine';
    } catch {
      return 'mine';
    }
  };
  let owner = $state(storedOwner()); // mine | bots | all
  let botTasks = $state([]); // поручения между ботами (GET /api/tasks), только чтение
  let executor = $state(''); // исполнитель новой задачи: пусто — я сам, иначе id бота
  let query = $state('');
  let view = $state('board'); // board | archive
  let selectedId = $state('');
  let text = $state('');
  let composer = $state();
  let newBucket = $state('active');
  let busy = $state(false);
  let menuId = $state('');
  let dragId = $state('');
  let dragOver = $state('');
  let doneOpen = $state({ key: false, active: false, someday: false });
  let carrying = $state(false);
  const pending = new Set();

  // Поле ввода: исполнитель по «+Имя», календарь, подзадачи, файлы и голос.
  let bots = $state([]);
  let subtasks = $state([]); // тексты подзадач, которые уйдут в чек-лист новой задачи
  let files = $state([]); // File, которые уйдут к новой задаче
  let dateInput = $state();
  let fileInput = $state();
  let micButton = $state(null);
  const MAX_FILE = 10 * 1024 * 1024; // как у вложений чата (attach::MAX_BYTES)
  const MAX_FILES = 10;

  const scoped = $derived(filterByOwner(todos, owner));
  const board = $derived(groupBoard(scoped, { filter, query, today }));
  const handoffs = $derived(owner === 'mine' ? [] : filterTasks(botTasks, 'week').filter((t) => !query.trim() || t.text.toLowerCase().includes(query.trim().toLowerCase())));
  const ownerItems = $derived([
    { id: 'mine', label: tr('Мои') },
    { id: 'bots', label: tr('Ботов') },
    { id: 'all', label: tr('Все') },
  ]);
  const executorOptions = $derived([{ value: '', label: tr('Я') }, ...bots.map((b) => ({ value: b.id, label: b.name }))]);
  const botName = (id) => bots.find((b) => b.id === id)?.name || id;
  const counts = $derived(filterCounts(todos, today));
  const candidates = $derived(carryOverCandidates(todos, today));
  const selected = $derived(todos.find((t) => t.id === selectedId) || null);
  const archiveRows = $derived(archived.filter((t) => matchesQuery(t, query)));
  const shown = $derived(BUCKETS.reduce((n, b) => n + board[b].open.length + board[b].done.length, 0) + handoffs.length);

  const bucketName = (b) => ({ key: tr('Главное'), active: tr('Актуальное'), someday: tr('Когда-нибудь') })[b];
  const filterName = (f) => ({ all: tr('Все'), today: tr('Сегодня'), overdue: tr('Просрочено'), nodue: tr('Без срока') })[f];

  function fail(err) {
    error = serverText(err?.data?.message || err?.message) || tr('Не удалось выполнить запрос');
  }

  // Запросы списка не накладываются: пока один идёт, следующий ждёт и идёт ровно один раз.
  let loading = false;
  let again = false;
  async function load() {
    if (loading) {
      again = true;
      return;
    }
    loading = true;
    try {
      const list = await api('/api/todos?filter=all');
      if (Array.isArray(list)) todos = list;
      today = todayIso();
      error = '';
    } catch (err) {
      fail(err);
    } finally {
      loading = false;
      loaded = true;
      if (again) {
        again = false;
        load();
      }
    }
  }

  async function loadBotTasks() {
    try {
      const data = await api('/api/tasks');
      if (Array.isArray(data?.tasks)) botTasks = data.tasks;
    } catch {
      botTasks = [];
    }
  }

  function pickOwner(next) {
    try {
      localStorage.setItem(OWNER_KEY, next);
    } catch {
      // без хранилища выбор живёт до закрытия окна
    }
    if (next !== 'mine') loadBotTasks();
  }

  async function loadArchive() {
    try {
      const list = await api('/api/todos/archive');
      if (Array.isArray(list)) archived = list;
      error = '';
    } catch (err) {
      fail(err);
    }
  }

  $effect(() => {
    load();
    loadBots();
    if (owner !== 'mine') loadBotTasks();
    const onEvent = (e) => {
      if (e.detail?.type === 'task_update' && owner !== 'mine') loadBotTasks();
      if (e.detail?.type !== 'todos_changed') return;
      load();
      if (view === 'archive') loadArchive();
    };
    window.addEventListener('deloist:event', onEvent);
    return () => window.removeEventListener('deloist:event', onEvent);
  });

  const put = (next) => {
    todos = todos.map((t) => (t.id === next.id ? next : t));
  };

  // Фразу разбирает сервер: код, а где его мало — модель человека (todo_parse.rs); задача не теряется.
  function create(parsed) {
    return api('/api/todos/from-text', { method: 'POST', body: { text: parsed.text, bucket: newBucket, ...(executor ? { assignee: executor } : {}) } });
  }

  // Боты для «+Имя»: исполнителем бывает только бот (людей-исполнителей в данных нет).
  async function loadBots() {
    try {
      const state = await api('/api/state');
      if (Array.isArray(state?.bots)) bots = state.bots.filter((b) => b && b.name && !b.archived);
    } catch {
      bots = [];
    }
  }

  function toBase64(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1] || '');
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }

  // Подзадачи и файлы относятся к созданной задаче; если фраза распалась на несколько, они уходят к первой.
  async function attachExtras(created) {
    const first = Array.isArray(created) ? created[0] : null;
    if (!first?.id) return;
    const items = subtasks.map((t) => t.trim()).filter(Boolean).map((text) => ({ text, done: false }));
    try {
      if (items.length) await api(`/api/todos/${encodeURIComponent(first.id)}`, { method: 'PATCH', body: { items } });
      for (const file of files) {
        await api(`/api/todos/${encodeURIComponent(first.id)}/files`, { method: 'POST', body: { name: file.name, data: await toBase64(file) } });
      }
    } catch (err) {
      fail(err);
    }
  }

  function pickDate() {
    if (!dateInput) return;
    try {
      dateInput.showPicker();
    } catch {
      dateInput.focus();
      dateInput.click();
    }
  }

  function addFiles(list) {
    const next = [...files];
    for (const file of list) {
      if (next.length >= MAX_FILES) {
        error = tr('К задаче можно приложить до {n} файлов', { n: MAX_FILES });
        break;
      }
      if (file.size > MAX_FILE) {
        error = tr('Файл «{name}» больше {mb} МБ', { name: file.name, mb: MAX_FILE / (1024 * 1024) });
        continue;
      }
      next.push(file);
    }
    files = next;
  }

  // Голос: тот же микрофон, что в чате (voice.js); распознанное дописывается в поле. Без voice.js кнопки нет.
  const voiceReady = typeof window !== 'undefined' && Boolean(window.DeloistVoice?.voiceButton);
  $effect(() => {
    if (!voiceReady || !micButton) return;
    const mic = window.DeloistVoice.voiceButton({
      button: micButton,
      onText: (spoken) => composer?.append(spoken),
      onError: (message) => (error = message),
    });
    return () => mic?.cancel?.();
  });

  async function add(parsed) {
    if (busy) return;
    busy = true;
    try {
      const created = await create(parsed);
      text = '';
      error = '';
      await attachExtras(created);
      subtasks = [];
      files = [];
      await load();
    } catch (err) {
      fail(err);
    } finally {
      busy = false;
    }
  }

  // Отметка сразу меняет вид; при отказе сервера возвращаем как было.
  export async function toggle(todo) {
    if (pending.has(todo.id)) return;
    pending.add(todo.id);
    const before = todo;
    const on = !todo.done;
    put({ ...todo, done: on, done_at: on ? Date.now() : null });
    try {
      const res = await api(`/api/todos/${encodeURIComponent(todo.id)}/done`, { method: on ? 'POST' : 'DELETE' });
      error = '';
      const done = res?.todo ?? res;
      if (done?.id) put(done);
      if (res?.next) await load();
    } catch (err) {
      put(before);
      fail(err);
    } finally {
      pending.delete(todo.id);
    }
  }

  export async function moveTo(todo, bucket) {
    if (!todo || todo.bucket === bucket) return;
    const before = todo;
    put({ ...todo, bucket });
    try {
      const next = await api(`/api/todos/${encodeURIComponent(todo.id)}`, { method: 'PATCH', body: { bucket } });
      error = '';
      if (next?.id) put(next);
    } catch (err) {
      put(before);
      fail(err);
    }
  }

  async function carryOver() {
    if (carrying) return;
    carrying = true;
    try {
      await api('/api/todos/carry-over', { method: 'POST' });
      error = '';
      await load();
    } catch (err) {
      fail(err);
    } finally {
      carrying = false;
    }
  }

  async function restore(todo) {
    try {
      await api(`/api/todos/${encodeURIComponent(todo.id)}/restore`, { method: 'POST' });
      archived = archived.filter((t) => t.id !== todo.id);
      error = '';
      await load();
    } catch (err) {
      fail(err);
    }
  }

  async function archive(todo) {
    try {
      await api(`/api/todos/${encodeURIComponent(todo.id)}`, { method: 'DELETE' });
      todos = todos.filter((t) => t.id !== todo.id);
      if (selectedId === todo.id) selectedId = '';
      error = '';
      const name = todo.title.length > 40 ? `${todo.title.slice(0, 40)}…` : todo.title;
      toast(tr('В архиве: {title}', { title: name }), { duration: 7000, action: { label: tr('Вернуть'), onclick: () => restore(todo) } });
    } catch (err) {
      fail(err);
    }
  }

  async function showView(next) {
    view = next;
    menuId = '';
    if (next === 'archive') await loadArchive();
  }

  // ---- меню строки и перетаскивание ----

  const menuItems = (todo) => [
    { kind: 'note', label: tr('Переместить в…') },
    ...BUCKETS.map((b) => ({ id: `move:${b}`, label: bucketName(b), disabled: todo.bucket === b, hint: todo.bucket === b ? tr('сейчас') : '' })),
    { kind: 'sep' },
    { id: 'open', label: tr('Открыть карточку') },
    { id: 'archive', label: tr('В архив'), danger: true },
  ];

  function pickMenu(todo, id) {
    menuId = '';
    if (id.startsWith('move:')) moveTo(todo, id.slice(5));
    else if (id === 'open') selectedId = todo.id;
    else if (id === 'archive') archive(todo);
  }

  function dragStart(event, todo) {
    dragId = todo.id;
    menuId = '';
    event.dataTransfer?.setData('text/plain', todo.id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  function dragEnd() {
    dragId = '';
    dragOver = '';
  }

  function over(event, bucket) {
    if (!dragId) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
    dragOver = bucket;
  }

  function drop(event, bucket) {
    if (!dragId) return;
    event.preventDefault();
    const todo = todos.find((t) => t.id === dragId);
    dragEnd();
    moveTo(todo, bucket);
  }

  // ---- подписи ----

  const locale = () => (typeof window !== 'undefined' && window.LOCALE) || undefined;

  function dueText(info) {
    const day = {
      today: tr('Сегодня'),
      tomorrow: tr('Завтра'),
      yesterday: tr('Вчера'),
      date: dateLabel(info.date, locale()),
    }[info.when];
    return info.time ? `${day}, ${info.time}` : day;
  }

  const repeatText = (rule) => recurrenceSummary(rule, tr, locale());
</script>

{#snippet row(todo)}
  {@const due = dueInfo(todo, today)}
  {@const progress = checklistProgress(todo)}
  <ListRow
    class="td-row {todo.done ? 'td-row--done' : ''} {selectedId === todo.id ? 'td-row--picked' : ''} {dragId === todo.id ? 'td-row--drag' : ''}"
    title={todo.title}
    titleClass="td-title"
    done={todo.done}
    draggable={!todo.done}
    ondragstart={(e) => dragStart(e, todo)}
    ondragend={dragEnd}
    onclick={() => (selectedId = todo.id)}
  >
    {#snippet leading()}
      <button
        type="button"
        class="td-check"
        class:td-check--on={todo.done}
        role="checkbox"
        aria-checked={todo.done}
        aria-label={tr('Отметить: {title}', { title: todo.title })}
        onclick={() => toggle(todo)}
      >
        <Icon name="check" size={16} stroke={3} />
      </button>
    {/snippet}
    {#snippet meta()}
      {#if due || todo.recurrence || progress.total || todo.assignee}
        <span class="td-meta">
          {#if due}<span class="td-chip" class:td-chip--late={due.overdue && !todo.done}>{dueText(due)}</span>{/if}
          {#if todo.assignee}<span class="td-chip">{tr('Исполнитель: {name}', { name: botName(todo.assignee) })}</span>{/if}
          {#if todo.recurrence}<span class="td-chip td-chip--repeat">{repeatText(todo.recurrence)}</span>{/if}
          {#if progress.total}<span class="td-chip" aria-label={tr('Чек-лист: {done} из {total}', progress)}>{progress.done}/{progress.total}</span>{/if}
        </span>
      {/if}
    {/snippet}
    {#snippet trailing()}
      <div class="td-more">
        <IconButton label={tr('Действия: {title}', { title: todo.title })} size="sm" aria-haspopup="menu" aria-expanded={menuId === todo.id} data-menu-trigger onclick={() => (menuId = menuId === todo.id ? '' : todo.id)}>
          {#snippet icon()}
            <Icon name="more" size={16} stroke={2.5} />
          {/snippet}
        </IconButton>
        {#if menuId === todo.id}
          <div class="td-menu">
            <Menu label={tr('Действия: {title}', { title: todo.title })} items={menuItems(todo)} onpick={(id) => pickMenu(todo, id)} onclose={() => (menuId = '')} />
          </div>
        {/if}
      </div>
    {/snippet}
  </ListRow>
{/snippet}

<div class="td">
  {#if error}<p class="td-error" role="alert">{error}</p>{/if}

  <div class="td-form">
    <Composer bind:this={composer} bind:value={text} {api} {bots} placeholder={tr('Позвонить маме завтра в 10:00. «+Имя» назначит бота')} onsubmit={add} />
    {#if subtasks.length}
      <ul class="td-sub" aria-label={tr('Подзадачи новой задачи')}>
        {#each subtasks as _, i (i)}
          <li class="td-sub-row">
            <input class="td-sub-input" type="text" maxlength="200" bind:value={subtasks[i]} placeholder={tr('Подзадача')} aria-label={tr('Подзадача {n}', { n: i + 1 })} />
            <IconButton class="td-x" name="close" size="sm" label={tr('Убрать подзадачу {n}', { n: i + 1 })} onclick={() => (subtasks = subtasks.filter((_, at) => at !== i))} />
          </li>
        {/each}
      </ul>
    {/if}
    {#if files.length}
      <ul class="td-files" aria-label={tr('Файлы новой задачи')}>
        {#each files as file, i (i)}
          <li class="td-file">
            <span class="td-file-name">{file.name}</span>
            <IconButton class="td-x" name="close" size="sm" label={tr('Убрать: {what}', { what: file.name })} onclick={() => (files = files.filter((_, at) => at !== i))} />
          </li>
        {/each}
      </ul>
    {/if}
    <div class="td-form-bar">
      <div class="td-tools">
        <Button size="sm" variant="ghost" onclick={pickDate} aria-label={tr('Выбрать дату')}>{tr('Дата')}</Button>
        <input bind:this={dateInput} class="td-hidden-input" type="date" tabindex="-1" aria-hidden="true" onchange={(e) => { composer?.setDate(e.currentTarget.value); e.currentTarget.value = ''; }} />
        <span class="td-executor"><Select options={executorOptions} bind:value={executor} aria-label={tr('Исполнитель')} /></span>
        <Button size="sm" variant="ghost" onclick={() => (newBucket = nextBucket(newBucket))} aria-label={tr('Раздел новой задачи: {name}', { name: bucketName(newBucket) })}>
          {tr('Раздел: {name}', { name: bucketName(newBucket) })}
        </Button>
        <Button size="sm" variant="ghost" onclick={() => (subtasks = [...subtasks, ''])} aria-label={tr('Добавить подзадачу')}>{tr('Подзадача')}</Button>
        <Button size="sm" variant="ghost" onclick={() => fileInput?.click()} aria-label={tr('Приложить файл')}>{tr('Файл')}</Button>
        <input bind:this={fileInput} class="td-hidden-input" type="file" multiple tabindex="-1" aria-hidden="true" onchange={(e) => { addFiles([...e.currentTarget.files]); e.currentTarget.value = ''; }} />
        {#if voiceReady}
          <IconButton bind:ref={micButton} name="mic" size="sm" label={tr('Сказать голосом')} class="td-mic" />
        {/if}
      </div>
      <Button variant="primary" size="sm" loading={busy} disabled={!text.trim()} onclick={() => composer?.submit()}>{tr('Добавить')}</Button>
    </div>
  </div>

  <div class="td-owner">
    <Tabs items={ownerItems} bind:value={owner} label={tr('Чьи задачи')} onchange={pickOwner} />
  </div>

  <div class="td-bar">
    {#if view === 'board'}
      <div class="td-filters" role="group" aria-label={tr('Фильтр по сроку')}>
        {#each FILTERS as f (f)}
          <Chip selected={filter === f} onclick={() => (filter = f)}>
            {filterName(f)}{f === 'overdue' && counts.overdue ? ` (${counts.overdue})` : ''}
          </Chip>
        {/each}
      </div>
    {:else}
      <h4 class="td-h">{tr('Архив')}</h4>
    {/if}
    <TextInput class="td-search" type="search" bind:value={query} placeholder={tr('Поиск по задачам')} aria-label={tr('Поиск по задачам')} />
    <Button size="sm" variant="ghost" aria-pressed={view === 'archive'} onclick={() => showView(view === 'archive' ? 'board' : 'archive')}>
      {view === 'archive' ? tr('К задачам') : tr('Архив')}
    </Button>
  </div>

  {#if view === 'board' && candidates.length}
    <div class="td-carry" role="status">
      <span>{tr('С прошлых дней осталось {n}', { n: candidates.length })}</span>
      <Button size="sm" variant="primary" loading={carrying} onclick={carryOver}>{tr('Перенести на сегодня')}</Button>
    </div>
  {/if}

  <div class="td-body" class:td-body--card={selected && view === 'board'}>
    <div class="td-main-col">
      {#if view === 'archive'}
        {#if !archiveRows.length}
          <EmptyState title={tr('В архиве пусто')} text={tr('Сюда попадают убранные и вчерашние выполненные задачи.')} />
        {:else}
          <ul class="k-lrows">
            {#each archiveRows as t (t.id)}
              <ListRow class="td-row td-row--done" title={t.title} titleClass="td-title" done>
                {#snippet meta()}
                  {#if t.due_date}<span class="td-meta"><span class="td-chip">{dateLabel(t.due_date, locale())}</span></span>{/if}
                {/snippet}
                {#snippet trailing()}
                  <Button size="sm" variant="ghost" onclick={() => restore(t)}>{tr('Вернуть')}</Button>
                {/snippet}
              </ListRow>
            {/each}
          </ul>
        {/if}
      {:else if loaded && !todos.length}
        <EmptyState title={tr('Задач пока нет')} text={tr('Напиши, что нужно сделать, и когда. Срок и повтор найдёт код, модель не тратится.')} />
      {:else}
        {#each BUCKETS as b (b)}
          {@const section = board[b]}
          <section
            class="td-section"
            class:td-section--over={dragOver === b}
            aria-label={bucketName(b)}
            ondragover={(e) => over(e, b)}
            ondragleave={() => dragOver === b && (dragOver = '')}
            ondrop={(e) => drop(e, b)}
          >
            <h4 class="td-h">{bucketName(b)} <span class="td-count">{section.open.length}</span></h4>
            {#if section.open.length}
              <ul class="k-lrows">
                {#each section.open as t (t.id)}{@render row(t)}{/each}
              </ul>
            {:else}
              <p class="td-empty">{dragId ? tr('Отпусти здесь') : tr('Пока пусто')}</p>
            {/if}
            {#if section.done.length}
              <Button class="td-done-toggle" variant="ghost" size="sm" aria-expanded={doneOpen[b]} onclick={() => (doneOpen[b] = !doneOpen[b])}>
                <Icon name="chevron-right" size={14} stroke={2.4} class={['td-caret', doneOpen[b] && 'td-caret--open']} />
                {tr('Выполнено ({n})', { n: section.done.length })}
              </Button>
              {#if doneOpen[b]}
                <ul class="k-lrows">
                  {#each section.done as t (t.id)}{@render row(t)}{/each}
                </ul>
              {/if}
            {/if}
          </section>
        {/each}
        {#if handoffs.length}
          <section class="td-section" aria-label={tr('Поручения между ботами')}>
            <h4 class="td-h">{tr('Поручения между ботами')} <span class="td-count">{handoffs.length}</span></h4>
            <ul class="k-lrows">
              {#each handoffs as task (task.id)}
                {@const [tone, word] = TASK_STATES[task.state] || TASK_STATES.error}
                <ListRow
                  class="td-row"
                  title={task.text}
                  titleClass="td-title"
                  sub={[`${task.fromName} → ${task.toName}`, task.deadline ? tr('срок {when}', { when: whenText(task.deadline) }) : ''].filter(Boolean).join(' · ')}
                  onclick={() => onopentask?.(task.id)}
                >
                  {#snippet trailing()}<Badge {tone} text={word} />{/snippet}
                </ListRow>
              {/each}
            </ul>
          </section>
        {/if}
        {#if loaded && !shown}
          <p class="td-empty">{tr('Ничего не найдено')}</p>
        {/if}
      {/if}
    </div>
    {#if selected && view === 'board'}
      {#key selected.id}
        <TodoCard todo={selected} {api} {bots} onsaved={put} onclose={() => (selectedId = '')} onarchive={archive} />
      {/key}
    {/if}
  </div>
</div>

<style>
  .td {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding-top: var(--k-space-2, 8px);
  }
  .td-error {
    margin: 0;
    color: var(--k-danger-text);
    font: 600 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .td-form {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-3, 12px) var(--k-space-4, 16px);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2);
  }
  .td-form-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2, 8px);
  }
  .td-tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-1, 4px);
    min-width: 0;
  }
  .td-hidden-input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }
  :global(.td-mic[data-state='listening']) {
    background: var(--k-attention);
    color: var(--k-on-brand);
  }
  :global(.td-mic[data-state='working']),
  :global(.td-mic[data-state='downloading']),
  :global(.td-mic[data-state='loading']) {
    opacity: 0.6;
  }
  .td-sub,
  .td-files {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-1, 4px);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .td-files {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .td-sub-row,
  .td-file {
    display: flex;
    align-items: center;
    gap: var(--k-space-1, 4px);
  }
  .td-sub-row {
    padding-left: var(--k-space-3, 12px);
  }
  .td-file {
    padding: var(--k-space-1) var(--k-space-1) var(--k-space-1) var(--k-space-3);
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface);
    border: 1px solid var(--k-line);
    font: 600 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    color: var(--k-text);
  }
  .td-file-name {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .td-sub-input {
    flex: 1;
    min-width: 0;
    padding: var(--k-space-2) var(--k-space-3);
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface);
    color: var(--k-text);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .td-sub-input:focus {
    outline: none;
    border-color: var(--k-brand);
    box-shadow: var(--k-focus);
  }
  .td-executor {
    width: 150px;
  }
  .td-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-2, 8px);
  }
  .td-filters {
    display: flex;
    flex-wrap: wrap;
    flex: 1 1 auto;
    gap: 6px;
  }
  .td-carry {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-2, 8px) var(--k-space-4, 16px);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .td-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--k-space-4, 16px);
  }
  .td-body--card {
    grid-template-columns: minmax(0, 1fr) minmax(260px, 340px);
  }
  @media (max-width: 760px) {
    .td-body--card {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .td-main-col {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    min-width: 0;
  }
  .td-section {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-2, 8px);
    border: 1px dashed transparent;
    border-radius: var(--k-radius-lg, 26px);
  }
  .td-section--over {
    border-color: var(--k-brand);
    background: var(--k-brand-tint);
  }
  .td-h {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--k-text);
    font: 700 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
  }
  .td-count {
    color: var(--k-text-3);
    font-size: var(--k-text-sm, 13px);
    font-weight: 600;
  }
  .td-empty {
    margin: 0;
    padding: 4px var(--k-space-2, 8px);
    color: var(--k-text-3);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  :global(.td-row[draggable='true']) {
    cursor: grab;
  }
  :global(.td-row--picked) {
    box-shadow: inset 0 0 0 1px var(--k-brand);
  }
  :global(.td-row--drag) {
    opacity: 0.5;
  }
  :global(.td-row--done .td-title) {
    color: var(--k-text-3);
    text-decoration: line-through;
  }
  .td-check {
    display: grid;
    flex: none;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 2px solid var(--k-line-strong);
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: background 160ms var(--k-ease-out, ease), border-color 160ms var(--k-ease-out, ease), color 160ms var(--k-ease-out, ease);
  }
  .td-check:hover {
    border-color: var(--k-brand);
  }
  .td-check--on {
    border-color: var(--k-brand);
    background: var(--k-brand);
    color: var(--k-on-brand);
  }
  .td-check:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  :global(.td-title) {
    color: var(--k-text);
    font: 600 var(--k-text-md, 15px) / 1.35 var(--k-font-body, system-ui);
    overflow-wrap: anywhere;
  }
  .td-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 6px;
  }
  .td-chip {
    padding: 1px 8px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font: 600 var(--k-text-xs, 12px) / 1.5 var(--k-font-body, system-ui);
  }
  .td-chip--late {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .td-chip--repeat {
    background: var(--k-ai-tint);
    color: var(--k-text-2);
  }
  .td-more {
    position: relative;
    flex: none;
  }
  .td-menu {
    position: absolute;
    top: 100%;
    right: 0;
    z-index: 5;
    width: 230px;
    margin-top: 4px;
  }
  :global(.td-caret) {
    transition: transform 160ms var(--k-ease-out, ease);
  }
  :global(.td-caret--open) {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .td-check,
    :global(.td-caret) {
      transition: none;
    }
  }
  .td-bar :global(.td-search) {
    flex: 0 1 200px;
    min-width: 120px;
    width: auto;
  }
  :global(.td-done-toggle) {
    align-self: flex-start;
  }
</style>
