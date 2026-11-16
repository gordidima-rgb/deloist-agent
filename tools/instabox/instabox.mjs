#!/usr/bin/env node
// Deloist on the user's own free Instabox machine, step by step.
// Every step is idempotent: run it again after any error. Secrets are written to files, never printed.
// Usage: node tools/instabox/instabox.mjs <signin|create|deploy|open|status|update|delete|all> [--image <ref>] [--yes]
// Runbook for people and models: tools/instabox/INSTABOX.md
// Instabox API facts: https://instabox.sh/docs/llms.txt (checked live on 16.11.2026, see the runbook).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';

const API = 'https://api.instabox.sh';
const MACHINE_NAME = 'deloist';
const TEMPLATE = 'agent-browser'; // free plan: agent-browser, android, omarchy, ci
const PORT = 8080;
const USER_ID = 'owner';
const DEFAULT_IMAGE = 'ghcr.io/gordidima-rgb/deloist-server:stand'; // only tag published on 16.11.2026
const IMAGE_RE = /^ghcr\.io\/gordidima-rgb\/deloist-server:[A-Za-z0-9._-]{1,64}$/;

const DIR = process.env.DELOIST_INSTABOX_DIR || path.join(os.homedir(), '.deloist', 'instabox');
const F = {
  key: path.join(DIR, 'api-key'),          // ibk_… (secret)
  claim: path.join(DIR, 'claim.json'),     // pending sign-up (secret claimToken)
  gateway: path.join(DIR, 'gateway.json'), // Ed25519 private key + CREW_SECRETS_KEY (secret)
  machine: path.join(DIR, 'machine.json'), // id, origin (not secret)
};

// ---------- output ----------
const ok = (msg) => console.log(`OK: ${msg}`);
const info = (msg) => console.log(`.. ${msg}`);
class StepError extends Error { constructor(code, msg, fix) { super(msg); this.code = code; this.fix = fix; } }
const fail = (code, msg, fix) => { throw new StepError(code, msg, fix); };

// ---------- files ----------
function readJson(file) { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return null; } }
function writeSecret(file, text) {
  fs.mkdirSync(DIR, { recursive: true });
  fs.writeFileSync(file, text, { mode: 0o600 });
}
function apiKey() {
  try { return fs.readFileSync(F.key, 'utf8').trim(); } catch { return null; }
}
function scrub(text, extra = []) {
  let t = String(text ?? '');
  for (const s of [apiKey(), ...extra]) if (s) t = t.replaceAll(s, '<secret>');
  return t;
}

