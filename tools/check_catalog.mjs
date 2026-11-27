#!/usr/bin/env node
// Validates the bot catalog (catalog/) against manifest schema 1.
//
//   node tools/check_catalog.mjs            check ./catalog and ./THIRD_PARTY next to this repo
//   node tools/check_catalog.mjs <root>     check <root>/catalog and <root>/THIRD_PARTY
//
// Exit code 0 = clean, 1 = errors. Warnings do not fail the run.
// No dependencies: Node 18+.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(process.argv[2] || join(here, '..'));
// Public catalog, plus private/catalog when that folder exists (official build only).
const catalogRoots = [{ dir: join(root, 'catalog'), prefix: 'catalog' }];
if (existsSync(join(root, 'private', 'catalog'))) catalogRoots.push({ dir: join(root, 'private', 'catalog'), prefix: 'private/catalog' });
const catalogDir = catalogRoots[0].dir;
const thirdPartyDir = join(root, 'THIRD_PARTY');

const KINDS = ['skill', 'agent', 'app'];
const KIND_DIR = { skill: 'skills', agent: 'agents', app: 'apps' };
const CATEGORIES = ['productivity', 'docs', 'research', 'business', 'marketing', 'dev', 'personal', 'design'];
const TIERS = ['verified', 'public'];
const MODEL_HINTS = ['strong', 'standard', 'fast', 'smart'];
const EFFORTS = ['low', 'medium', 'high'];
const RANKS = ['lead', 'specialist'];
// A catalog agent must never ship with the "decide on your own" mode.
const APPROVALS = ['ask', 'strict'];
const OWN_SOURCE = 'Deloist';
const ID_RE = /^(skill|agent|app)\.[a-z0-9]+(-[a-z0-9]+)*$/;
const SEMVER_RE = /^\d+\.\d+\.\d+$/;
const HOST_RE = /^([a-z0-9-]+\.)+[a-z]{2,}$/;
const COLOR_RE = /^#[0-9A-Fa-f]{6}$/;
const PERSONA_MAX = 12000;
const MAX_SHAPE = 7; // src/store.rs MAX_SHAPE
const MAX_EYES = 2; // src/store.rs MAX_EYES

// Tool names a skill body may talk about (src/tools.rs, browser_tools.rs, workflow_tools.rs, goal_tools.rs, skills.rs).
const KNOWN_TOOLS = new Set([
  'list_files', 'read_file', 'write_file', 'run_command', 'web_fetch', 'remember', 'recall', 'ask_user',
  'list_bots', 'create_bot', 'update_bot', 'delegate', 'save_skill', 'read_skill',
  'browser_nav', 'browser_page', 'browser_mouse', 'browser_keyboard', 'browser_devtools', 'browser_human', 'browser_capture_key',
  'list_workflows', 'create_workflow', 'edit_workflow', 'delete_workflow', 'run_workflow',
  'list_goals', 'create_goal', 'update_goal', 'run_goal', 'pause_goal', 'resume_goal', 'complete_goal', 'report_goal_progress',
]);

// Licenses that are allowed for third-party content.
const ALLOWED_LICENSES = /^(MIT|Apache-2\.0|BSD-[23]-Clause|CC-BY-(3\.0|4\.0)|CC0-1\.0)$/;

const errors = [];
const warnings = [];
const err = (where, msg) => errors.push(`${where}: ${msg}`);
const warn = (where, msg) => warnings.push(`${where}: ${msg}`);

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const isStr = (v) => typeof v === 'string';
const nonEmpty = (v) => isStr(v) && v.trim().length > 0;
const isStrArray = (v) => Array.isArray(v) && v.every(isStr);
const hasCyrillic = (s) => /[Ѐ-ӿ]/.test(s);

function headingsInOrder(text, headings) {
  let pos = 0;
  for (const h of headings) {
    const idx = String(text).slice(pos).indexOf(h);
    if (idx === -1) return false;
    pos += idx + h.length;
  }
  return true;
}

