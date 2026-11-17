import { describe, it, expect } from 'vitest';
import { markdown, linkIcon, workspaceRef, fileHref } from './markdown.js';

describe('markdown', () => {
  it('escapes tags from the model', () => {
    expect(markdown('<img src=x onerror=1>')).not.toContain('<img');
  });

  it('renders paragraphs, bold, code and lists', () => {
    const html = markdown('Привет **мир** и `код`\n\n- один\n- два\n\n1. раз\n2. два');
    expect(html).toContain('<strong>мир</strong>');
    expect(html).toContain('<code>код</code>');
    expect(html).toContain('<ul><li>один</li><li>два</li></ul>');
    expect(html).toContain('<ol><li>раз</li><li>два</li></ol>');
  });

  it('renders headings', () => {
    expect(markdown('## Итог\nтекст')).toContain('<h3>Итог</h3><p>текст</p>');
  });

  it('keeps code blocks as they are', () => {
    const html = markdown('```js\nlet a = 1 | 2;\n```');
    expect(html).toBe('<pre><code>let a = 1 | 2;</code></pre>');
  });

  it('renders a GFM table with alignment and inline markup', () => {
    const src = '| Вариант | Цена | Итог |\n|:---|---:|:---:|\n| **А** | 10 | [сайт](https://example.com) |\n| Б | 20 |';
    const html = markdown(src);
    expect(html).toContain('<div class="md-table-wrap"><table class="md-table">');
    expect(html).toContain('<th scope="col">Вариант</th>');
    expect(html).toContain('<th scope="col" class="al-r">Цена</th>');
    expect(html).toContain('<td><strong>А</strong></td>');
    expect(html).toContain('<td class="al-r">10</td>');
    expect(html).toContain('<td class="al-c"><a href="https://example.com"');
    expect(html).toContain('<td class="al-c"></td>'); // short row is padded
    expect(html).not.toContain('|---');
  });

  it('does not treat a lone pipe line as a table', () => {
    expect(markdown('a | b\nпросто текст')).toBe('<p>a | b<br>просто текст</p>');
  });

  it('puts a table straight after a paragraph line', () => {
    const html = markdown('Сравнение:\n| a | b |\n|---|---|\n| 1 | 2 |');
    expect(html.startsWith('<p>Сравнение:</p><div class="md-table-wrap">')).toBe(true);
  });

  it('adds a Telegram icon to Telegram links only', () => {
    for (const href of ['https://t.me/deloist', 'https://telegram.me/x', 'https://web.telegram.org/a/', 'tg://resolve?domain=x']) {
      expect(linkIcon(href)).toContain('<svg');
    }
    expect(linkIcon('https://example.com/t.me')).toBe('');
    const html = markdown('Пиши [сюда](https://t.me/deloist) или https://example.com.');
    expect(html).toContain('class="has-ico"><svg');
    expect(html).toContain('>сюда</a>');
    expect(html).toContain('<a href="https://example.com" target="_blank" rel="noreferrer">https://example.com</a>.');
  });

  it('has icons for the other known hosts, subdomains included', () => {
    for (const href of ['https://x.com/a', 'https://twitter.com/a', 'https://discord.gg/abc', 'https://youtu.be/xyz', 'https://m.youtube.com/watch?v=1', 'https://old.reddit.com/r/x', 'https://vk.com/id1', 'https://github.com/a/b', 'https://habr.com/ru/articles/1/']) {
      expect(linkIcon(href), href).toContain('md-link-ico');
    }
    expect(linkIcon('https://notgithub.com/')).toBe('');
  });

  it('links open the same way as before', () => {
    expect(markdown('[a](https://x.io/p?q=1&r=2)')).toContain('<a href="https://x.io/p?q=1&amp;r=2" target="_blank" rel="noreferrer">a</a>');
  });

  it('renders the X badge for both platform hosts without changing the link', () => {
    for (const host of ['x.com', 'twitter.com']) {
      const container = document.createElement('div');
      container.innerHTML = markdown(`[Profile](https://${host}/name)`);
      const link = container.querySelector('a');
      expect(link.getAttribute('href')).toBe(`https://${host}/name`);
      expect(link.target).toBe('_blank');
      expect(link.rel).toBe('noreferrer');
      expect(link.textContent).toBe('Profile');
      expect(link.querySelector('svg.md-link-ico-x path').getAttribute('d')).toContain('M14.234 10.162');
      expect(link.querySelector('svg').getAttribute('aria-hidden')).toBe('true');
    }
    expect(linkIcon('https://x.com.evil.test/name')).toBe('');
  });
});

