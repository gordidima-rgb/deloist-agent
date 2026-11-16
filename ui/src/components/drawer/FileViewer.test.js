import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import FileViewer from './FileViewer.svelte';
import { highlightLines, languageOf } from '../../lib/highlight.js';

let handle;
let host;

function mount(file, api) {
  host = document.createElement('div');
  document.body.append(host);
  const onclose = vi.fn();
  handle = mountComponent(FileViewer, host, { bot: { id: 'b1' }, api, file, onclose });
  return onclose;
}

async function waitFor(check) {
  await vi.waitFor(() => {
    flushSync();
    check();
  });
}

const buttonByText = (text) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === text);
const apiWith = (text) => vi.fn(async () => ({ text }));

afterEach(() => {
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
  document.body.classList.remove('drawer-wide');
});

describe('highlight', () => {
  it('picks a language by file name', () => {
    expect(languageOf('a.html')).toBe('html');
    expect(languageOf('src/Main.RS')).toBe('rs');
    expect(languageOf('Makefile')).toBe('make');
    expect(languageOf('notes.txt')).toBe(null);
    expect(languageOf('.env')).toBe('ini');
  });

  it('escapes the text and keeps one HTML string per line', () => {
    const lines = highlightLines('const a = "<b>";\n// x & y\n', 'js');
    expect(lines).toHaveLength(2);
    expect(lines[0]).toContain('shj-syn-');
    expect(lines.join('')).not.toContain('<b>');
    expect(lines[1]).toContain('&amp;');
  });

  it('closes a multi-line token on every line', () => {
    const lines = highlightLines('/* one\ntwo */\nlet x', 'js');
    expect(lines).toHaveLength(3);
    for (const line of lines.slice(0, 2)) expect((line.match(/<span/g) || []).length).toBe((line.match(/<\/span>/g) || []).length);
  });

  it('returns plain escaped lines for an unknown language', () => {
    expect(highlightLines('a <b>\nc', null)).toEqual(['a &lt;b&gt;', 'c']);
  });
});

