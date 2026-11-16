<script>
  // Вкладка «Память» в ящике бота: факты по типам, правка в строке, журнал правок с отменой.
  // Данные: GET /api/bots/{id}/memory. Перечитывание по событию памяти идёт через registerReload(load).
  import Section from '../kit/Section.svelte';
  import ListItem from '../kit/ListItem.svelte';
  import EmptyState from '../kit/EmptyState.svelte';
  import Field from '../kit/Field.svelte';
  import TextInput from '../kit/TextInput.svelte';
  import TextArea from '../kit/TextArea.svelte';
  import Select from '../kit/Select.svelte';
  import Button from '../kit/Button.svelte';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { whenText, numberRu } from '../../lib/when.js';

  let { bot, api, registerReload = null } = $props();

  const KINDS = ['user', 'feedback', 'project', 'reference'];

  function kindLabel(kind) {
    const labels = {
      user: tr('О человеке'),
      feedback: tr('Поправка'),
      project: tr('Дела и решения'),
      reference: tr('Ссылки и кабинеты'),
    };
    return labels[kind] || kind;
  }
  const kindOptions = $derived(KINDS.map((value) => ({ value, label: kindLabel(value) })));

  function whoText(who) {
    if (!who) return '';
    if (who.kind === 'bot') return who.name || tr('Бот');
    if (who.kind === 'human') return tr('Ты');
    if (who.kind === 'doctor') return tr('Доктор памяти');
    if (who.kind === 'account') return who.name || tr('Аккаунт');
    return who.name || '';
  }

  function actionText(entry) {
    const map = {
      add: tr('Дописал в память'),
      edit: tr('Поправил память'),
      doctor: tr('Навёл порядок'),
      undo: tr('Отменил правку'),
      remove: tr('Убрал из памяти'),
    };
    return map[entry.action] || '';
  }

  function journalLines(entry) {
    return [
      ...(entry.added || []).slice(0, 3).map((t) => `+ ${t}`),
      ...(entry.removed || []).slice(0, 3).map((t) => `− ${t}`),
      ...(entry.edited || []).slice(0, 3).map((t) => `~ ${t}`),
    ];
  }

  function blankDraft(fact = null) {
    return {
      title: fact?.title || '',
      body: fact?.body || '',
      kind: fact?.kind || 'user',
      entities: (fact?.entities || []).join(', '),
    };
  }

  let data = $state(null);
  let adding = $state(false);
  let editing = $state(null);
  let draft = $state(blankDraft());
  let expanded = $state({});
  let undoing = $state(null);
  let alive = true;

  const formOpen = $derived(adding || editing !== null);
  const groups = $derived(
    KINDS.map((kind) => ({ kind, facts: (data?.facts || []).filter((f) => f.kind === kind) })).filter((g) => g.facts.length),
  );
  const journal = $derived(Array.isArray(data?.journal) ? data.journal : []);
  const countText = $derived(
    data?.limit ? tr('{n} из {limit} знаков', { n: numberRu(data.used || 0), limit: numberRu(data.limit) }) : '',
  );

  // Открытую форму не трогаем: перечитывание не должно стереть набранный текст.
  async function load() {
    if (formOpen) return;
    try {
      const next = await api(`/api/bots/${encodeURIComponent(bot.id)}/memory`);
      if (alive && !formOpen) data = next;
    } catch (err) {
      toast(err.message);
    }
  }

  $effect(() => {
    alive = true;
    const register = registerReload;
    register?.(load);
    return () => {
      alive = false;
      register?.(null);
    };
  });

  load();

  function startAdd() {
    if (formOpen) return;
    draft = blankDraft();
    adding = true;
  }

  function startEdit(fact) {
    if (formOpen) return;
    draft = blankDraft(fact);
    editing = fact.id;
  }

  function cancel() {
    adding = false;
    editing = null;
    load();
  }

  async function save(event) {
    event.preventDefault();
    const body = {
      title: draft.title.trim(),
      body: draft.body.trim(),
      kind: draft.kind,
      entities: draft.entities.split(',').map((x) => x.trim()).filter(Boolean),
    };
    try {
      if (editing !== null) await api(`/api/memory/facts/${encodeURIComponent(editing)}`, { method: 'PATCH', body });
      else await api('/api/memory/facts', { method: 'POST', body: { ...body, scope: bot.id } });
      toast(tr('Факт сохранён'));
      cancel();
    } catch (err) {
      toast(err.message);
    }
  }

  async function remove(fact) {
    try {
      await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'DELETE' });
      toast(tr('Удалено. Отменить можно в журнале'));
      await load();
    } catch (err) {
      toast(err.message);
    }
  }

  async function moveShared(fact) {
    try {
      await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'PATCH', body: { scope: 'shared' } });
      toast(tr('Перенёс в общую память'));
      await load();
    } catch (err) {
      toast(err.message);
    }
  }

  async function undo(entry) {
    undoing = entry.id;
    try {
      await api(`/api/memory/undo/${encodeURIComponent(entry.id)}`, { method: 'POST' });
      await load();
    } catch (err) {
      toast(err.message);
    } finally {
      undoing = null;
    }
  }

  function toggle(id) {
    expanded[id] = !expanded[id];
  }
</script>

