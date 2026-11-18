import { describe, it, expect } from 'vitest';
import { markdownToText, escapeHtml, imageUrl, parseSection, buildPages, buildFullList, parseNotes, mergeSections, splitLead } from './whatsNew.js';

describe('markdownToText', () => {
  it('escapes HTML', () => {
    expect(markdownToText('<script>alert(1)</script>')).toBe('&lt;script&gt;alert(1)&lt;/script&gt;');
  });
  it('allows bold', () => {
    expect(markdownToText('**bold**')).toBe('<strong>bold</strong>');
  });
  it('escapes inside bold', () => {
    expect(markdownToText('**<b>x</b>**')).toBe('<strong>&lt;b&gt;x&lt;/b&gt;</strong>');
  });
  it('renders code without touching its content', () => {
    expect(markdownToText('run `a **b** <i>`')).toBe('run <code>a **b** &lt;i&gt;</code>');
  });
  it('renders only http(s) links', () => {
    expect(markdownToText('[a](https://x.ru/p?q=1&r="2")')).toContain('href="https://x.ru/p?q=1&amp;r=&quot;2&quot;"');
    expect(markdownToText('[a](javascript:alert(1))')).toBe('[a](javascript:alert(1))');
  });
  it('escapeHtml handles null', () => {
    expect(escapeHtml(null)).toBe('');
  });
});

describe('imageUrl', () => {
  it('accepts only the whats-new folder', () => {
    expect(imageUrl('docs/images/whats-new/hero.webp')).toBe('/api/whats-new/image/hero.webp');
    expect(imageUrl('docs/images/whats-new/../x.png')).toBe('');
    expect(imageUrl('docs/images/whats-new/a/b.png')).toBe('');
    expect(imageUrl('https://evil.example/x.png')).toBe('');
    expect(imageUrl('docs/images/other.png')).toBe('');
  });
});

const SECTION = `### Добавлено

![hero](docs/images/whats-new/hero.webp)

- **Рабочие процессы.** Новый раздел
  с холстом.
- Простой пункт

### Изменено

- **Агент.** Своя сессия

### Исправлено

- **Ключ.** Починено

### Безопасность

- Закрыто`;

describe('parseSection', () => {
  it('splits groups, leads and continuation lines, finds the image', () => {
    const s = parseSection(SECTION);
    expect(s.image).toBe('/api/whats-new/image/hero.webp');
    expect(s.groups.map((g) => g.key)).toEqual(['new', 'improved', 'fixed', 'other:Безопасность']);
    expect(s.groups[0].items[0]).toEqual({ lead: 'Рабочие процессы.', text: 'Новый раздел с холстом.' });
    expect(s.groups[0].items[1]).toEqual({ lead: '', text: 'Простой пункт' });
  });
  it('puts bullets without a heading into «new»', () => {
    expect(parseSection('- a').groups[0].key).toBe('new');
  });
  it('splitLead keeps text without bold intact', () => {
    expect(splitLead('просто')).toEqual({ lead: '', text: 'просто' });
  });
});

describe('buildPages', () => {
  it('merges categories across versions, newest first', () => {
    const { pages } = buildPages([
      { version: '0.3.7', markdown: '### Добавлено\n\n- a\n\n### Исправлено\n\n- f1' },
      { version: '0.3.6', markdown: '### Добавлено\n\n- b\n\n### Изменено\n\n- c' },
    ]);
    expect(pages.map((p) => p.key)).toEqual(['new', 'improved', 'fixed']);
    expect(pages[0].items.map((i) => i.text)).toEqual(['a', 'b']);
    expect(pages[0].items[1].version).toBe('0.3.6');
  });
  it('is empty without bullets', () => {
    expect(buildPages([{ version: '1', markdown: '' }]).pages).toEqual([]);
  });
});

describe('parseNotes', () => {
  it('turns an old one-line note into one bullet', () => {
    const [s] = parseNotes('Починил вход', '0.4.1');
    expect(buildPages([s]).pages[0].items[0].text).toBe('Починил вход');
  });
  it('reads a changelog section with its heading', () => {
    const list = parseNotes('## [0.4.1] — 2026-11-20\n\n### Добавлено\n\n- **A.** b', '0.4.1');
    expect(list).toHaveLength(1);
    expect(list[0].version).toBe('0.4.1');
    expect(list[0].date).toBe('2026-11-20');
  });
  it('reads a section without a heading', () => {
    expect(parseNotes('### Исправлено\n\n- x', '0.4.2')[0].version).toBe('0.4.2');
  });
  it('returns nothing for empty notes', () => {
    expect(parseNotes('  ', '1')).toEqual([]);
  });
  it('picks the block of the interface language from bilingual notes', () => {
    const notes = '<!-- lang:en -->\n### Added\n\n- **A.** english\n\n<!-- lang:ru -->\n### Добавлено\n\n- **А.** русский';
    expect(buildPages(parseNotes(notes, '0.4.5', 'en')).pages[0].items[0].text).toBe('english');
    expect(buildPages(parseNotes(notes, '0.4.5', 'ru')).pages[0].items[0].text).toBe('русский');
  });
});

describe('mergeSections / buildFullList', () => {
  it('drops repeated versions', () => {
    expect(mergeSections([{ version: '2' }], [{ version: '2' }, { version: '1' }]).map((s) => s.version)).toEqual(['2', '1']);
  });
  it('lists every version with its groups', () => {
    const list = buildFullList([{ version: '2', date: 'd', markdown: SECTION }, { version: '1', date: '', markdown: '' }]);
    expect(list[0].groups).toHaveLength(4);
    expect(list[1].groups).toEqual([]);
  });
});
