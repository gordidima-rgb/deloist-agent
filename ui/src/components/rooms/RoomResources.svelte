<script>
  // Общие папки и навыки команды в окне правки команды. Действуют на каждого бота команды,
  // но только в разговоре этой команды. Сервер проверяет пути при сохранении (src/rooms.rs).
  import { untrack } from 'svelte';
import { Button, Checkbox, TextInput, IconButton } from '../kit';
  import { tr } from '../../lib/i18n.js';

  let { folders = [], skills = [], allSkills = [], max = 8, onchange } = $props();

  let list = $state(untrack(() => [...(folders ?? [])]));
  let chosen = $state(untrack(() => Object.fromEntries((allSkills ?? []).map((s) => [s.id, (skills ?? []).includes(s.id)]))));
  let draft = $state('');
  let note = $state('');

  const emit = () => onchange?.({ folders: [...list], skills: Object.keys(chosen).filter((id) => chosen[id]) });

  function add() {
    const path = draft.trim();
    if (!path) return;
    if (list.length >= max) {
      note = tr('Не больше {n} папок', { n: max });
      return;
    }
    note = '';
    if (!list.some((p) => p.toLowerCase() === path.toLowerCase())) list = [...list, path];
    draft = '';
    emit();
  }

  function remove(path) {
    list = list.filter((p) => p !== path);
    emit();
  }

  function onkey(event) {
    // Enter в строке ввода добавляет папку, а не отправляет окно.
    if (event.key === 'Enter') {
      event.preventDefault();
      add();
    }
  }
</script>

<div class="rr">
  <section class="rr-block" aria-labelledby="rr-folders-title">
    <h4 class="rr-title" id="rr-folders-title">{tr('Папки команды')}</h4>
    <p class="rr-hint">{tr('Каждый бот команды работает в этих папках без вопросов, но только внутри этой команды.')}</p>
    {#if list.length}
      <ul class="rr-list">
        {#each list as path (path)}
          <li class="rr-item">
            <span class="rr-path" title={path}>{path}</span>
            <IconButton class="rr-del" size="sm" name="close" label={tr('Убрать папку {path}', { path })} onclick={() => remove(path)} />
          </li>
        {/each}
      </ul>
    {/if}
    <div class="rr-add">
      <TextInput bind:value={draft} placeholder={tr('Полный путь к папке')} aria-label={tr('Путь к папке')} onkeydown={onkey} />
      <Button size="sm" onclick={add} disabled={!draft.trim()}>{tr('Добавить')}</Button>
    </div>
    {#if note}<p class="rr-note" role="status">{note}</p>{/if}
  </section>

  <section class="rr-block" aria-labelledby="rr-skills-title">
    <h4 class="rr-title" id="rr-skills-title">{tr('Навыки команды')}</h4>
    <p class="rr-hint">{tr('Навыки получает каждый бот команды, но только внутри этой команды.')}</p>
    {#if allSkills.length}
      <ul class="rr-list rr-skills">
        {#each allSkills as skill (skill.id)}
          <li><Checkbox value={skill.id} label={skill.name} bind:checked={chosen[skill.id]} onchange={emit} /></li>
        {/each}
      </ul>
    {:else}
      <p class="rr-hint">{tr('Навыков пока нет: их можно создать в каталоге.')}</p>
    {/if}
  </section>
</div>

<style>
  .rr { display: flex; flex-direction: column; gap: var(--k-space-4); }
  .rr-block { display: flex; flex-direction: column; gap: var(--k-space-2); }
  .rr-title { margin: 0; color: var(--k-text); font: 600 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); }
  .rr-hint, .rr-note { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); }
  .rr-note { color: var(--k-text-2); }
  .rr-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: var(--k-space-1) var(--k-space-3);
    list-style: none;
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface);
  }
  .rr-list li { padding: var(--k-space-2) 0; }
  .rr-list li + li { border-top: 1px solid var(--k-line); }
  .rr-skills :global(.k-check) { position: relative; }
  .rr-item { display: flex; align-items: center; gap: var(--k-space-2); }
  .rr-path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--k-text-2); font: 400 var(--k-text-sm) / 1.4 var(--k-font-mono, monospace); }
  .rr-add { display: flex; gap: var(--k-space-2); align-items: center; }
  .rr-add :global(.k-input) { flex: 1; min-width: 0; }
</style>
