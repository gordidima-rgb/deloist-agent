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
    editorFromRecurrence,
    matchesQuery,
    todayIso,
  } from '../../lib/todos.js';
  import Composer from './Composer.svelte';
  import TodoCard from './TodoCard.svelte';
  import Button from '../kit/Button.svelte';
  import IconButton from '../kit/IconButton.svelte';
  import EmptyState from '../kit/EmptyState.svelte';
  import Menu from '../Menu.svelte';

  let { api = defaultApi } = $props();

  let todos = $state([]);
  let archived = $state([]);
  let loaded = $state(false);
  let error = $state('');
  let today = $state(todayIso());
  let filter = $state('all');
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
  let micButton = $state();
  const MAX_FILE = 10 * 1024 * 1024; // как у вложений чата (attach::MAX_BYTES)
  const MAX_FILES = 10;

  const board = $derived(groupBoard(todos, { filter, query, today }));
  const counts = $derived(filterCounts(todos, today));
  const candidates = $derived(carryOverCandidates(todos, today));
  const selected = $derived(todos.find((t) => t.id === selectedId) || null);
  const archiveRows = $derived(archived.filter((t) => matchesQuery(t, query)));
  const shown = $derived(BUCKETS.reduce((n, b) => n + board[b].open.length + board[b].done.length, 0));

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
    const onEvent = (e) => {
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
    return api('/api/todos/from-text', { method: 'POST', body: { text: parsed.text, bucket: newBucket } });
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

  function repeatText(rule) {
    const e = editorFromRecurrence(rule);
    const names = [tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')];
    const days = e.weekdays.map((d) => names[d - 1]).join(', ');
    if (e.preset === 'daily') return tr('каждый день');
    if (e.preset === 'weekdays') return tr('по будням');
    if (e.preset === 'weekly') return tr('каждую неделю: {days}', { days });
    if (e.preset === 'biweekly') return tr('раз в 2 недели: {days}', { days });
    if (e.preset === 'monthly') return tr('каждый месяц');
    if (e.preset === 'yearly') return tr('каждый год');
    return tr('повторяется');
  }
</script>

{#snippet row(todo)}
  {@const due = dueInfo(todo, today)}
  {@const progress = checklistProgress(todo)}
  <li
    class="td-row"
    class:td-row--done={todo.done}
    class:td-row--picked={selectedId === todo.id}
    class:td-row--drag={dragId === todo.id}
    draggable={!todo.done}
    ondragstart={(e) => dragStart(e, todo)}
    ondragend={dragEnd}
  >
    <button
      type="button"
      class="td-check"
      class:td-check--on={todo.done}
      role="checkbox"
      aria-checked={todo.done}
      aria-label={tr('Отметить: {title}', { title: todo.title })}
      onclick={() => toggle(todo)}
    >
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
    </button>
    <button type="button" class="td-main" onclick={() => (selectedId = todo.id)} aria-label={tr('Открыть: {title}', { title: todo.title })}>
      <span class="td-title">{todo.title}</span>
      {#if due || todo.recurrence || progress.total}
        <span class="td-meta">
          {#if due}<span class="td-chip" class:td-chip--late={due.overdue && !todo.done}>{dueText(due)}</span>{/if}
          {#if todo.recurrence}<span class="td-chip td-chip--repeat">{repeatText(todo.recurrence)}</span>{/if}
          {#if progress.total}<span class="td-chip" aria-label={tr('Чек-лист: {done} из {total}', progress)}>{progress.done}/{progress.total}</span>{/if}
        </span>
      {/if}
    </button>
    <div class="td-more">
      <IconButton label={tr('Действия: {title}', { title: todo.title })} size="sm" aria-haspopup="menu" aria-expanded={menuId === todo.id} data-menu-trigger onclick={() => (menuId = menuId === todo.id ? '' : todo.id)}>
        {#snippet icon()}
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="12" r="1" /></svg>
        {/snippet}
      </IconButton>
      {#if menuId === todo.id}
        <div class="td-menu">
          <Menu label={tr('Действия: {title}', { title: todo.title })} items={menuItems(todo)} onpick={(id) => pickMenu(todo, id)} onclose={() => (menuId = '')} />
        </div>
      {/if}
    </div>
  </li>
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
            <button type="button" class="td-x" aria-label={tr('Убрать подзадачу {n}', { n: i + 1 })} title={tr('Убрать')} onclick={() => (subtasks = subtasks.filter((_, at) => at !== i))}>×</button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if files.length}
      <ul class="td-files" aria-label={tr('Файлы новой задачи')}>
        {#each files as file, i (i)}
          <li class="td-file">
            <span class="td-file-name">{file.name}</span>
            <button type="button" class="td-x" aria-label={tr('Убрать: {what}', { what: file.name })} title={tr('Убрать')} onclick={() => (files = files.filter((_, at) => at !== i))}>×</button>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="td-form-bar">
      <div class="td-tools">
        <Button size="sm" variant="ghost" onclick={pickDate} aria-label={tr('Выбрать дату')}>{tr('Дата')}</Button>
        <input bind:this={dateInput} class="td-hidden-input" type="date" tabindex="-1" aria-hidden="true" onchange={(e) => { composer?.setDate(e.currentTarget.value); e.currentTarget.value = ''; }} />
        <Button size="sm" variant="ghost" onclick={() => (newBucket = nextBucket(newBucket))} aria-label={tr('Раздел новой задачи: {name}', { name: bucketName(newBucket) })}>
          {tr('Раздел: {name}', { name: bucketName(newBucket) })}
        </Button>
        <Button size="sm" variant="ghost" onclick={() => (subtasks = [...subtasks, ''])} aria-label={tr('Добавить подзадачу')}>{tr('Подзадача')}</Button>
        <Button size="sm" variant="ghost" onclick={() => fileInput?.click()} aria-label={tr('Приложить файл')}>{tr('Файл')}</Button>
        <input bind:this={fileInput} class="td-hidden-input" type="file" multiple tabindex="-1" aria-hidden="true" onchange={(e) => { addFiles([...e.currentTarget.files]); e.currentTarget.value = ''; }} />
        {#if voiceReady}
          <button type="button" bind:this={micButton} class="td-mic" aria-label={tr('Сказать голосом')} title={tr('Сказать голосом')}>
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0" /><path d="M12 17.5V21" /></svg>
          </button>
        {/if}
      </div>
      <Button variant="primary" size="sm" loading={busy} disabled={!text.trim()} onclick={() => composer?.submit()}>{tr('Добавить')}</Button>
    </div>
  </div>

  <div class="td-bar">
    {#if view === 'board'}
      <div class="td-filters" role="group" aria-label={tr('Фильтр по сроку')}>
        {#each FILTERS as f (f)}
          <button type="button" class="k-chip" aria-pressed={filter === f} onclick={() => (filter = f)}>
            {filterName(f)}{f === 'overdue' && counts.overdue ? ` (${counts.overdue})` : ''}
          </button>
        {/each}
      </div>
    {:else}
      <h4 class="td-h">{tr('Архив')}</h4>
    {/if}
    <input class="k-input td-search" type="search" bind:value={query} placeholder={tr('Поиск по задачам')} aria-label={tr('Поиск по задачам')} />
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
          <ul class="td-list">
            {#each archiveRows as t (t.id)}
              <li class="td-row td-row--done">
                <span class="td-main td-main--static">
                  <span class="td-title">{t.title}</span>
                  {#if t.due_date}<span class="td-meta"><span class="td-chip">{dateLabel(t.due_date, locale())}</span></span>{/if}
                </span>
                <Button size="sm" variant="ghost" onclick={() => restore(t)}>{tr('Вернуть')}</Button>
              </li>
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
              <ul class="td-list">
                {#each section.open as t (t.id)}{@render row(t)}{/each}
              </ul>
            {:else}
              <p class="td-empty">{dragId ? tr('Отпусти здесь') : tr('Пока пусто')}</p>
            {/if}
            {#if section.done.length}
              <button type="button" class="td-done-toggle" aria-expanded={doneOpen[b]} onclick={() => (doneOpen[b] = !doneOpen[b])}>
                <svg class="td-caret" class:td-caret--open={doneOpen[b]} viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
                {tr('Выполнено ({n})', { n: section.done.length })}
              </button>
              {#if doneOpen[b]}
                <ul class="td-list">
                  {#each section.done as t (t.id)}{@render row(t)}{/each}
                </ul>
              {/if}
            {/if}
          </section>
        {/each}
        {#if loaded && !shown}
          <p class="td-empty">{tr('Ничего не найдено')}</p>
        {/if}
      {/if}
    </div>
    {#if selected && view === 'board'}
      {#key selected.id}
        <TodoCard todo={selected} {api} onsaved={put} onclose={() => (selectedId = '')} onarchive={archive} />
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
    color: var(--k-attention-text, #b4321b);
    font: 600 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .td-form {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-3, 12px) var(--k-space-4, 16px);
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2, #f1f0ec);
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
  .td-mic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: var(--k-text-2, #5c5f66);
    cursor: pointer;
  }
  .td-mic:hover {
    background: color-mix(in srgb, var(--k-text, #262931) 10%, transparent);
    color: var(--k-text, #262931);
  }
  .td-mic:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  .td-mic:global([data-state='listening']) {
    background: var(--k-attention, #ff4f3a);
    color: #fff;
  }
  .td-mic:global([data-state='working']),
  .td-mic:global([data-state='downloading']),
  .td-mic:global([data-state='loading']) {
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
    padding: 3px 4px 3px 10px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface, #fff);
    border: 1px solid var(--k-line, #e0ded9);
    font: 600 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    color: var(--k-text, #262931);
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
    padding: 6px 10px;
    border: 1px solid var(--k-line-strong, #cfcdc7);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface, #fff);
    color: var(--k-text, #262931);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .td-sub-input:focus {
    outline: none;
    border-color: var(--k-brand, #025bff);
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  .td-x {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: var(--k-text-2, #5c5f66);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }
  .td-x:hover {
    background: color-mix(in srgb, var(--k-text, #262931) 12%, transparent);
    color: var(--k-text, #262931);
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
  .td-search {
    flex: 0 1 200px;
    min-width: 120px;
  }
  .td-carry {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2, 8px);
    padding: var(--k-space-2, 8px) var(--k-space-4, 16px);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-attention-tint, #ffe8e2);
    color: var(--k-attention-text, #b4321b);
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
    border-color: var(--k-brand, #025bff);
    background: var(--k-brand-tint, #e7efff);
  }
  .td-h {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--k-text, #262931);
    font: 700 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
  }
  .td-count {
    color: var(--k-text-3, #8d8f94);
    font-size: var(--k-text-sm, 13px);
    font-weight: 600;
  }
  .td-empty {
    margin: 0;
    padding: 4px var(--k-space-2, 8px);
    color: var(--k-text-3, #8d8f94);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .td-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .td-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--k-space-3, 12px);
    padding: 8px var(--k-space-3, 12px);
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-surface, #fff);
  }
  .td-row[draggable='true'] {
    cursor: grab;
  }
  .td-row--picked {
    border-color: var(--k-brand, #025bff);
  }
  .td-row--drag {
    opacity: 0.5;
  }
  .td-row--done .td-title {
    color: var(--k-text-3, #8d8f94);
    text-decoration: line-through;
  }
  .td-check {
    display: grid;
    flex: none;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 2px solid var(--k-line-strong, #cfcdc7);
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: background 160ms var(--k-ease-out, ease), border-color 160ms var(--k-ease-out, ease), color 160ms var(--k-ease-out, ease);
  }
  .td-check:hover {
    border-color: var(--k-brand, #025bff);
  }
  .td-check--on {
    border-color: var(--k-brand, #025bff);
    background: var(--k-brand, #025bff);
    color: var(--k-on-brand, #fff);
  }
  .td-check:focus-visible,
  .td-main:focus-visible,
  .td-done-toggle:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  .td-main {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 2px 4px;
    border: 0;
    border-radius: var(--k-radius-xs, 6px);
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .td-main--static {
    cursor: default;
  }
  .td-title {
    color: var(--k-text, #262931);
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
    background: var(--k-surface-2, #f1f0ec);
    color: var(--k-text-2, #5c5f66);
    font: 500 var(--k-text-xs, 12px) / 1.5 var(--k-font-body, system-ui);
  }
  .td-chip--late {
    background: var(--k-attention-tint, #ffe8e2);
    color: var(--k-attention-text, #b4321b);
  }
  .td-chip--repeat {
    background: var(--k-ai-tint, #eeebff);
    color: var(--k-text-2, #5c5f66);
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
  .td-done-toggle {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: 4px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--k-radius-xs, 6px);
    background: none;
    color: var(--k-text-2, #5c5f66);
    font: 600 var(--k-text-sm, 13px) / 1.3 var(--k-font-body, system-ui);
    cursor: pointer;
  }
  .td-caret {
    transition: transform 160ms var(--k-ease-out, ease);
  }
  .td-caret--open {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .td-check,
    .td-caret {
      transition: none;
    }
  }
</style>
