#!/usr/bin/env node
// Checks specialist bots in catalog/agents/*.json.
//
//   node tools/check_agents.mjs
//
// Exit code 0 = clean, 1 = problems. Plain Node, no dependencies.

import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
// Public catalog plus private/catalog (official build only, when that folder exists).
const catalogRoots = [{ dir: join(root, 'catalog'), prefix: 'catalog' }];
if (existsSync(join(root, 'private', 'catalog'))) catalogRoots.push({ dir: join(root, 'private', 'catalog'), prefix: 'private/catalog' });

// Exact headings of the reference specialist (agent.rust-developer).
const RU_HEADINGS = [
  '## Роль',
  '## Когда звать',
  '## Как работаю',
  '## Стандарты качества',
  '## Проверка',
  '## Чего не делаю',
  '## Грабли',
  '## Формат ответа',
];
const EN_HEADINGS = [
  '## Role',
  '## When to call me',
  '## How I work',
  '## Quality bar',
  '## Self-check',
  '## What I don\'t do',
  '## Pitfalls',
  '## Answer format',
];
const MODEL_HINTS = ['strong', 'standard', 'fast'];
const PERSONA_MIN = 3 * 1024;
const PERSONA_MAX = 9 * 1024;

const SECRETS = [
  /\bsk-/,
  /\bghp_/,
  /\bAKIA/,
  /-----BEGIN/,
];
const ABSOLUTE_PATHS = [
  /(?:^|[^A-Za-z])[A-Za-z]:\\/,
  /\/(?:Users|home)\//,
];
const FORBIDDEN = [/internal[ -]only/i, /[^a-z]confidential[^a-z]/i];

const problems = [];
const report = (where, message) => problems.push(`${where}: ${message}`);
const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const isText = (value) => typeof value === 'string' && value.trim().length > 0;

function hasBothLanguages(value) {
  return isObject(value) && isText(value.ru) && isText(value.en);
}

function headingsInOrder(text, headings) {
  let pos = 0;
  for (const heading of headings) {
    const index = text.slice(pos).indexOf(heading);
    if (index === -1) return false;
    pos += index + heading.length;
  }
  return true;
}

function checkTriggers(where, triggers) {
  if (!isObject(triggers)) {
    report(where, 'agent.triggers must be an object');
    return;
  }
  if (!Array.isArray(triggers.files) || triggers.files.length === 0 || !triggers.files.every(isText)) {
    report(where, 'agent.triggers.files must be a non-empty array of strings');
  }
  if (!isObject(triggers.keywords)) {
    report(where, 'agent.triggers.keywords must be an object');
    return;
  }
  for (const lang of ['ru', 'en']) {
    const list = triggers.keywords[lang];
    if (!Array.isArray(list) || list.length === 0 || !list.every(isText)) {
      report(where, `agent.triggers.keywords.${lang} must be a non-empty array of strings`);
    }
  }
}

function checkPatterns(where, patterns, name, allowEmpty = false) {
  if (!Array.isArray(patterns) || patterns.length > 4 || (!allowEmpty && patterns.length === 0) || !patterns.every(isText)) {
    report(where, `${name} must be an array of ${allowEmpty ? '0-4' : '1-4'} non-empty strings`);
    return;
  }
  for (const pattern of patterns) {
    try {
      new RegExp(pattern, 'iu');
    } catch (error) {
      report(where, `${name} has an invalid regular expression ${JSON.stringify(pattern)}: ${error.message}`);
    }
  }
}

function checkChecks(where, checks) {
  if (!Array.isArray(checks) || checks.length < 2 || checks.length > 3) {
    report(where, 'agent.checks must have 2-3 items');
    return;
  }
  checks.forEach((check, i) => {
    if (!isObject(check)) {
      report(where, `agent.checks[${i}] must be an object`);
      return;
    }
    if (!hasBothLanguages(check.task)) {
      report(where, `agent.checks[${i}].task must have both ru and en`);
    }
    if (!isObject(check.criteria)) {
      report(where, `agent.checks[${i}].criteria must be an object`);
      return;
    }
    for (const lang of ['ru', 'en']) {
      const criteria = check.criteria[lang];
      if (!Array.isArray(criteria) || criteria.length === 0 || !criteria.every(isText)) {
        report(where, `agent.checks[${i}].criteria.${lang} must be a non-empty array of strings`);
      }
    }
    // Исполняемая проверка ответа: без неё релизный прогон не решит «прошёл/не прошёл».
    if (!isObject(check.behavior)) {
      report(where, `agent.checks[${i}].behavior must be an object`);
      return;
    }
    checkPatterns(where, check.behavior.require, `agent.checks[${i}].behavior.require`);
    checkPatterns(where, check.behavior.forbid, `agent.checks[${i}].behavior.forbid`, true);
  });
}