describe('FileViewer', () => {
  it('shows code with line numbers and highlighting', async () => {
    mount({ name: 'page.css', path: 'page.css' }, apiWith('a { color: red; }\nb { margin: 0 }\n'));
    await waitFor(() => expect(host.querySelectorAll('.fv-line')).toHaveLength(2));
    expect(host.querySelector('.fv-no').textContent).toBe('1');
    expect(host.querySelector('.fv-text').innerHTML).toContain('shj-syn-');
  });

  it('never turns file text into markup', async () => {
    for (const name of ['evil.txt', 'evil.html', 'evil.md']) {
      mount({ name, path: name }, apiWith('<img src=x onerror=alert(1)>\n<script>1</script>'));
      await waitFor(() => expect(host.querySelector('.fv-body').textContent.trim().length).toBeGreaterThan(0));
      if (name === 'evil.md') buttonByText('Исходник').click();
      await waitFor(() => expect(host.querySelector('.fv-text').textContent).toContain('<img src=x'));
      expect(host.querySelector('.fv-body img, .fv-body script')).toBeNull();
      handle.destroy();
      host.remove();
    }
  });

  it('renders markdown by default and can switch to the source', async () => {
    mount({ name: 'x.md', path: 'x.md' }, apiWith('# Title\n\ntext'));
    await waitFor(() => expect(host.querySelector('.fv-md h2')?.textContent).toBe('Title'));
    buttonByText('Исходник').click();
    await waitFor(() => {
      expect(host.querySelector('.fv-md')).toBeNull();
      expect(host.querySelector('.fv-text').textContent).toBe('# Title');
    });
  });

  it('shows a CSV as a table', async () => {
    mount({ name: 'd.csv', path: 'd.csv' }, apiWith('name,qty\n"a, b",2\n'));
    await waitFor(() => expect(host.querySelectorAll('tbody tr')).toHaveLength(1));
    expect([...host.querySelectorAll('th')].map((t) => t.textContent)).toEqual(['name', 'qty']);
    expect(host.querySelector('tbody td').textContent).toBe('a, b');
  });

  it('shows an image by its raw address without reading the text', async () => {
    const api = apiWith('x');
    mount({ name: 'p.png', path: 'pics/p.png' }, api);
    await waitFor(() => expect(host.querySelector('img')?.getAttribute('src')).toBe('/api/bots/b1/file/raw?path=pics%2Fp.png'));
    expect(api).not.toHaveBeenCalled();
  });

  it('offers only a download for formats without a preview', async () => {
    const api = apiWith('x');
    mount({ name: 'a.pdf', path: 'a.pdf' }, api);
    await waitFor(() => expect(host.querySelector('.fv-note')).toBeTruthy());
    expect(api).not.toHaveBeenCalled();
    expect(host.querySelector('a[download]').getAttribute('href')).toContain('download=1');
  });

  it('says so when the file is not text', async () => {
    mount({ name: 'blob.bin', path: 'blob.bin' }, apiWith('�'.repeat(40)));
    await waitFor(() => expect(host.querySelector('.fv-note')?.textContent).toContain('не читается как текст'));
  });

  it('reports a read error', async () => {
    mount({ name: 'a.txt', path: 'a.txt' }, vi.fn(async () => { throw new Error('нет доступа'); }));
    await waitFor(() => expect(host.querySelector('.fv-err')?.textContent).toBe('нет доступа'));
  });

  it('widens the drawer and gives it back on close', async () => {
    mount({ name: 'a.txt', path: 'a.txt' }, apiWith('hi'));
    await waitFor(() => expect(buttonByText('Шире')).toBeTruthy());
    buttonByText('Шире').click();
    await waitFor(() => expect(document.body.classList.contains('drawer-wide')).toBe(true));
    handle.destroy();
    handle = null;
    expect(document.body.classList.contains('drawer-wide')).toBe(false);
  });

  it('sends the selected lines with a comment to the chat', async () => {
    const onquote = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    handle = mountComponent(FileViewer, host, { bot: { id: 'b1' }, api: apiWith('one\ntwo\nthree\n'), file: { name: 'a.txt', path: 'dir/a.txt' }, onclose: vi.fn(), onquote });
    await waitFor(() => expect(host.querySelectorAll('.fv-line')).toHaveLength(3));
    const first = host.querySelectorAll('.fv-text')[0].firstChild;
    const second = host.querySelectorAll('.fv-text')[1].firstChild;
    const range = document.createRange();
    range.setStart(first, 0);
    range.setEnd(second, 3);
    range.getBoundingClientRect = () => ({ left: 10, bottom: 20 });
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    host.querySelector('.fv-body').dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    await waitFor(() => expect(host.querySelector('.fv-pop')).toBeTruthy());
    const box = host.querySelector('.fv-pop textarea');
    box.value = 'поправь';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    buttonByText('В чат').click();
    expect(onquote).toHaveBeenCalledTimes(1);
    const { text } = onquote.mock.calls[0][0];
    expect(text).toContain('Файл `dir/a.txt`, строки 1–2:');
    // jsdom не ставит перенос между блоками выделения, браузер ставит: проверяем только содержимое.
    // Ещё jsdom не знает user-select: none и кладёт в выделение номер строки; браузер его не кладёт.
    expect(text).toMatch(/one.*two/s);
    expect(text).not.toContain('three');
    expect(text).toContain('Комментарий: поправь');
    flushSync();
    expect(host.querySelector('.fv-pop')).toBeNull();
  });

  it('opens the file in VS Code with the first selected line', async () => {
    const api = vi.fn(async (path) => (path.includes('/file?') ? { text: 'one two' } : { ok: true }));
    mount({ name: 'a.txt', path: 'dir/a.txt' }, api);
    await waitFor(() => expect(host.querySelector('.fv-line')).toBeTruthy());
    buttonByText('Открыть в VS Code').click();
    await waitFor(() => expect(api).toHaveBeenCalledWith('/api/bots/b1/file/open-in-vscode', { method: 'POST', body: { path: 'dir/a.txt', line: undefined } }));
  });

  it('shows no comment box when the chat cannot take it', async () => {
    mount({ name: 'a.txt', path: 'a.txt' }, apiWith('one'));
    await waitFor(() => expect(host.querySelector('.fv-line')).toBeTruthy());
    const node = host.querySelector('.fv-text').firstChild;
    const range = document.createRange();
    range.selectNodeContents(node);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
    host.querySelector('.fv-body').dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    flushSync();
    expect(host.querySelector('.fv-pop')).toBeNull();
  });

  it('goes back to the list with the button and with Escape', async () => {
    const onclose = mount({ name: 'a.txt', path: 'a.txt' }, apiWith('hi'));
    await waitFor(() => expect(host.querySelector('.fv-line')).toBeTruthy());
    buttonByText('← Файлы').click();
    host.querySelector('.fv').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(onclose).toHaveBeenCalledTimes(2);
  });
});
