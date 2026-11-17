<script>
  // Вкладка «Память» в ящике бота: memory.md и brain.md как текст, память команд, журнал правок с отменой.
  // Данные: GET /api/bots/{id}/memory. Перечитывание по событию памяти идёт через registerReload(load).
  import { Section, ListItem, Field, TextArea, Button, Switch } from '../kit';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { whenText, numberRu } from '../../lib/when.js';

  let { bot, api, registerReload = null } = $props();

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
      files: tr('Правил memory.md'),
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

  let data = $state(null);
  let fileText = $state('');
  let brainText = $state('');
  let fileSaved = $state('');
  let brainSaved = $state('');
  let savingFile = $state(false);
  let savingBrain = $state(false);
  let expanded = $state({});
  let undoing = $state(null);
  let alive = true;

  const sharedByBot = $derived(Array.isArray(data?.sharedByBot) ? data.sharedByBot : []);
  const teams = $derived(Array.isArray(data?.teams) ? data.teams : []);
  const fileDirty = $derived(fileText !== fileSaved);
  const brainDirty = $derived(brainText !== brainSaved);
  const journal = $derived(Array.isArray(data?.journal) ? data.journal : []);
  const fileHint = $derived(
    tr('Каждая запись — строка «- …», подробности — с отступом. Раздел «Всегда» бот видит в каждом ходе целиком, остальное — первой строкой.')
      + (data?.fullLimit ? ' ' + tr('Пока память меньше {n} знаков, бот видит её целиком.', { n: numberRu(data.fullLimit) }) : ''),
  );
  const brainCount = $derived(
    data?.brainLimit ? tr('{n} из {limit} знаков', { n: numberRu(brainText.length), limit: numberRu(data.brainLimit) }) : '',
  );
  const countText = $derived(
    data?.limit ? tr('{n} из {limit} знаков', { n: numberRu(data.used || 0), limit: numberRu(data.limit) }) : '',
  );

  // Набранный текст не затираем: если поле правили, перечитывание его не трогает.
  async function load() {
    try {
      const next = await api(`/api/bots/${encodeURIComponent(bot.id)}/memory`);
      if (!alive) return;
      data = next;
      const file = next.file ?? '';
      const brain = next.brain ?? '';
      if (fileText === fileSaved) fileText = file;
      if (brainText === brainSaved) brainText = brain;
      fileSaved = file;
      brainSaved = brain;
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

  async function saveFile() {
    savingFile = true;
    try {
      await api(`/api/bots/${encodeURIComponent(bot.id)}/memory-file`, { method: 'PUT', body: { text: fileText } });
      toast(tr('Память сохранена'));
      fileSaved = fileText;
      await load();
    } catch (err) {
      toast(err.message);
    } finally {
      savingFile = false;
    }
  }

  function revertFile() {
    fileText = fileSaved;
  }

  async function saveBrain() {
    savingBrain = true;
    try {
      await api(`/api/bots/${encodeURIComponent(bot.id)}/brain`, { method: 'PUT', body: { text: brainText } });
      toast(tr('Правила сохранены'));
      brainSaved = brainText;
      await load();
    } catch (err) {
      toast(err.message);
    } finally {
      savingBrain = false;
    }
  }

  function revertBrain() {
    brainText = brainSaved;
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

  // «Всегда в ходе»: закреплённый факт идёт в промпт целиком, остальные — заголовком.
  async function setPinned(fact, pinned) {
    try {
      await api(`/api/memory/facts/${encodeURIComponent(fact.id)}`, { method: 'PATCH', body: { pinned } });
      await load();
    } catch (err) {
      toast(err.message);
      await load();
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

{#snippet pinSwitch(fact)}
  <div class="dm-pin">
    <Switch label={tr('Всегда в ходе')} checked={!!fact.pinned} onchange={(value) => setPinned(fact, value)} />
    <span class="dm-note">{tr('Всегда в ходе')}</span>
  </div>
{/snippet}

<div class="dm">
  {#if data}
  <Section title={tr('Память — memory.md')}>
    <Field label={tr('Текст памяти')} hint={fileHint}>
      {#snippet children(f)}<TextArea id={f.id} describedby={f.describedby} rows={10} autosize={false} bind:value={fileText} />{/snippet}
    </Field>
    <div class="dm-bar">
      <span class="dm-note">{countText}</span>
      <div class="dm-form-actions">
        <Button size="sm" variant="ghost" disabled={!fileDirty || savingFile} onclick={revertFile}>{tr('Отменить правку')}</Button>
        <Button size="sm" variant="primary" disabled={!fileDirty || savingFile} onclick={saveFile}>{tr('Сохранить')}</Button>
      </div>
    </div>
  </Section>

  <Section title={tr('Правила — brain.md')}>
    <Field label={tr('Текст правил')} hint={tr('Правила от человека: бот следует им в каждом ходе. Сам бот правил не меняет — только предлагает карточкой.')}>
      {#snippet children(f)}<TextArea id={f.id} describedby={f.describedby} rows={6} autosize={false} bind:value={brainText} />{/snippet}
    </Field>
    <div class="dm-bar">
      <span class="dm-note">{brainCount}</span>
      <div class="dm-form-actions">
        <Button size="sm" variant="ghost" disabled={!brainDirty || savingBrain} onclick={revertBrain}>{tr('Отменить правку')}</Button>
        <Button size="sm" variant="primary" disabled={!brainDirty || savingBrain} onclick={saveBrain}>{tr('Сохранить')}</Button>
      </div>
    </div>
  </Section>

  {/if}

  {#each teams as team (team.id)}
    <Section title={tr('Память команды «{name}» · {n}', { name: team.name, n: (team.facts || []).length })}>
      <p class="dm-note">{tr('Эти факты видят только боты этой команды. Договорённости команды бот пишет сюда сам (remember со scope team).')}</p>
      {#each team.facts || [] as fact (fact.id)}
        {#snippet teamActions()}
          <div class="dm-actions">
            <Button size="sm" variant="ghost" onclick={() => remove(fact)}>{tr('Удалить')}</Button>
          </div>
        {/snippet}
        <div class="dm-fact">
          <ul class="k-list">
            <ListItem title={fact.title} meta={`[${fact.id}]`} onclick={() => toggle(fact.id)} context={teamActions} />
          </ul>
          {@render pinSwitch(fact)}
          {#if expanded[fact.id] && fact.body}
            <div class="dm-detail"><p class="dm-body">{fact.body}</p></div>
          {/if}
        </div>
      {/each}
    </Section>
  {/each}

  {#if sharedByBot.length}
    <Section title={tr('Записал в общую память · {n}', { n: sharedByBot.length })}>
      <p class="dm-note">{tr('Эти факты видят все боты. Править их можно на экране общей памяти.')}</p>
      {#each sharedByBot as fact (fact.id)}
        <div class="dm-fact">
          <ul class="k-list">
            <ListItem title={fact.title} meta={`[${fact.id}]`} onclick={() => toggle(fact.id)} />
          </ul>
          {#if expanded[fact.id] && fact.body}
            <div class="dm-detail"><p class="dm-body">{fact.body}</p></div>
          {/if}
        </div>
      {/each}
    </Section>
  {/if}

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
  .dm-form-actions { display: flex; justify-content: flex-end; gap: var(--k-space-2); }
  .dm-fact { border-top: 1px solid var(--k-line); }
  .dm-fact:first-child { border-top: 0; }
  .dm-pin { display: flex; align-items: center; gap: var(--k-space-2); padding: 0 var(--k-space-3) var(--k-space-2); }
  .dm-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--k-space-1); }
  .dm-detail { display: flex; flex-direction: column; gap: var(--k-space-2); padding: 0 var(--k-space-3) var(--k-space-3); }
  .dm-body {
    margin: 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .dm-journal { display: flex; flex-direction: column; }
  .dm-jr { display: flex; flex-direction: column; gap: 4px; padding: var(--k-space-2) 0; border-top: 1px solid var(--k-line); }
  .dm-jr:first-child { border-top: 0; }
  .dm-jr-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px var(--k-space-3); }
  .dm-jr-meta { color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.3 var(--k-font-body); font-variant-numeric: tabular-nums; }
  .dm-jr-title { margin: 0; color: var(--k-text); font: 600 var(--k-text-sm) / 1.3 var(--k-font-body); }
  .dm-line { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); overflow-wrap: anywhere; }
</style>
