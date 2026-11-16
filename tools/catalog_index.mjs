// Пересобрать индексы каталога по его файлам. Запуск из корня репозитория:
//   node tools/catalog_index.mjs          записать индексы
//   node tools/catalog_index.mjs --check  только сверить (код 1, если устарел)
// Два корня: catalog/ (публичный) и private/catalog/ (только в официальной сборке, если папка есть).
// У каждого свой index.json; программа читает оба и сверяет сумму.
// Хэш — sha256 текста файла с переводами строк LF: так же считает src/catalog.rs.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const roots = [join(repo, 'catalog'), join(repo, 'private', 'catalog')].filter((dir) => existsSync(dir));
const check = process.argv.includes('--check');
let stale = false;

for (const root of roots) {
  const walk = (dir, out = []) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path, out);
      else if (name.endsWith('.json') && relative(root, path) !== 'index.json') out.push(path);
    }
    return out;
  };
  const items = walk(root)
    .map((path) => {
      const text = readFileSync(path, 'utf8').replace(/\r\n/g, '\n');
      const m = JSON.parse(text);
      return {
        id: m.id,
        kind: m.kind,
        category: m.category,
        title: m.title,
        tier: m.tier,
        version: m.version,
        path: relative(root, path).split(sep).join('/'),
        hash: createHash('sha256').update(text, 'utf8').digest('hex'),
      };
    })
    .sort((a, b) => a.id.localeCompare(b.id));
  const index = JSON.stringify({ schema: 1, items }, null, 2) + '\n';
  const target = join(root, 'index.json');
  const label = relative(repo, target).split(sep).join('/');
  if (check) {
    let current = '';
    try {
      current = readFileSync(target, 'utf8').replace(/\r\n/g, '\n');
    } catch {}
    if (current !== index) {
      console.log(`${label} устарел: запусти node tools/catalog_index.mjs`);
      stale = true;
    } else console.log(`${label} в порядке: ${items.length} элементов`);
  } else {
    writeFileSync(target, index);
    console.log(`${label}: ${items.length} элементов`);
  }
}
if (stale) process.exit(1);
