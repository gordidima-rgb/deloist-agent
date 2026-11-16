import { describe, it, expect } from 'vitest';
import { markdownToText, escapeHtml } from './whatsNew.js';

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
});