function readText(path, where) {
  const text = readFileSync(path, 'utf8');
  if (text.includes('\r')) err(where, 'contains CR characters; files must use LF');
  if (text.charCodeAt(0) === 0xfeff) err(where, 'starts with a BOM');
  return text;
}

function localized(obj, field, where, { russianRequired = true } = {}) {
  const v = obj?.[field];
  if (!isObj(v)) {
    err(where, `"${field}" must be an object {ru, en}`);
    return null;
  }
  if (russianRequired) {
    if (!nonEmpty(v.ru)) err(where, `"${field}.ru" is missing or empty`);
    else if (!hasCyrillic(v.ru)) err(where, `"${field}.ru" has no Cyrillic text`);
  }
  if (!nonEmpty(v.en)) warn(where, `"${field}.en" is missing or empty`);
  return v;
}

// ---------------------------------------------------------------------------
// Load manifests
// ---------------------------------------------------------------------------

if (!existsSync(catalogDir)) {
  console.error(`catalog folder not found: ${catalogDir}`);
  process.exit(1);
}

const manifests = []; // { rel, path, text, data }
for (const { dir: catalogRoot, prefix } of catalogRoots) for (const kind of KINDS) {
  const dir = join(catalogRoot, KIND_DIR[kind]);
  if (!existsSync(dir)) continue;
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (!statSync(path).isFile() || !name.endsWith('.json')) continue;
    const rel = `${prefix}/${KIND_DIR[kind]}/${name}`;
    const text = readText(path, rel);
    let data;
    try {
      data = JSON.parse(text);
    } catch (e) {
      err(rel, `invalid JSON: ${e.message}`);
      continue;
    }
    manifests.push({ rel, path, text, data, dirKind: kind, prefix });
  }
}

const ids = new Map(); // id -> rel
for (const m of manifests) {
  const id = m.data?.id;
  if (!nonEmpty(id)) continue;
  if (ids.has(id)) err(m.rel, `duplicate id "${id}" (also in ${ids.get(id)})`);
  else ids.set(id, m.rel);
}
const skillIds = new Set(manifests.filter((m) => m.data?.kind === 'skill').map((m) => m.data.id));

// ---------------------------------------------------------------------------
// Validate each manifest
// ---------------------------------------------------------------------------

const thirdPartyNeeded = new Map(); // file name -> Set of ids