describe('local workspace file links', () => {
  const ws = 'C:/Users/Public/Deloist/data/workspaces/bot_b24b7f86c24671ca';

  it('renders a link to a workspace file as an in-app file link, not a raw URL', () => {
    const html = markdown(`лежат в [заметках](${ws}/x-analiz-konkurentov.md).`);
    expect(html).toContain('class="md-file"');
    expect(html).toContain(`data-file="${ws}/x-analiz-konkurentov.md"`);
    expect(html).toContain('>заметках</a>');
    expect(html).not.toContain('](');
    expect(html).not.toContain('target=');
  });

  it('accepts backslashes, spaces, Cyrillic and file:/// forms', () => {
    expect(workspaceRef('C:\\Users\\Public\\Deloist\\data\\workspaces\\bot_1\\Мои заметки\\отчёт 1.md')).toEqual({
      owner: 'bot_1',
      rel: 'Мои заметки/отчёт 1.md',
    });
    expect(workspaceRef('file:///C:/data/workspaces/bot_1/%D0%BE%D1%82%D1%87%D1%91%D1%82.md')).toEqual({ owner: 'bot_1', rel: 'отчёт.md' });
    expect(markdown('[x](C:\\d\\workspaces\\bot_1\\a b.md)')).toContain('class="md-file"');
  });

  it('gives file links a real file:/// href so "Copy link" copies the path, not the window address', () => {
    expect(markdown(`[x](${ws}/github-konkurenty.md)`)).toContain(`href="file:///${ws}/github-konkurenty.md"`);
    expect(markdown(`[x](file:///${ws}/a.md)`)).toContain(`href="file:///${ws}/a.md"`);
    expect(markdown(`[x](${ws}/a.md)`)).not.toContain('href="#"');
    expect(fileHref('C:\\d\\workspaces\\bot_1\\Мои заметки\\#1?.md')).toBe(
      'file:///C:/d/workspaces/bot_1/%D0%9C%D0%BE%D0%B8%20%D0%B7%D0%B0%D0%BC%D0%B5%D1%82%D0%BA%D0%B8/%231%3F.md',
    );
    expect(fileHref('file:///C:/d/workspaces/bot_1/%D0%BE.md')).toBe('file:///C:/d/workspaces/bot_1/%D0%BE.md');
  });

  it('leaves links outside bot workspaces and traversal as plain text', () => {
    expect(markdown('[x](C:/Windows/system32/cmd.exe)')).not.toContain('md-file');
    expect(markdown(`[x](${ws}/../../secret.txt)`)).not.toContain('md-file');
    expect(workspaceRef('/etc/workspaces/bot_1/a.md')).toBeNull();
    expect(workspaceRef('javascript:alert(1)')).toBeNull();
  });

  it('escapes quotes and tags in the path and label', () => {
    const html = markdown(`[<b>x</b>](${ws}/a"onmouseover="1.md)`);
    expect(html).not.toContain('<b>');
    expect(html).not.toMatch(/data-file="[^"]*"onmouseover/);
  });

  it('in remote mode shows the real address with a source mark and never makes file anchors', () => {
    const html = markdown('[safe](https://evil.example/x) и [файл](C:/Users/me/AppData/workspaces/bot/a.md)', { remote: { mark: 'from another PC' } });
    expect(html).toContain('href="https://evil.example/x"');
    expect(html).toContain('(https://evil.example/x');
    expect(html).toContain('from another PC');
    expect(html).not.toContain('data-file');
    expect(html).not.toContain('md-file');
    expect(markdown('<img src=x onerror=1> https://a.b/c', { remote: { mark: 'm' } })).not.toContain('<img');
    // Обычный режим не изменился.
    expect(markdown('[x](https://a.b/c)')).not.toContain('md-remote-url');
  });
});
