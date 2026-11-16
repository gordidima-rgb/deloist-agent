<script>
  // Вкладка «Файлы» в ящике бота: только чтение. Папки открываются, файл открывает просмотрщик (FileViewer).
  // Данные: GET /api/bots/{id}/files?path=, GET /api/bots/{id}/file?path=.
  import ListItem from '../kit/ListItem.svelte';
  import EmptyState from '../kit/EmptyState.svelte';
  import Button from '../kit/Button.svelte';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { fileType } from '../../lib/fileType.js';
  import FileViewer from './FileViewer.svelte';
  import { FILE_MIME, fileRef } from '../../lib/fileRef.js';

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
  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-linecap="round" stroke-width="1.8" stroke-linejoin="round" aria-hidden="true">
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
  </svg>
{/snippet}

{#snippet fileIcon(type)}
  <svg class="df-icon df-icon--{type}" data-file-type={type} viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {#if type === 'image'}
      <rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8" cy="9" r="1.5"/><path d="m4 18 5-5 3 3 3-4 5 6"/>
    {:else if type === 'audio'}
      <path d="M10 17V5l8-2v12M10 8l8-2"/><ellipse cx="7" cy="18" rx="3" ry="2"/><ellipse cx="15" cy="16" rx="3" ry="2"/>
    {:else if type === 'video'}
      <rect x="3" y="5" width="18" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/>
    {:else}
      <path d="M6 3h8l4 4v14H6zM14 3v4h4" />
      {#if type === 'spreadsheet'}<path d="M8 11h8M8 14h8M8 17h8M12 11v6"/>
      {:else if type === 'presentation'}<path d="M8 10h8v6H8zM12 16v2m-2 0h4"/>
      {:else if type === 'archive'}<path d="M11 4h2m-2 2h2m-2 2h2m-2 2h2m-2 2h2v3h-2z"/>
      {:else if type === 'code'}<path d="m10 11-2 2 2 2m4-4 2 2-2 2m-1-5-2 7"/>
      {:else if type === 'pdf'}<path d="M8 16c2-2 3-5 3-7 1 4 3 6 5 6-3 0-5 1-8 1z"/>
      {:else if type === 'document'}<path d="M8 10h8M8 13h8M8 16h6M8 18h4"/>
      {:else if type === 'text'}<path d="M8 11h8M8 14h8M8 17h5"/>
      {/if}
    {/if}
  </svg>
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
  .df-icon { color: var(--k-text-2); }
  .df-icon--image, .df-icon--presentation { color: var(--k-ai); }
  .df-icon--audio, .df-icon--spreadsheet { color: var(--k-brand-text); }
  .df-icon--video, .df-icon--pdf { color: var(--k-attention-text); }
</style>
