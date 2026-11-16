import { describe, expect, it } from 'vitest';
import { formatQuote } from './fileQuote.js';

describe('formatQuote', () => {
  it('names the file, the lines and the comment', () => {
    expect(formatQuote({ path: 'src/a.rs', from: 3, to: 5, quote: 'let x = 1;', comment: ' сделай u64 ', language: 'rs' })).toBe(
      'Файл `src/a.rs`, строки 3–5:\n```rs\nlet x = 1;\n```\nКомментарий: сделай u64',
    );
  });

  it('says one line for a single-line selection and nothing for a rendered one', () => {
    expect(formatQuote({ path: 'a.md', from: 7, to: 7, quote: 'x' })).toContain('строка 7:');
    expect(formatQuote({ path: 'a.md', quote: 'x' }).split('\n')[0]).toBe('Файл `a.md`:');
  });

  it('omits the comment line when empty', () => {
    expect(formatQuote({ path: 'a.txt', quote: 'x', comment: '  ' })).not.toContain('Комментарий');
  });

  it('uses a longer fence when the quote has backticks', () => {
    const out = formatQuote({ path: 'a.md', quote: 'a ```js\nb\n``` c' });
    expect(out).toContain('````\na ```js');
  });

  it('cuts a very long quote', () => {
    const out = formatQuote({ path: 'a.txt', quote: 'x'.repeat(5000) });
    expect(out.length).toBeLessThan(4200);
    expect(out).toContain('…');
  });
});
