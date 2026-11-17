import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerFiles from './DrawerFiles.svelte';
import { fileType } from '../../lib/fileType.js';

const listings = {
  '': { items: [{ name: 'notes', path: 'notes', dir: true }, { name: 'a.txt', path: 'a.txt', dir: false }] },
  notes: { items: [{ name: 'day.md', path: 'notes/day.md', dir: false }] },
};

function fakeApi() {
  return vi.fn(async (path) => {
    if (path === '/api/bots/b1/files?path=') return listings[''];
    if (path === '/api/bots/b1/files?path=notes') return listings.notes;
    if (path === '/api/bots/b1/file?path=a.txt') return { text: 'hello' };
    throw new Error(`unexpected request: ${path}`);
  });
}

let handle;
let host;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerFiles, host, props);
}

async function waitFor(check) {
  await vi.waitFor(() => {
    flushSync();
    check();
  });
}

const rowByName = (name) => [...host.querySelectorAll('.k-list-item__main')].find((el) => el.textContent.includes(name));
const buttonByText = (root, text) => [...root.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

afterEach(() => {
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
});

describe('DrawerFiles', () => {
  it('classifies common formats without treating bare dotfiles as extensions', () => {
    for (const [name, type] of Object.entries({
      'photo.JPG': 'image', 'sound.wav': 'audio', 'movie.MP4': 'video', 'letter.docx': 'document',
      'book.pdf': 'pdf', 'table.csv': 'spreadsheet', 'talk.pptx': 'presentation', 'backup.tar.gz': 'archive',
      'generate_apple_music.py': 'code', 'AGENTS.md': 'text', '.env': 'unknown', '.config.json': 'code',
      'README': 'unknown', 'file.': 'unknown', 'file.what': 'unknown',
    })) expect(fileType(name), name).toBe(type);
  });

  it('renders distinct decorative icons for file formats and folders', async () => {
    const names = ['apple.jpg', 'apple_music.wav', 'generate_apple_music.py', 'AGENTS.md', 'a.pdf'];
    const api = vi.fn(async () => ({ items: [
      { name: 'folder', path: 'folder', dir: true },
      ...names.map((name) => ({ name, path: name, dir: false })),
    ] }));
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(rowByName('a.pdf')).toBeTruthy());
    const icons = [...host.querySelectorAll('.k-list-item__avatar svg')];
    expect(icons.map((icon) => icon.getAttribute('data-file-type'))).toEqual([null, 'image', 'audio', 'code', 'text', 'pdf']);
    expect(icons.every((icon) => icon.getAttribute('aria-hidden') === 'true')).toBe(true);
    expect(new Set(icons.map((icon) => icon.innerHTML)).size).toBe(icons.length);
  });

  it('shows MAIN.md first in the root folder', async () => {
    const api = vi.fn(async () => ({ items: [
      { name: 'notes', path: 'notes', dir: true },
      { name: 'a.txt', path: 'a.txt', dir: false },
      { name: 'MAIN.md', path: 'MAIN.md', dir: false },
    ] }));
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(rowByName('MAIN.md')).toBeTruthy());
    const first = host.querySelector('.k-list-item');
    expect(first.textContent).toContain('MAIN.md');
  });

  it('opens a folder and shows its path in the breadcrumb', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(rowByName('notes')).toBeTruthy());
    rowByName('notes').click();
    await waitFor(() => {
      expect(api).toHaveBeenCalledWith('/api/bots/b1/files?path=notes');
      expect(host.querySelector('.df-path').textContent).toContain('notes');
      expect(rowByName('day.md')).toBeTruthy();
    });
  });

  it('shows the text of a file in the preview', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(rowByName('a.txt')).toBeTruthy());
    rowByName('a.txt').click();
    await waitFor(() => {
      expect(api).toHaveBeenCalledWith('/api/bots/b1/file?path=a.txt');
      expect(host.querySelector('.fv-code .fv-text').textContent).toBe('hello');
    });
  });

  it('the up button goes to the parent folder', async () => {
    const api = fakeApi();
    mount({ bot: { id: 'b1' }, api });
    await waitFor(() => expect(rowByName('notes')).toBeTruthy());
    rowByName('notes').click();
    await waitFor(() => expect(rowByName('day.md')).toBeTruthy());
    buttonByText(host, '↑ вверх').click();
    await waitFor(() => {
      expect(api).toHaveBeenLastCalledWith('/api/bots/b1/files?path=');
      expect(rowByName('notes')).toBeTruthy();
    });
  });
});

describe('DrawerFiles drag to chat', () => {
  it('a file row carries its path as text and as a file reference', async () => {
    mount({ bot: { id: 'b1' }, api: fakeApi() });
    await waitFor(() => expect(rowByName('a.txt')).toBeTruthy());
    const row = rowByName('a.txt').closest('li');
    expect(row.getAttribute('draggable')).toBe('true');
    const data = new Map();
    const event = new Event('dragstart', { bubbles: true });
    event.dataTransfer = { setData: (type, value) => data.set(type, value), effectAllowed: '' };
    row.dispatchEvent(event);
    expect(data.get('text/plain')).toBe('`a.txt`');
    expect(JSON.parse(data.get('application/x-deloist-file'))).toEqual({ botId: 'b1', path: 'a.txt', dir: false, ref: '`a.txt`' });
  });

  it('a folder reference ends with a slash', async () => {
    mount({ bot: { id: 'b1' }, api: fakeApi() });
    await waitFor(() => expect(rowByName('notes')).toBeTruthy());
    const data = new Map();
    const event = new Event('dragstart', { bubbles: true });
    event.dataTransfer = { setData: (type, value) => data.set(type, value), effectAllowed: '' };
    rowByName('notes').closest('li').dispatchEvent(event);
    expect(data.get('text/plain')).toBe('`notes/`');
  });
});