{#snippet factForm()}
  <form class="dm-form" onsubmit={save}>
    <Field label={tr('Заголовок')}>
      {#snippet children(f)}<TextInput id={f.id} bind:value={draft.title} required />{/snippet}
    </Field>
    <Field label={tr('Текст')}>
      {#snippet children(f)}<TextArea id={f.id} rows={4} autosize={false} bind:value={draft.body} />{/snippet}
    </Field>
    <div class="dm-pair">
      <Field label={tr('Тип')}>
        {#snippet children(f)}<Select id={f.id} options={kindOptions} bind:value={draft.kind} />{/snippet}
      </Field>
      <Field label={tr('Про что (через запятую)')}>
        {#snippet children(f)}<TextInput id={f.id} bind:value={draft.entities} />{/snippet}
      </Field>
    </div>
    <div class="dm-form-actions">
      <Button size="sm" variant="ghost" onclick={cancel}>{tr('Отмена')}</Button>
      <Button size="sm" variant="primary" type="submit">{tr('Сохранить')}</Button>
    </div>
  </form>
{/snippet}

{#snippet factRow(fact)}
  {#snippet actions()}
    <div class="dm-actions">
      <Button size="sm" variant="ghost" onclick={() => startEdit(fact)}>{tr('Изменить')}</Button>
      <Button size="sm" variant="ghost" onclick={() => remove(fact)}>{tr('Удалить')}</Button>
      <Button size="sm" variant="ghost" onclick={() => moveShared(fact)}>{tr('В общую память')}</Button>
    </div>
  {/snippet}
  <div class="dm-fact">
    <ul class="k-list">
      <ListItem title={fact.title} meta={`[${fact.id}]`} onclick={() => toggle(fact.id)} context={actions} />
    </ul>
    {#if expanded[fact.id]}
      <div class="dm-detail">
        {#if fact.body}<p class="dm-body">{fact.body}</p>{/if}
        {#if fact.entities?.length}
          <div class="dm-ents">
            {#each fact.entities as name (name)}<span class="k-chip dm-ent">{name}</span>{/each}
          </div>
        {/if}
        {#if fact.links?.length}<p class="dm-note">{tr('связано: {ids}', { ids: fact.links.join(', ') })}</p>{/if}
      </div>
    {/if}
  </div>
{/snippet}

<div class="dm">
  <p class="dm-note">{tr('Память подмешивается в каждый ход. Бот дописывает сюда сам через инструмент {tool}.', { tool: 'remember' })}</p>

  <div class="dm-bar">
    <span class="dm-note">{countText}</span>
    <Button size="sm" variant="ghost" disabled={formOpen} onclick={startAdd}>{tr('Добавить факт')}</Button>
  </div>

  {#if adding}{@render factForm()}{/if}

  {#if data && !groups.length}
    <EmptyState title={tr('Фактов пока нет.')} />
  {/if}

  {#each groups as group (group.kind)}
    <Section title={`${kindLabel(group.kind)} · ${group.facts.length}`}>
      {#each group.facts as fact (fact.id)}
        {#if editing === fact.id}
          {@render factForm()}
        {:else}
          {@render factRow(fact)}
        {/if}
      {/each}
    </Section>
  {/each}

  <Section title={tr('Журнал')}>
    {#if journal.length}
      <div class="dm-journal">
        {#each journal as entry (entry.id)}
          <div class="dm-jr">
            <div class="dm-jr-head">
              <span class="dm-jr-meta">{whenText(entry.at)}</span>
              <span class="dm-jr-meta">{whoText(entry.who)}</span>
              {#if entry.undoable}
                <Button size="sm" variant="ghost" disabled={undoing === entry.id} onclick={() => undo(entry)}>{tr('Отменить')}</Button>
              {/if}
            </div>
            <p class="dm-jr-title">{entry.note || actionText(entry)}</p>
            {#each journalLines(entry) as line, i (i)}<p class="dm-line">{line}</p>{/each}
          </div>
        {/each}
      </div>
    {:else}
      <p class="dm-note">{tr('Пока без правок.')}</p>
    {/if}
  </Section>
</div>

<style>
  .dm { display: flex; flex-direction: column; gap: var(--k-space-4); min-width: 0; }
  .dm-note { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); overflow-wrap: anywhere; }
  .dm-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--k-space-3); }
  .dm-form {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line-strong);
    border-radius: var(--k-radius-md);
    background: var(--k-surface);
  }
  .dm-pair { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--k-space-3); }
  .dm-form-actions { display: flex; justify-content: flex-end; gap: var(--k-space-2); }
  .dm-fact { border-top: 1px solid var(--k-line); }
  .dm-fact:first-child { border-top: 0; }
  .dm-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--k-space-1); }
  .dm-detail { display: flex; flex-direction: column; gap: var(--k-space-2); padding: 0 var(--k-space-3) var(--k-space-3); }
  .dm-body {
    margin: 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .dm-ents { display: flex; flex-wrap: wrap; gap: var(--k-space-1); }
  .dm-ent {
    border: 1px solid var(--k-line-strong);
    background: var(--k-surface);
    color: var(--k-text-2);
    font: 500 var(--k-text-xs) / 1.3 var(--k-font-body);
  }
  .dm-journal { display: flex; flex-direction: column; }
  .dm-jr { display: flex; flex-direction: column; gap: 4px; padding: var(--k-space-2) 0; border-top: 1px solid var(--k-line); }
  .dm-jr:first-child { border-top: 0; }
  .dm-jr-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px var(--k-space-3); }
  .dm-jr-meta { color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.3 var(--k-font-body); font-variant-numeric: tabular-nums; }
  .dm-jr-title { margin: 0; color: var(--k-text); font: 600 var(--k-text-sm) / 1.3 var(--k-font-body); }
  .dm-line { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); overflow-wrap: anywhere; }
  @media (max-width: 520px) {
    .dm-pair { grid-template-columns: 1fr; }
  }
</style>