let indexIds = new Set();
for (const { dir, prefix } of catalogRoots) {
  try {
    const index = JSON.parse(readFileSync(join(dir, 'index.json'), 'utf8'));
    const items = Array.isArray(index.items) ? index.items : [];
    for (const item of items) if (isText(item?.id)) indexIds.add(item.id);
  } catch (error) {
    report(`${prefix}/index.json`, `cannot read the catalog index: ${error.message}`);
  }
}

const agents = [];
const specialists = [];
const entries = catalogRoots.flatMap(({ dir, prefix }) => {
  const agentsDir = join(dir, 'agents');
  if (!existsSync(agentsDir)) return [];
  return readdirSync(agentsDir).filter((name) => name.endsWith('.json')).sort().map((name) => ({ name, agentsDir, prefix }));
});

for (const { name, agentsDir, prefix } of entries) {
  const rel = `${prefix}/agents/${name}`;
  let data;
  try {
    data = JSON.parse(readFileSync(join(agentsDir, name), 'utf8'));
  } catch (error) {
    report(rel, `invalid JSON: ${error.message}`);
    continue;
  }
  const rank = data?.agent?.rank || 'lead'; // старые манифесты без rank — ведущие (как при загрузке каталога).
  if (!['lead', 'specialist'].includes(rank)) continue;
  agents.push({ rel, data });
  if (rank === 'specialist') specialists.push({ rel, data });
}

for (const { rel, data } of agents) {
  if (!hasBothLanguages(data.title)) report(rel, 'title must have both ru and en');
  if (!hasBothLanguages(data.summary)) report(rel, 'summary must have both ru and en');

  const agent = isObject(data.agent) ? data.agent : {};
  if (!hasBothLanguages(agent.role)) report(rel, 'agent.role must have both ru and en');

  const persona = isObject(agent.persona) ? agent.persona : null;
  if (!persona || !isText(persona.ru) || !isText(persona.en)) {
    report(rel, 'agent.persona must have both ru and en');
  } else if (data.agent.rank === 'specialist') {
    if (!headingsInOrder(persona.ru, RU_HEADINGS)) {
      report(rel, 'agent.persona.ru is missing the 8 specialist headings in order');
    }
    if (!headingsInOrder(persona.en, EN_HEADINGS)) {
      report(rel, 'agent.persona.en is missing the 8 specialist headings in order');
    }
    for (const lang of ['ru', 'en']) {
      const bytes = Buffer.byteLength(persona[lang], 'utf8');
      if (bytes < PERSONA_MIN || bytes > PERSONA_MAX) {
        report(rel, `agent.persona.${lang} is ${bytes} bytes, expected 3-9 KB`);
      }
    }
  }

  if (data.agent.rank === 'specialist') checkTriggers(rel, agent.triggers);
  if (!MODEL_HINTS.includes(agent.model_hint)) {
    report(rel, `agent.model_hint must be one of ${MODEL_HINTS.join(', ')}`);
  }
  if (!['low', 'medium', 'high'].includes(agent.effort)) {
    report(rel, 'agent.effort must be one of low, medium, high');
  }
  checkChecks(rel, agent.checks);

  const text = readFileSync(join(root, rel), 'utf8');
  // Work on a decoded copy so \b still sees real line breaks inside JSON strings.
  const scanText = text.replace(/\\n/g, '\n').replace(/\\r/g, '\r').replace(/\\t/g, '\t');
  for (const pattern of SECRETS) {
    if (pattern.test(scanText)) report(rel, `contains a secret-like value (${pattern})`);
  }
  for (const pattern of ABSOLUTE_PATHS) {
    if (pattern.test(text)) report(rel, `contains an absolute path (${pattern})`);
  }
  for (const pattern of FORBIDDEN) {
    if (pattern.test(text)) report(rel, `mentions forbidden text (${pattern})`);
  }

  if (rel.startsWith('catalog/') && !indexIds.has(data.id)) {
    report('catalog/index.json', `agent "${data.id}" is missing from the index`);
  }
}

if (problems.length > 0) {
  for (const problem of problems) console.log(problem);
  process.exit(1);
}
console.log(`ok ${agents.length} agents (${specialists.length} specialists)`);
