<script>
  // Просмотрщик файла в ящике бота: текст и код с номерами строк, Markdown с видом «Просмотр»/«Исходник»,
  // таблицы CSV и TSV, картинки, звук и видео. Данные: GET /api/bots/{id}/file?path= (текст, первые 200 000 знаков)
  // и GET /api/bots/{id}/file/raw?path= (файл как есть: картинки, медиа, «Скачать»).
  import Button from '../kit/Button.svelte';
  import Spinner from '../kit/Spinner.svelte';
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import { fileType } from '../../lib/fileType.js';
  import { markdown } from '../../lib/markdown.js';
  import { highlightLines, languageOf } from '../../lib/highlight.js';
  import { formatQuote } from '../../lib/fileQuote.js';

  // onquote({ text }) — отправить выделенное с комментарием в чат; без него выделение просто копируется.
  let { bot, api, file, onclose, onquote } = $props();

  const TEXT_LIMIT = 200_000;
  const ROW_LIMIT = 500;

  const name = $derived(file.name);
  const type = $derived(fileType(file.name));
  const ext = $derived(String(file.name).split('.').pop().toLowerCase());
  const isMarkdown = $derived(ext === 'md' || ext === 'markdown');
  const isTable = $derived(ext === 'csv' || ext === 'tsv');
  const isMedia = $derived(type === 'image' || type === 'audio' || type === 'video');
  // Форматы, у которых нет текстового вида: показываем сообщение и «Скачать».
  const isBinary = $derived(['document', 'pdf', 'presentation', 'archive'].includes(type) || (type === 'spreadsheet' && !isTable));
  const rawUrl = $derived(`/api/bots/${encodeURIComponent(bot.id)}/file/raw?path=${encodeURIComponent(file.path)}`);

  let text = $state('');
  let loading = $state(false);
  let failed = $state('');
  let view = $state('rendered');
  let wrap = $state(false);
  let wide = $state(false);
  let copied = $state(false);

  const truncated = $derived(text.length >= TEXT_LIMIT);
  // Сервер отдаёт нечитаемые байты как U+FFFD: если их много, это не текст.
  const unreadable = $derived(text.length > 0 && (text.match(/�/g)?.length ?? 0) > Math.max(8, text.length / 50));

  function prettyJson(source) {
    if (ext !== 'json' || source.length >= TEXT_LIMIT) return source;
    try {
      const parsed = JSON.parse(source);
      return source.trimEnd().includes('\n') ? source : JSON.stringify(parsed, null, 2);
    } catch {
      return source;
    }
  }

  const shown = $derived(prettyJson(text));
  // Подсветка по расширению (текст экранирует highlightLines); у формата без правил строки остаются простым текстом.
  const lines = $derived(shown.length ? highlightLines(shown, languageOf(file.name)) : []);

  // Простой разбор CSV с кавычками: хватает для просмотра, не для правки.
  function parseTable(source, delimiter) {
    const rows = [];
    let row = [];
    let cell = '';
    let quoted = false;
    for (let i = 0; i < source.length && rows.length <= ROW_LIMIT; i += 1) {
      const ch = source[i];
      if (quoted) {
        if (ch === '"' && source[i + 1] === '"') { cell += '"'; i += 1; }
        else if (ch === '"') quoted = false;
        else cell += ch;
      } else if (ch === '"') quoted = true;
      else if (ch === delimiter) { row.push(cell); cell = ''; }
      else if (ch === '\n' || ch === '\r') {
        if (ch === '\r' && source[i + 1] === '\n') i += 1;
        row.push(cell); cell = '';
        rows.push(row); row = [];
      } else cell += ch;
    }
    if (rows.length <= ROW_LIMIT && (cell || row.length)) { row.push(cell); rows.push(row); }
    return rows;
  }
  const table = $derived(isTable && text ? parseTable(text, ext === 'tsv' ? '\t' : ',') : []);
  const tableCut = $derived(table.length > ROW_LIMIT);
  const tableRows = $derived(table.slice(0, ROW_LIMIT));

  async function loadText() {
    loading = true;
    failed = '';
    try {
      const data = await api(`/api/bots/${encodeURIComponent(bot.id)}/file?path=${encodeURIComponent(file.path)}`);
      text = data.text ?? '';
    } catch (err) {
      failed = err.message || tr('Не удалось открыть файл');
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Новый файл: сбрасываем вид и читаем заново. Медиа и бинарным текст не нужен.
    text = '';
    view = isMarkdown ? 'rendered' : 'source';
    if (!isMedia && !isBinary) loadText();
  });

  $effect(() => {
    document.body.classList.toggle('drawer-wide', wide);
    return () => document.body.classList.remove('drawer-wide');
  });

  // Редактор запускает программа на компьютере (ссылка vscode://file/…): со страницы адрес не открыть. Строка — начало выделения.
  async function openInEditor() {
    try {
      await api(`/api/bots/${encodeURIComponent(bot.id)}/file/open-in-vscode`, { method: 'POST', body: { path: file.path, line: note?.from ?? undefined } });
    } catch (err) {
      toast(err.message || tr('Не удалось открыть в VS Code'));
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      toast(tr('Не удалось скопировать'));
    }
  }

  // Выделение в тексте: рядом появляется поле комментария, а «В чат» кладёт кусок и комментарий в поле ввода чата.
  let root = $state(null);
  let body = $state(null);
  let commentBox = $state(null);
  let note = $state(null);
  let comment = $state('');

  const lineOf = (node) => {
    const el = node?.nodeType === 1 ? node : node?.parentElement;
    const row = el?.closest?.('.fv-line');
    return row ? Number(row.dataset.line) : null;
  };

  function pickSelection() {
    if (!onquote) return;
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !selection.rangeCount) return;
    const range = selection.getRangeAt(0);
    if (!body?.contains(range.commonAncestorContainer)) return;
    const quote = selection.toString().replace(/\u00a0/g, ' ').replace(/^\n+/, '').replace(/\s+$/, '');
    if (!quote.trim()) return;
    let from = lineOf(range.startContainer);
    let to = lineOf(range.endContainer);
    if (from && to && to < from) [from, to] = [to, from];
    // Тройной щелчок захватывает начало следующей строки: её в цитату не считаем.
    if (from && to && to > from && range.endOffset === 0) to -= 1;
    const rect = range.getBoundingClientRect();
    const box = root.getBoundingClientRect();
    const x = Math.max(0, Math.min(rect.left - box.left, box.width - 300));
    note = { quote, from, to, x, y: rect.bottom - box.top + 6 };
    comment = '';
  }

  function cancelNote() {
    note = null;
    comment = '';
  }

  function sendNote() {
    if (!note) return;
    onquote({ text: formatQuote({ path: file.path, from: note.from, to: note.to, quote: note.quote, comment, language: languageOf(file.name) || '' }) });
    window.getSelection()?.removeAllRanges();
    cancelNote();
  }

  $effect(() => {
    if (note) commentBox?.focus();
  });

  function onNoteKey(event) {
    event.stopPropagation();
    if (event.key === 'Escape') cancelNote();
    else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      sendNote();
    }
  }

  function onkeydown(event) {
    if (event.key === 'Escape') onclose?.();
  }