function checkSource(m) {
  const { rel, data } = m;
  const s = data.source;
  if (!isObj(s)) return err(rel, '"source" must be an object');
  if (!nonEmpty(s.name)) err(rel, '"source.name" is missing');
  if (!nonEmpty(s.url) || !/^https:\/\//.test(s.url)) err(rel, '"source.url" must be an https URL');
  if (!nonEmpty(s.license)) err(rel, '"source.license" is missing');
  if (!isStr(s.notice)) err(rel, '"source.notice" must be a string');
  if (typeof s.modified !== 'boolean') err(rel, '"source.modified" must be true or false');
  if (!nonEmpty(s.name) || !nonEmpty(s.license)) return;

  if (s.name === OWN_SOURCE) {
    if (!nonEmpty(s.notice)) err(rel, 'own items need a "source.notice"');
    return;
  }
  // Third-party content.
  if (!nonEmpty(s.file)) err(rel, '"source.file" (the exact file taken) is missing');
  if (!nonEmpty(s.commit) || !/^[0-9a-f]{40}$/.test(s.commit)) err(rel, '"source.commit" must be a full 40-character SHA');
  if (!ALLOWED_LICENSES.test(s.license)) err(rel, `license "${s.license}" is not allowed (MIT, Apache-2.0, BSD, CC-BY, CC0)`);
  if (!nonEmpty(s.notice)) err(rel, 'third-party items need a copyright "source.notice"');
  if (s.modified !== true) warn(rel, '"source.modified" is not true; every item here is rewritten');
  const file = s.name.replace('/', '-') + '.txt';
  if (!thirdPartyNeeded.has(file)) thirdPartyNeeded.set(file, { license: s.license, ids: new Set() });
  const entry = thirdPartyNeeded.get(file);
  entry.ids.add(data.id);
  if (entry.license !== s.license) err(rel, `license "${s.license}" differs from other items of ${s.name} ("${entry.license}")`);
}

function checkRequires(m) {
  const { rel, data } = m;
  const r = data.requires;
  if (!isObj(r)) return err(rel, '"requires" must be an object');
  for (const key of ['apps', 'builtin', 'sites']) {
    if (!isStrArray(r[key])) err(rel, `"requires.${key}" must be an array of strings`);
  }
  for (const key of ['python', 'binaries']) {
    if (r[key] !== undefined && !isStrArray(r[key])) err(rel, `"requires.${key}" must be an array of strings`);
  }
  for (const site of r.sites || []) if (!HOST_RE.test(site)) err(rel, `"requires.sites" has a bad host "${site}"`);
  for (const b of r.builtin || []) if (!['browser', 'python'].includes(b)) warn(rel, `unknown "requires.builtin" value "${b}"`);
  if (data.kind === 'skill' && (r.python || []).length > 0 && !(r.builtin || []).includes('python')) {
    err(rel, '"requires.python" is set but "requires.builtin" lacks "python"');
  }
}

function checkSkill(m) {
  const { rel, data, path } = m;
  const s = data.skill;
  if (!isObj(s)) return err(rel, '"skill" must be an object');
  const body = localized(s, 'body', rel);
  if (body) {
    for (const lang of ['ru', 'en']) {
      const t = body[lang];
      if (!isStr(t)) continue;
      if (t.length > 20000) err(rel, `"skill.body.${lang}" is too long (${t.length} > 20000)`);
      if (t.length < 200) warn(rel, `"skill.body.${lang}" is very short (${t.length})`);
    }
  }
  if (!isStrArray(s.tools)) err(rel, '"skill.tools" must be an array of strings');
  else {
    for (const t of s.tools) {
      if (!KNOWN_TOOLS.has(t) && !t.startsWith('mcp:')) err(rel, `unknown tool "${t}" in skill.tools`);
    }
  }
  // Bundled files must exist under catalog/skills/<id>/.
  if (s.files !== undefined) {
    if (!isStrArray(s.files)) err(rel, '"skill.files" must be an array of strings');
    else {
      const dir = join(dirname(path), data.id);
      for (const f of s.files) {
        if (f.startsWith('/') || f.includes('..')) err(rel, `bad path in skill.files: "${f}"`);
        else if (!existsSync(join(dir, f))) err(rel, `skill.files lists "${f}" but catalog/skills/${data.id}/${f} does not exist`);
      }
      if (s.files.length && !s.files.includes('LICENSE.txt') && data.source?.name !== OWN_SOURCE) {
        warn(rel, 'bundled scripts without LICENSE.txt');
      }
    }
  }
  // A skill must never grant rights or tell the bot to skip confirmations.
  const forbidden = [
    /без\s+(?:подтверждени\w+|спроса|вопросов)\s+(?:отправл|удал|покуп|плат)/i,
    /не\s+(?:спрашивай|проси)\s+(?:подтверждени|разрешени|согласи)/i,
    /skip\s+(?:the\s+)?(?:confirmation|approval)/i,
    /do\s+not\s+ask\s+for\s+(?:confirmation|approval|permission)/i,
  ];
  for (const lang of ['ru', 'en']) {
    for (const re of forbidden) if (re.test(body?.[lang] || '')) err(rel, `skill.body.${lang} may tell the bot to skip confirmations (${re})`);
  }
}

function checkAgent(m) {
  const { rel, data } = m;
  const a = data.agent;
  if (!isObj(a)) return err(rel, '"agent" must be an object');
  localized(a, 'role', rel);
  const persona = localized(a, 'persona', rel);
  if (persona) {
    for (const lang of ['ru', 'en']) {
      const t = persona[lang];
      if (isStr(t) && [...t].length > PERSONA_MAX) err(rel, `"agent.persona.${lang}" is ${[...t].length} characters (max ${PERSONA_MAX})`);
    }
  }
  if (!isStrArray(a.skills) || a.skills.length === 0) err(rel, '"agent.skills" must be a non-empty array of skill ids');
  else {
    const seen = new Set();
    for (const id of a.skills) {
      if (seen.has(id)) err(rel, `agent.skills lists "${id}" twice`);
      seen.add(id);
      if (!skillIds.has(id)) err(rel, `agent skill "${id}" does not exist in the catalog`);
    }
  }
  if (!MODEL_HINTS.includes(a.model_hint)) err(rel, `"agent.model_hint" must be one of ${MODEL_HINTS.join(', ')} (never a model name)`);
  if (a.effort !== undefined && !EFFORTS.includes(a.effort)) err(rel, `"agent.effort" must be one of ${EFFORTS.join(', ')}`);
  if (a.rank !== undefined && !RANKS.includes(a.rank)) err(rel, `"agent.rank" must be one of ${RANKS.join(', ')}`);
  if (!APPROVALS.includes(a.approval)) err(rel, `"agent.approval" must be one of ${APPROVALS.join(', ')}`);
  const av = a.avatar;
  if (!isObj(av)) err(rel, '"agent.avatar" must be an object');
  else {
    if (!COLOR_RE.test(av.color || '')) err(rel, '"agent.avatar.color" must be #RRGGBB');
    if (!nonEmpty(av.emoji)) err(rel, '"agent.avatar.emoji" is missing');
    if (!Number.isInteger(av.shape) || av.shape < 0 || av.shape > MAX_SHAPE) err(rel, `"agent.avatar.shape" must be an integer 0..${MAX_SHAPE}`);
    if (!Number.isInteger(av.eyes) || av.eyes < 0 || av.eyes > MAX_EYES) err(rel, `"agent.avatar.eyes" must be an integer 0..${MAX_EYES}`);
  }
  if (!isStrArray(a.sites)) err(rel, '"agent.sites" must be an array of hostnames');
  else for (const site of a.sites) if (!HOST_RE.test(site)) err(rel, `"agent.sites" has a bad host "${site}"`);
  // The agent's sites must be declared in requires.sites so the install dialog can show them.
  for (const site of a.sites || []) {
    if (!(data.requires?.sites || []).includes(site)) err(rel, `site "${site}" is in agent.sites but not in requires.sites`);
  }

  const rank = a.rank || 'lead';
  if (rank === 'specialist') {
    // persona headings in order
    const ruHeadings = ['## Роль', '## Когда звать', '## Как работаю', '## Стандарты качества', '## Проверка', '## Чего не делаю', '## Грабли', '## Формат ответа'];
    const enHeadings = ['## Role', '## When to call me', '## How I work', '## Quality bar', '## Self-check', '## What I don\'t do', '## Pitfalls', '## Answer format'];
    const ruText = (isObj(a.persona) ? (a.persona.ru || '') : (a.persona || ''));
    const enText = (isObj(a.persona) ? (a.persona.en || '') : (a.persona || ''));
    if (!headingsInOrder(ruText, ruHeadings)) err(rel, 'agent.persona.ru is missing specialist headings in order');
    if (!headingsInOrder(enText, enHeadings)) err(rel, 'agent.persona.en is missing specialist headings in order');

    // triggers required
    const t = a.triggers;
    if (!isObj(t)) err(rel, '"agent.triggers" must be an object');
    else {
      if (!isStrArray(t.files) || t.files.length === 0 || t.files.length > 40) err(rel, '"agent.triggers.files" must be a non-empty array of strings (<=40)');
      const kw = t.keywords;
      if (!isObj(kw)) err(rel, '"agent.triggers.keywords" must be an object');
      else {
        for (const lang of ['ru', 'en']) {
          if (!isStrArray(kw[lang]) || kw[lang].length === 0 || kw[lang].length > 40) err(rel, `"agent.triggers.keywords.${lang}" must be a non-empty array of strings (<=40)`);
        }
      }
    }

    // checks required
    const checks = a.checks;
    if (!Array.isArray(checks) || checks.length < 2 || checks.length > 3) err(rel, '"agent.checks" must have 2-3 items');
    else {
      for (let i = 0; i < checks.length; i++) {
        const c = checks[i];
        if (!isObj(c)) { err(rel, `"agent.checks[${i}]" must be an object`); continue; }
        const task = localized(c, 'task', rel);
        if (task) {
          for (const lang of ['ru', 'en']) {
            if (!nonEmpty(task[lang])) err(rel, `"agent.checks[${i}].task.${lang}" is missing`);
          }
        }
        const crit = c.criteria;
        if (!isObj(crit)) err(rel, `"agent.checks[${i}].criteria" must be an object`);
        else {
          for (const lang of ['ru', 'en']) {
            if (!isStrArray(crit[lang]) || crit[lang].length === 0) err(rel, `"agent.checks[${i}].criteria.${lang}" must be a non-empty array of strings`);
          }
        }
      }
    }
  }
}

for (const m of manifests) {
  const { rel, data, dirKind } = m;
  if (!isObj(data)) {
    err(rel, 'manifest must be a JSON object');
    continue;
  }
  if (data.schema !== 1) err(rel, `"schema" must be 1 (got ${JSON.stringify(data.schema)})`);
  if (!KINDS.includes(data.kind)) err(rel, `"kind" must be one of ${KINDS.join(', ')}`);
  else if (data.kind !== dirKind) err(rel, `kind "${data.kind}" does not match the folder catalog/${KIND_DIR[dirKind]}`);
  if (!nonEmpty(data.id) || !ID_RE.test(data.id)) err(rel, `"id" must look like "${data.kind || 'kind'}.some-name"`);
  else {
    if (data.kind && !data.id.startsWith(data.kind + '.')) err(rel, `id "${data.id}" must start with "${data.kind}."`);
    if (!rel.endsWith(`/${data.id}.json`)) err(rel, `file name must be "${data.id}.json"`);
  }
  if (!nonEmpty(data.version) || !SEMVER_RE.test(data.version)) err(rel, '"version" must be x.y.z');
  if (!CATEGORIES.includes(data.category)) err(rel, `"category" must be one of ${CATEGORIES.join(', ')}`);
  if (!TIERS.includes(data.tier)) err(rel, `"tier" must be one of ${TIERS.join(', ')}`);
  localized(data, 'title', rel);
  localized(data, 'summary', rel);
  if (isStr(data.title?.ru) && data.title.ru.length > 80) warn(rel, 'title.ru is longer than 80 characters');
  if (isStr(data.summary?.ru) && data.summary.ru.length > 300) warn(rel, 'summary.ru is longer than 300 characters');
  if (!nonEmpty(data.icon) || !/^(emoji:|file:|https:)/.test(data.icon)) err(rel, '"icon" must start with emoji:, file: or https:');
  checkSource(m);
  checkRequires(m);
  if (data.kind === 'skill') checkSkill(m);
  else if (data.kind === 'agent') checkAgent(m);
  else if (data.kind === 'app') warn(rel, 'app manifests are not covered by this checker yet');
  // The other block must not be present.
  for (const k of ['skill', 'agent', 'app']) {
    if (k !== data.kind && data[k] !== undefined) err(rel, `unexpected "${k}" block in a ${data.kind}`);
  }
}

// ---------------------------------------------------------------------------
// THIRD_PARTY files
// ---------------------------------------------------------------------------

for (const [file, { license, ids: users }] of thirdPartyNeeded) {
  const path = join(thirdPartyDir, file);
  const where = `THIRD_PARTY/${file}`;
  if (!existsSync(path)) {
    err(where, `missing (needed by ${[...users].join(', ')})`);
    continue;
  }
  const text = readText(path, where);
  if (text.length < 500) err(where, 'is too short to contain a license text');
  if (!/copyright/i.test(text)) err(where, 'has no copyright line');
  if (license === 'MIT' && !/Permission is hereby granted, free of charge/.test(text)) err(where, 'does not contain the full MIT license text');
  if (license === 'Apache-2.0') {
    if (!/Apache License\s+Version 2\.0/.test(text)) err(where, 'does not contain the Apache-2.0 license text');
    if (!/NOTICE/.test(text)) err(where, 'must state whether the source has a NOTICE file');
    if (!/modif/i.test(text)) err(where, 'must carry a note that the files were modified');
  }
}

// ---------------------------------------------------------------------------
// index.json
// ---------------------------------------------------------------------------

for (const { dir: catalogRoot, prefix } of catalogRoots) {
  const indexRel = `${prefix}/index.json`;
  const indexPath = join(catalogRoot, 'index.json');
  const own = manifests.filter((m) => m.prefix === prefix);
  if (!existsSync(indexPath)) err(indexRel, 'missing');
  else {
    let index = null;
    try {
      index = JSON.parse(readText(indexPath, indexRel));
    } catch (e) {
      err(indexRel, `invalid JSON: ${e.message}`);
    }
    if (index) {
      if (index.schema !== 1) err(indexRel, '"schema" must be 1');
      const items = Array.isArray(index.items) ? index.items : (err(indexRel, '"items" must be an array'), []);
      const seen = new Set();
      for (const it of items) {
        if (seen.has(it.id)) err(indexRel, `duplicate id "${it.id}"`);
        seen.add(it.id);
        const m = own.find((x) => x.data?.id === it.id);
        if (!m) {
          err(indexRel, `lists "${it.id}" but no manifest has this id`);
          continue;
        }
        const hash = createHash('sha256').update(readFileSync(m.path)).digest('hex');
        if (it.hash !== hash) err(indexRel, `hash of "${it.id}" is stale (run the catalog build again)`);
        for (const key of ['kind', 'category', 'version', 'tier']) {
          if (it[key] !== m.data[key]) err(indexRel, `"${it.id}".${key} differs from its manifest`);
        }
        if (!isObj(it.title) || it.title.ru !== m.data.title?.ru) err(indexRel, `"${it.id}".title differs from its manifest`);
        const expected = m.rel.slice(prefix.length + 1);
        if (it.path !== expected) err(indexRel, `"${it.id}".path is "${it.path}", expected "${expected}"`);
      }
      for (const m of own) {
        if (nonEmpty(m.data?.id) && !seen.has(m.data.id)) err(indexRel, `does not list "${m.data.id}"`);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Bundled files that are not listed in a manifest, and line endings
// ---------------------------------------------------------------------------

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}
const TEXT_EXT = /\.(py|xml|txt|md|json|mjs)$/;
for (const f of catalogRoots.flatMap((r) => walk(r.dir))) {
  const rel = f.slice(root.length + 1).replaceAll('\\', '/');
  if (rel.endsWith('/index.json') || /^(private\/)?catalog\/(skills|agents|apps)\/[^/]+\.json$/.test(rel)) continue;
  if (TEXT_EXT.test(rel) && readFileSync(f, 'utf8').includes('\r')) err(rel, 'contains CR characters; files must use LF');
  const m = /^(?:private\/)?catalog\/skills\/([^/]+)\//.exec(rel);
  if (m) {
    const owner = manifests.find((x) => x.data?.id === m[1]);
    if (!owner) err(rel, `folder has no manifest "${m[1]}.json"`);
    else if (!(owner.data.skill?.files || []).includes(rel.slice(rel.indexOf(`skills/${m[1]}/`) + `skills/${m[1]}/`.length))) {
      err(rel, `is not listed in skill.files of ${m[1]}`);
    }
  }
}

// ---------------------------------------------------------------------------

const count = (kind) => manifests.filter((m) => m.data?.kind === kind).length;
for (const w of warnings) console.warn(`warning: ${w}`);
for (const e of errors) console.error(`error: ${e}`);
console.log(`catalog: ${manifests.length} manifests (${count('skill')} skills, ${count('agent')} agents, ${count('app')} apps), ${errors.length} errors, ${warnings.length} warnings`);
process.exit(errors.length ? 1 : 0);
