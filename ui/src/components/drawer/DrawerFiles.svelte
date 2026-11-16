<script>
  // Вкладка «Файлы» в ящике бота: только чтение. Папки открываются, файл открывает просмотрщик (FileViewer).
  // Данные: GET /api/bots/{id}/files?path=, GET /api/bots/{id}/file?path=.
  import ListItem from '../kit/ListItem.svelte';
  import EmptyState from '../kit/EmptyState.svelte';
  import Button from '../kit/Button.svelte';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { fileType } from '../../lib/fileType.js';
  import Icon from '../kit/Icon.svelte';
  import FileViewer from './FileViewer.svelte';
  import { FILE_MIME, fileRef } from '../../lib/fileRef.js';

  // Тип файла (lib/fileType.js) -> значок из lib/icons.js; незнакомый тип рисуется общим листом.
  const FILE_ICONS = Object.fromEntries(['image', 'audio', 'video', 'document', 'pdf', 'spreadsheet', 'presentation', 'archive', 'code', 'text'].map((type) => [type, `file-${type}`]));

  let { bot, api, initial = '', onquote } = $props();

  let path = $state('');
  let items = $state([]);
  let loaded = $state(false);
  let viewing = $state(null);

  const crumbs = $derived(path ? path.split('/') : []);

  async function load(rel) {
    try {
      const data = await api(`/api/bots/${encodeURIComponent(bot.id)}/files?path=${encodeURIComponent(rel)}`);
      path = rel;
      items = data.items || [];
      loaded = true;
    } catch (err) {
      toast(err.message);
    }
  }

  function open(item) {
    if (item.dir) return load(item.path);
    viewing = { name: item.name, path: item.path };
  }

  // Файл или папку тащат в чат: в сообщение встаёт ссылка на них, писать путь руками не нужно.
  // text/plain — чтобы и поле ввода само вставило её туда, куда бросили; свой тип — для остального чата.
  function dragFile(event, item) {
    const ref = fileRef(item);
    event.dataTransfer?.setData('text/plain', ref);
    event.dataTransfer?.setData(FILE_MIME, JSON.stringify({ botId: bot.id, path: item.path, dir: Boolean(item.dir), ref }));
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy';
  }

  const up = () => load(path.split('/').slice(0, -1).join('/'));
  const crumbPath = (index) => crumbs.slice(0, index + 1).join('/');

  // Ссылка из ответа бота: показать папку файла и сам файл.
  async function start() {
    const rel = String(initial || '').replace(/^\/+/, '');
    if (!rel) return load('');
    const parts = rel.split('/');
    await load(parts.slice(0, -1).join('/'));
    open({ name: parts[parts.length - 1], path: rel, dir: false });
  }

  start();
</script>

{#snippet folderIcon()}
  <Icon name="folder" size={18} stroke={1.8} />
{/snippet}

{#snippet fileIcon(type)}
  <Icon name={FILE_ICONS[type] ?? 'file'} size={18} stroke={1.7} class="df-icon df-icon--{type}" data-file-type={type} />
{/snippet}

{#if viewing}
  <FileViewer {bot} {api} {onquote} file={viewing} onclose={() => (viewing = null)} />
{:else}
<div class="df">
  <div class="df-bar">
    <Button size="sm" variant="ghost" disabled={!path} onclick={up}>{tr('↑ вверх')}</Button>
    <nav class="df-path" aria-label={tr('Путь')}>
      <button type="button" class="df-crumb" onclick={() => load('')}>/</button>
      {#each crumbs as name, i (i)}
        <span class="df-sep" aria-hidden="true">/</span>
        <button type="button" class="df-crumb" onclick={() => load(crumbPath(i))}>{name}</button>
      {/each}
    </nav>
    <Button size="sm" variant="ghost" onclick={() => load(path)}>{tr('Обновить')}</Button>
  </div>

  {#if loaded && !items.length}
    <EmptyState title={tr('Пусто. Бот ещё ничего не написал.')} />
  {:else if items.length}
    <ul class="k-list">
      {#each items as item (item.path)}
        {#snippet icon()}{#if item.dir}{@render folderIcon()}{:else}{@render fileIcon(fileType(item.name))}{/if}{/snippet}
        <ListItem
          title={item.name}
          avatar={icon}
          onclick={() => open(item)}
          draggable="true"
          data-path={item.path}
          ondragstart={(e) => dragFile(e, item)}
        />
      {/each}
    </ul>
  {/if}
</div>
{/if}

<style>
  .df { display: flex; flex-direction: column; gap: var(--k-space-3); min-width: 0; }
  .df-bar { display: flex; align-items: center; gap: var(--k-space-2); min-width: 0; }
  .df-path {
    display: flex;
    flex: 1;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    min-width: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-xs) / 1.4 var(--k-font-mono);
    overflow-wrap: anywhere;
  }
  .df-crumb {
    padding: 2px 4px;
    border: 0;
    border-radius: var(--k-radius-xs);
    background: none;
    color: var(--k-text-2);
    font: inherit;
    cursor: pointer;
  }
  .df-crumb:hover { background: var(--k-surface-2); color: var(--k-text); }
  .df-crumb:focus-visible { outline: none; box-shadow: var(--k-focus); }
  .df-sep { color: var(--k-text-3); }
  :global(.df-icon) { color: var(--k-text-2); }
  :global(.df-icon--image), :global(.df-icon--presentation) { color: var(--k-ai); }
  :global(.df-icon--audio), :global(.df-icon--spreadsheet) { color: var(--k-brand-text); }
  :global(.df-icon--video), :global(.df-icon--pdf) { color: var(--k-attention-text); }
</style>