</script>

<div class="fv" role="region" aria-label={name} {onkeydown} bind:this={root}>
  <div class="fv-bar">
    <Button size="sm" variant="ghost" onclick={onclose}>{tr('← Файлы')}</Button>
    <p class="fv-name" title={file.path}>{name}</p>
    <div class="fv-tools">
      {#if isMarkdown}
        <div class="fv-seg" role="group" aria-label={tr('Вид')}>
          <button type="button" class="fv-seg-btn" class:on={view === 'rendered'} aria-pressed={view === 'rendered'} onclick={() => (view = 'rendered')}>{tr('Просмотр')}</button>
          <button type="button" class="fv-seg-btn" class:on={view === 'source'} aria-pressed={view === 'source'} onclick={() => (view = 'source')}>{tr('Исходник')}</button>
        </div>
      {/if}
      {#if !isMedia && !isBinary && !isTable && !(isMarkdown && view === 'rendered')}
        <Button size="sm" variant="ghost" aria-pressed={wrap} onclick={() => (wrap = !wrap)}>{tr('Перенос строк')}</Button>
      {/if}
      {#if !isMedia && !isBinary}
        <Button size="sm" variant="ghost" disabled={!text} onclick={copy}>{copied ? tr('Скопировано') : tr('Копировать')}</Button>
      {/if}
      <Button size="sm" variant="ghost" onclick={openInEditor}>{tr('Открыть в VS Code')}</Button>
      <a class="k-btn k-btn--ghost k-btn--sm" href={`${rawUrl}&download=1`} download={name}>{tr('Скачать')}</a>
      <Button size="sm" variant="ghost" aria-pressed={wide} onclick={() => (wide = !wide)}>{wide ? tr('Уже') : tr('Шире')}</Button>
    </div>
  </div>

  <div class="fv-body" bind:this={body} onmouseup={pickSelection} onkeyup={pickSelection} onmousedown={cancelNote} role="presentation">
    {#if type === 'image'}
      <div class="fv-media"><img src={rawUrl} alt={name} /></div>
    {:else if type === 'audio'}
      <div class="fv-media"><audio src={rawUrl} controls preload="metadata"></audio></div>
    {:else if type === 'video'}
      <div class="fv-media"><video src={rawUrl} controls preload="metadata"><track kind="captions" /></video></div>
    {:else if isBinary}
      <p class="fv-note">{tr('Для этого формата просмотра нет: скачайте файл.')}</p>
    {:else if loading}
      <div class="fv-note"><Spinner /></div>
    {:else if failed}
      <p class="fv-note fv-err">{failed}</p>
    {:else if unreadable}
      <p class="fv-note">{tr('Файл не читается как текст: скачайте его.')}</p>
    {:else if !text}
      <p class="fv-note">{tr('Файл пустой.')}</p>
    {:else if isMarkdown && view === 'rendered'}
      <div class="fv-md">{@html markdown(text)}</div>
    {:else if isTable && view !== 'never'}
      <div class="fv-table-wrap">
        <table class="md-table">
          <thead><tr>{#each tableRows[0] ?? [] as cell, i (i)}<th scope="col">{cell}</th>{/each}</tr></thead>
          <tbody>
            {#each tableRows.slice(1) as row, r (r)}
              <tr>{#each row as cell, c (c)}<td>{cell}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if tableCut}<p class="fv-hint">{tr('Показаны первые {n} строк', { n: ROW_LIMIT })}</p>{/if}
    {:else}
      <div class="fv-code" class:wrap data-testid="fv-code">
        {#each lines as line, i (i)}
          <div class="fv-line" data-line={i + 1}><span class="fv-no" aria-hidden="true">{i + 1}</span><span class="fv-text">{@html line || ' '}</span></div>
        {/each}
      </div>
    {/if}
    {#if truncated && !isMedia && !isBinary}
      <p class="fv-hint">{tr('Показаны первые 200 000 знаков, остальное скачайте.')}</p>
    {/if}
  </div>

  {#if !isMedia && !isBinary && text && !isMarkdown}
    <p class="fv-meta">{tr('{n} строк', { n: lines.length })}</p>
  {/if}

  {#if note}
    <div class="fv-pop" style="left: {note.x}px; top: {note.y}px" role="dialog" aria-label={tr('Комментарий к выделенному')}>
      <p class="fv-pop-quote">{note.quote.length > 90 ? `${note.quote.slice(0, 90)}…` : note.quote}</p>
      <textarea bind:this={commentBox} bind:value={comment} rows="2" placeholder={tr('Что здесь исправить?')} aria-label={tr('Комментарий')} onkeydown={onNoteKey}></textarea>
      <div class="fv-pop-actions">
        <Button size="sm" variant="primary" onclick={sendNote}>{tr('В чат')}</Button>
        <Button size="sm" variant="ghost" onclick={cancelNote}>{tr('Отмена')}</Button>
      </div>
    </div>
  {/if}
</div>

<style>
  .fv { position: relative; display: flex; flex-direction: column; gap: var(--k-space-3); min-width: 0; min-height: 0; flex: 1; }
  .fv-bar { display: flex; flex-wrap: wrap; align-items: center; gap: var(--k-space-2); min-width: 0; }
  .fv-name {
    flex: 1;
    min-width: 80px;
    margin: 0;
    overflow: hidden;
    color: var(--k-text);
    font: 600 var(--k-text-sm) / 1.3 var(--k-font-body);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fv-tools { display: flex; flex-wrap: wrap; align-items: center; gap: 2px; }
  .fv-seg { display: inline-flex; padding: 2px; border-radius: var(--k-radius-sm); background: var(--k-surface-2); }
  .fv-seg-btn {
    padding: 3px 10px;
    border: 0;
    border-radius: var(--k-radius-xs);
    background: none;
    color: var(--k-text-2);
    font: 500 var(--k-text-xs) / 1.4 var(--k-font-body);
    cursor: pointer;
  }
  .fv-seg-btn.on { background: var(--k-surface); color: var(--k-text); box-shadow: var(--k-shadow-1); }
  .fv-seg-btn:focus-visible { outline: none; box-shadow: var(--k-focus); }
  .fv-body {
    flex: 1;
    min-width: 0;
    min-height: 120px;
    overflow: auto;
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md);
    background: var(--k-surface);
  }
  .fv-code { padding: var(--k-space-2) 0; font: 400 var(--k-text-xs) / 1.6 var(--k-font-mono); color: var(--k-text); }
  .fv-line { display: flex; min-width: max-content; }
  .fv-code.wrap .fv-line { min-width: 0; }
  .fv-no {
    flex: none;
    width: 3.5em;
    padding-right: var(--k-space-3);
    color: var(--k-text-3);
    text-align: right;
    user-select: none;
  }
  .fv-text { white-space: pre; padding-right: var(--k-space-3); }
  .fv-text :global(.shj-syn-kwd), .fv-text :global(.shj-syn-section) { color: var(--k-brand-text); font-weight: 600; }
  .fv-text :global(.shj-syn-str), .fv-text :global(.shj-syn-insert) { color: var(--k-ai); }
  .fv-text :global(.shj-syn-num), .fv-text :global(.shj-syn-bool) { color: var(--k-attention-text); }
  .fv-text :global(.shj-syn-cmnt) { color: var(--k-text-3); font-style: italic; }
  .fv-text :global(.shj-syn-type), .fv-text :global(.shj-syn-class) { color: var(--k-brand-text); }
  .fv-text :global(.shj-syn-func) { color: var(--k-ai); font-weight: 500; }
  .fv-text :global(.shj-syn-oper) { color: var(--k-text-2); }
  .fv-text :global(.shj-syn-deleted), .fv-text :global(.shj-syn-err) { color: var(--k-attention-text); text-decoration: line-through; }
  .fv-code.wrap .fv-text { white-space: pre-wrap; overflow-wrap: anywhere; min-width: 0; }
  .fv-md { padding: var(--k-space-3) var(--k-space-4); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); overflow-wrap: anywhere; }
  .fv-md :global(h1) { font-size: var(--k-text-xl, 1.4em); margin: 0.2em 0 0.6em; }
  .fv-md :global(h2) { font-size: var(--k-text-lg, 1.2em); margin: 1em 0 0.5em; }
  .fv-md :global(h3) { font-size: var(--k-text-md, 1.05em); margin: 0.9em 0 0.4em; }
  .fv-md :global(pre) { overflow: auto; padding: var(--k-space-3); border-radius: var(--k-radius-sm); background: var(--k-surface-2); font: 400 var(--k-text-xs) / 1.5 var(--k-font-mono); }
  .fv-md :global(code) { padding: 1px 5px; border-radius: var(--k-radius-xs); background: var(--k-surface-2); color: var(--k-text); font-family: var(--k-font-mono); font-size: 0.92em; }
  .fv-md :global(pre code) { padding: 0; background: none; }
  .fv-md :global(p) { margin: 0 0 0.7em; }
  .fv-md :global(ul), .fv-md :global(ol) { margin: 0 0 0.7em; padding-left: 1.4em; }
  .fv-table-wrap { overflow: auto; }
  .fv-table-wrap :global(table) { min-width: max-content; }
  .fv-media { display: flex; align-items: center; justify-content: center; padding: var(--k-space-3); }
  .fv-media img, .fv-media video { max-width: 100%; max-height: 70vh; object-fit: contain; border-radius: var(--k-radius-sm); }
  .fv-media audio { width: 100%; }
  .fv-note { margin: 0; padding: var(--k-space-5, 24px) var(--k-space-4); color: var(--k-text-2); text-align: center; font: 400 var(--k-text-sm) / 1.5 var(--k-font-body); }
  .fv-err { color: var(--k-attention-text); }
  .fv-hint, .fv-meta { margin: 0; padding: var(--k-space-2) var(--k-space-3); color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); }
  .fv-meta { padding: 0 var(--k-space-1); }
  .fv-pop {
    position: absolute;
    z-index: 5;
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    width: 300px;
    max-width: 100%;
    padding: var(--k-space-3);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
  }
  .fv-pop-quote {
    margin: 0;
    max-height: 3.2em;
    overflow: hidden;
    color: var(--k-text-3);
    font: 400 var(--k-text-xs) / 1.5 var(--k-font-mono);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  .fv-pop textarea {
    width: 100%;
    box-sizing: border-box;
    padding: var(--k-space-2);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-sm);
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 400 var(--k-text-sm) / 1.4 var(--k-font-body);
    resize: vertical;
  }
  .fv-pop textarea:focus { outline: none; border-color: var(--k-brand); box-shadow: var(--k-focus); }
  .fv-pop-actions { display: flex; gap: var(--k-space-2); }
</style>
