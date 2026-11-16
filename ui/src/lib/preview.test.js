import { describe, expect, it } from 'vitest';
import { plainPreview, teamSubtitle } from './preview.js';

describe('plainPreview', () => {
  it('drops bold markers, including an unclosed one from a cut preview', () => {
    expect(plainPreview('Передал данные **@Редакт')).toBe('Передал данные @Редакт');
    expect(plainPreview('Группа **Лента X** собрана')).toBe('Группа Лента X собрана');
  });
  it('keeps link text, code and snake_case identifiers', () => {
    expect(plainPreview('см. [доки](https://x.y) и `run_task`')).toBe('см. доки и run_task');
    expect(plainPreview('файл my_file_name.md')).toBe('файл my_file_name.md');
  });
  it('flattens headings, bullets and line breaks', () => {
    expect(plainPreview('## Итог\n- один\n- два')).toBe('Итог один два');
    expect(plainPreview('ты: *важно* и _так_')).toBe('ты: важно и так');
  });
  it('handles empty input', () => {
    expect(plainPreview(undefined)).toBe('');
  });
});

describe('teamSubtitle', () => {
  it('prefers the last message', () => {
    expect(teamSubtitle('Редактор: **готово**', ['Редактор'], 'пусто')).toBe('Редактор: готово');
  });
  it('lists members while nobody wrote in the team', () => {
    expect(teamSubtitle('', ['Редактор', 'Картинка', 'Пост'], 'пусто')).toBe('Редактор, Картинка, Пост');
  });
  it('is empty only without bots', () => {
    expect(teamSubtitle('', [], 'пусто')).toBe('пусто');
  });
});
