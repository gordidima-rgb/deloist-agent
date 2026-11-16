import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as deny from '../extension/deny.js';

const root = dirname(fileURLToPath(import.meta.url));
const denyPath = join(root, '..', 'extension', 'deny.json');
const data = JSON.parse(readFileSync(denyPath, 'utf8'));

const fetchOk = async () => ({ json: async () => data });
await deny.loadDeny(fetchOk, 'deny.json');

let failures = 0;
const check = (cond, msg) => { if (!cond) { console.log('FAIL:', msg); failures++; } };

// each entry: apex and a subdomain denied
for (const pat of data.deny) {
  const hostpart = pat.split('/')[0];
  const apex = hostpart.replace(/^\*\./, '');
  const url = pat.includes('/')
    ? `https://${apex}/${pat.split('/').slice(1).join('/')}`
    : `https://${apex}/`;
  check(deny.isDeniedUrl(url), `${pat} -> ${url}`);
  if (hostpart.startsWith('*.')) {
    const suburl = pat.includes('/')
      ? `https://sub.${apex}/${pat.split('/').slice(1).join('/')}`
      : `https://sub.${apex}/`;
    check(deny.isDeniedUrl(suburl), `${pat} -> ${suburl}`);
  }
}

check(!deny.isDeniedUrl('https://evilsberbank.ru/'), 'evilsberbank not denied');
check(!deny.isDeniedUrl('https://sberbank.ru.evil.com/'), 'sberbank.ru.evil.com not denied');
check(deny.isDeniedUrl('https://sberbank.ru./'), 'trailing dot denied');
check(deny.isDeniedUrl('not a url'), 'unparseable denied');
check(deny.isDeniedUrl('https://chrome.google.com/WebStore/detail/x'), 'path case is ignored');
check(deny.isDeniedUrl('https://Chrome.Google.com/%57EBSTORE'), 'encoded upper-case path denied');
check(!deny.isDeniedUrl('https://chrome.google.com/other'), 'other path not denied');

// fail closed on load failure
const fetchBad = async () => { throw new Error('boom'); };
await deny.loadDeny(fetchBad, 'deny.json');
check(deny.isDeniedUrl('https://example.com/'), 'fail closed');

if (failures) { console.log(`\n${failures} failures`); process.exit(1); }
console.log('check_deny OK');