// ---------- HTTP ----------
async function call(method, p, body, { auth = true, headers = {}, okStatus } = {}) {
  const h = { 'Content-Type': 'application/json', ...headers };
  if (auth) {
    const key = apiKey();
    if (!key) fail('NO_KEY', 'no Instabox key yet', 'run: node tools/instabox/instabox.mjs signin');
    h['X-API-Key'] = key;
  }
  if (method !== 'GET') h['Idempotency-Key'] = crypto.randomUUID();
  let res;
  try {
    res = await fetch(API + p, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
  } catch (e) {
    fail('NETWORK', `cannot reach ${API}: ${e.cause?.code || e.message}`, 'check the internet connection and run the same command again');
  }
  const text = await res.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  if (okStatus ? !okStatus.includes(res.status) : res.status >= 400) {
    const code = json?.error?.code || json?.error || `HTTP_${res.status}`;
    if (res.status === 401) fail('KEY_REJECTED', 'Instabox rejected the key (revoked or expired)', `delete ${F.key} and run: node tools/instabox/instabox.mjs signin`);
    if (code === 'machine_cooldown') fail('COOLDOWN', `machine is in a forced pause (free plan run limit); retry after ${res.headers.get('retry-after') || '?'} s`, 'wait and run the same command again; nothing is broken');
    fail(String(code).toUpperCase(), `${method} ${p} → ${res.status}: ${scrub(json?.error?.message || text).slice(0, 300)}`, 'run the same command again; if it repeats, send this line to the developers');
  }
  return { status: res.status, json, headers: res.headers };
}

async function waitOperation(op, what) {
  for (let i = 0; i < 20; i++) {
    const { json } = await call('GET', `/v1/operations/${op.id}?wait=25s`);
    if (json.state === 'succeeded') return json;
    if (json.state === 'failed') fail('OPERATION_FAILED', `${what} failed: ${scrub(JSON.stringify(json.error)).slice(0, 300)}`, 'run the same command again');
  }
  fail('OPERATION_TIMEOUT', `${what} did not finish in 8 minutes`, 'run: node tools/instabox/instabox.mjs status');
}

async function exec(id, cmd, { stdin, timeoutSeconds = 120, secrets = [] } = {}) {
  const { json } = await call('POST', `/v1/machines/${id}/exec`, { cmd, stdin, timeoutSeconds });
  return { code: json.exitCode, out: scrub(json.stdout, secrets), err: scrub(json.stderr, secrets), timedOut: json.timedOut };
}

function openBrowser(url) {
  // System browser on purpose: the user's Google/GitHub sign-ins live there, and Google forbids embedded web views.
  const [cmd, args] = process.platform === 'win32' ? ['rundll32', ['url.dll,FileProtocolHandler', url]]
    : process.platform === 'darwin' ? ['open', [url]] : ['xdg-open', [url]];
  try { spawn(cmd, args, { detached: true, stdio: 'ignore' }).unref(); return true; } catch { return false; }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------- steps ----------
async function signin() {
  if (apiKey()) {
    const { status } = await call('GET', '/v1/machines', undefined, { okStatus: [200, 401] });
    if (status === 200) return ok('Instabox key is already saved and works');
    fs.rmSync(F.key, { force: true });
    info('saved key no longer works, signing in again');
  }
  let claim = readJson(F.claim);
  if (!claim || Date.now() > claim.expiresAt) {
    const { json } = await call('POST', '/v1/auth/agent', { name: 'Deloist', scopes: ['machines:read', 'machines:write', 'machines:delete'] }, { auth: false });
    claim = { claimToken: json.claimToken, userCode: json.userCode, url: json.verificationUriComplete, interval: json.interval || 5, expiresAt: Date.now() + (json.expiresIn || 86400) * 1000 };
    writeSecret(F.claim, JSON.stringify(claim));
  }
  console.log(`\nACTION FOR THE PERSON: open ${claim.url}\n  sign in with Google or GitHub if asked, check that the code is ${claim.userCode} and the name is "Deloist", press Approve.`);
  console.log(`  If after sign-in Instabox shows the machines list instead, open the same link again.\n`);
  openBrowser(claim.url);
  let interval = claim.interval * 1000;
  const until = Date.now() + 15 * 60 * 1000;
  while (Date.now() < until) {
    await sleep(interval);
    const { status, json } = await call('POST', '/v1/auth/agent/token', { claimToken: claim.claimToken }, { auth: false, okStatus: [200, 400] });
    if (status === 200 && json.key) {
      writeSecret(F.key, json.key);
      fs.rmSync(F.claim, { force: true });
      return ok(`approved by ${json.login || 'the account owner'}; key saved to ${F.key}`);
    }
    const e = json?.error?.code || json?.error;
    if (e === 'slow_down') interval += 5000;
    else if (e === 'expired_token') { fs.rmSync(F.claim, { force: true }); fail('CLAIM_EXPIRED', 'the approval code expired', 'run signin again'); }
    else if (e === 'access_denied') { fs.rmSync(F.claim, { force: true }); fail('DENIED', 'the person pressed Deny', 'run signin again if that was a mistake'); }
    else if (e !== 'authorization_pending') fail('SIGNIN', `unexpected answer: ${e}`, 'run signin again');
  }
  fail('SIGNIN_WAIT', 'nobody approved within 15 minutes', 'run signin again: it reuses the same code for 24 hours');
}

async function findMachine() {
  const { json } = await call('GET', '/v1/machines');
  const list = Array.isArray(json) ? json : json.machines || json.items || json.data || [];
  return list.find((m) => m.name === MACHINE_NAME) || null;
}

async function create() {
  let m = await findMachine();
  if (!m) {
    const { json: op } = await call('POST', '/v1/machines', { name: MACHINE_NAME, templateId: TEMPLATE }, { okStatus: [200, 201, 202, 403, 409] })
      .then((r) => {
        if (r.status === 403 || r.status === 409) fail('LIMIT', `Instabox refused a new machine: ${r.json?.error?.message || r.status}`, 'the free plan allows one machine: delete the other machine in dashboard.instabox.sh (ask the person first), then run create again');
        return r;
      });
    const done = await waitOperation(op, 'machine creation');
    m = done.machine || (await findMachine());
    ok(`machine ${MACHINE_NAME} created`);
  } else ok(`machine ${MACHINE_NAME} already exists`);
  if (m.state === 'suspended' || m.state === 'stopped') {
    const { json: op } = await call('POST', `/v1/machines/${m.id}/${m.state === 'stopped' ? 'start' : 'resume'}`, {});
    await waitOperation(op, 'resume');
    ok('machine resumed');
  }
  const saved = readJson(F.machine) || {};
  fs.mkdirSync(DIR, { recursive: true });
  fs.writeFileSync(F.machine, JSON.stringify({ ...saved, id: m.id }, null, 2));
  return m.id;
}

function gateway() {
  let gw = readJson(F.gateway);
  if (!gw) {
    const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
    gw = {
      pub: publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex'),
      priv: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
      secretsKey: crypto.randomBytes(32).toString('hex'), // CREW_SECRETS_KEY; losing it loses the cloud's saved secrets
    };
    writeSecret(F.gateway, JSON.stringify(gw));
  }
  return gw;
}

async function publishPort(id) {
  const { json } = await call('GET', `/v1/machines/${id}/ports`);
  const list = Array.isArray(json) ? json : json.ports || json.items || json.data || [];
  let port = list.find((p) => p.port === PORT);
  if (port && port.access !== 'public') {
    await call('DELETE', `/v1/machines/${id}/ports/${PORT}`);
    port = null;
  }
  if (!port) port = (await call('POST', `/v1/machines/${id}/ports`, { port: PORT, name: 'deloist', access: 'public' })).json;
  if (!/^https:\/\/[a-z0-9-]+\.[a-z0-9.-]+$/.test(port.url || '')) fail('PORT_URL', `unexpected port address: ${port.url}`, 'send this line to the developers');
  return port.url.toLowerCase();
}

async function deploy(image) {
  if (!IMAGE_RE.test(image)) fail('IMAGE', `image ${image} is not a Deloist server image`, `use the default (${DEFAULT_IMAGE}) or another ghcr.io/gordidima-rgb/deloist-server tag`);
  const id = await create();
  const origin = await publishPort(id);
  ok(`public address ${origin}`);
  const gw = gateway();
  const env = [
    'CREW_CLOUD=1', 'HOST=0.0.0.0', `PORT=${PORT}`,
    `CREW_GATEWAY_PUBKEY=${gw.pub}`, `CREW_USER_ID=${USER_ID}`, `CREW_PUBLIC_ORIGIN=${origin}`,
    `CREW_SECRETS_KEY=${gw.secretsKey}`,
  ].join('\n') + '\n';
  // The image runs as uid 10001; the data folder lives on the persistent /data disk of the machine.
  const cmd = [
    'set -e',
    'umask 077',
    'sudo mkdir -p /data/deloist/data',
    'sudo chown 10001:10001 /data/deloist/data',
    'sudo chmod 700 /data/deloist/data',
    'sudo chown "$(id -u)" /data/deloist',
    'cat > /data/deloist/cloud.env',
    `docker pull -q ${image} >/dev/null`,
    'docker rm -f deloist >/dev/null 2>&1 || true',
    `docker run -d --name deloist --restart=always --cap-drop=ALL --security-opt no-new-privileges --pids-limit 512 --memory 6g -p ${PORT}:${PORT} -v /data/deloist/data:/data --env-file /data/deloist/cloud.env ${image} >/dev/null`,
    `for i in $(seq 1 60); do c=$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:${PORT}/ || true); [ "$c" != 000 ] && break; sleep 1; done`,
    'echo "http=$c"',
    'docker logs --tail 5 deloist 2>&1',
  ].join('\n');
  info('starting Deloist on the machine (first time downloads ~1.3 GB, about 20 s)');
  const r = await exec(id, cmd, { stdin: env, timeoutSeconds: 600, secrets: [gw.secretsKey] });
  if (r.code !== 0 || !/http=(200|303|403)/.test(r.out)) {
    fail('DEPLOY', `Deloist did not start (exit ${r.code}): ${(r.out + r.err).slice(-600)}`, 'run deploy again; if it repeats, send this output to the developers');
  }
  const saved = readJson(F.machine) || {};
  fs.writeFileSync(F.machine, JSON.stringify({ ...saved, id, origin, image }, null, 2));
  ok('Deloist is running in the cloud');
}

function pass(origin) {
  const gw = readJson(F.gateway);
  if (!gw) fail('NO_GATEWAY', 'no gateway key on this computer', 'run deploy first');
  const now = Math.floor(Date.now() / 1000);
  const payload = Buffer.from(JSON.stringify({ uid: USER_ID, aud: origin, iat: now, exp: now + 30, nonce: crypto.randomBytes(12).toString('hex') }));
  const key = crypto.createPrivateKey({ key: Buffer.from(gw.priv, 'base64'), format: 'der', type: 'pkcs8' });
  const sig = crypto.sign(null, Buffer.concat([Buffer.from('deloist-gw-v1\n'), payload]), key);
  return `${origin}/auth/gateway?t=${payload.toString('base64url')}.${sig.toString('base64url')}`;
}

async function open() {
  const m = readJson(F.machine);
  if (!m?.origin) fail('NOT_DEPLOYED', 'Deloist is not deployed yet', 'run: node tools/instabox/instabox.mjs all');
  await wake(m);
  openBrowser(pass(m.origin)); // one-time link, valid 30 s; never paste it into chats or logs
  ok('opened the cloud Deloist in the browser (the link works once, for 30 seconds)');
}

async function wake(m) {
  const { json } = await call('GET', `/v1/machines/${m.id}`);
  if (json.state === 'suspended' || json.state === 'stopped') {
    const { json: op } = await call('POST', `/v1/machines/${m.id}/${json.state === 'stopped' ? 'start' : 'resume'}`, {});
    await waitOperation(op, 'wake-up');
  }
}

async function status() {
  const m = readJson(F.machine);
  if (!apiKey()) return console.log('STATUS: not signed in to Instabox');
  if (!m?.id) return console.log('STATUS: no machine yet');
  const { status: st, json } = await call('GET', `/v1/machines/${m.id}`, undefined, { okStatus: [200, 404] });
  if (st === 404) return console.log('STATUS: machine was deleted in Instabox; run: node tools/instabox/instabox.mjs all');
  const cooldown = json.cooldownUntil ? ` forced pause until ${json.cooldownUntil}` : '';
  console.log(`STATUS: machine ${json.state}${cooldown}; address ${m.origin || 'not published'}; image ${m.image || '-'}`);
  console.log(`  sleeps after ${json.autoSuspendAfterSeconds}s idle; run limit ${json.maxRunMinutes} min; free=${json.free}`);
}

async function del(yes) {
  const m = readJson(F.machine);
  if (!m?.id) return ok('nothing to delete');
  if (!yes) fail('CONFIRM', 'deleting removes the machine and all bot data in the cloud', 'ask the person; only with a clear yes run: delete --yes');
  await call('DELETE', `/v1/machines/${m.id}`, undefined, { headers: { 'X-Confirm-Delete': m.id }, okStatus: [200, 202, 204, 404] });
  fs.rmSync(F.machine, { force: true });
  ok('machine deleted; the Instabox key stays saved (revoke it in dashboard.instabox.sh → API Keys if not needed)');
}

// ---------- main ----------
const [cmd, ...rest] = process.argv.slice(2);
const flag = (name) => rest.includes(name);
const image = rest.includes('--image') ? rest[rest.indexOf('--image') + 1] : DEFAULT_IMAGE;
const steps = {
  signin, create, open, status,
  deploy: () => deploy(image),
  update: () => deploy(image),
  delete: () => del(flag('--yes')),
  all: async () => { await signin(); await deploy(image); await open(); },
};
if (!steps[cmd]) {
  console.log('usage: node tools/instabox/instabox.mjs <signin|create|deploy|open|status|update|delete|all> [--image <ref>] [--yes]');
  process.exit(2);
}
try {
  await steps[cmd]();
} catch (e) {
  if (e instanceof StepError) {
    console.log(`ERROR ${e.code}: ${e.message}\nFIX: ${e.fix}`);
    process.exit(1);
  }
  console.log(`ERROR UNEXPECTED: ${scrub(e.stack || e.message).slice(0, 800)}\nFIX: send this output to the developers`);
  process.exit(1);
}
