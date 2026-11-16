// Проверка, что собранный интерфейс web/ui соответствует исходникам ui/.
// Запуск из корня репозитория:  node tools/check_ui_built.mjs
// С флагом --write записывает хэш исходников в web/ui/.source-hash (его зовёт `npm run build` в ui/).
// Без зависимостей. Выходит с кодом 1, если вывод устарел или потерян.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const ui = join(root, 'ui');
const out = join(root, 'web', 'ui');
const hashFile = join(out, '.source-hash');

// Что влияет на результат: исходники, конфиги и точные версии пакетов.
function sources() {
  const files = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const full = join(dir, name);
      if (statSync(full).isDirectory()) walk(full);
      else files.push(full);
    }
  };
  walk(join(ui, 'src'));
  for (const name of ['package.json', 'package-lock.json', 'vite.config.js', 'svelte.config.js']) files.push(join(ui, name));
  return files.map((f) => relative(root, f).replace(/\\/g, '/')).sort();
}

function sourceHash() {
  const h = createHash('sha256');
  for (const rel of sources()) {
    // Переводы строк нормализуем: в Windows git может отдать CRLF.
    h.update(rel + '\0' + readFileSync(join(root, rel), 'utf8').replace(/\r\n/g, '\n') + '\0');
  }
  return h.digest('hex');
}

const current = sourceHash();

if (process.argv.includes('--write')) {
  writeFileSync(hashFile, current + '\n');
  console.log(`ui hash записан: ${current.slice(0, 12)}`);
  process.exit(0);
}

const problems = [];
for (const name of ['kit.js', 'kit.css']) {
  if (!existsSync(join(out, name))) problems.push(`web/ui/${name} нет: выполни npm run build в ui/`);
}
if (!existsSync(hashFile)) problems.push('web/ui/.source-hash нет: выполни npm run build в ui/');
else {
  const stored = readFileSync(hashFile, 'utf8').trim();
  if (stored !== current) problems.push(`исходники ui/ изменились после последней сборки (${stored.slice(0, 12)} → ${current.slice(0, 12)}): выполни npm run build в ui/ и закоммить web/ui`);
}

if (problems.length) {
  console.log(problems.join('\n'));
  process.exit(1);
}
console.log(`ui OK: web/ui собран из текущих исходников (${current.slice(0, 12)}, файлов ${sources().length})`);
