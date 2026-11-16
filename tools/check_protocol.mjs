// Проверка протокола v1 для расширения: векторы и состояние подключения (Conn из protocol.js).
// Запуск: node tools/check_protocol.mjs (без пакетов; нужен globalThis.crypto.subtle).
import { readFileSync } from 'node:fs';
const { deriveK, sessionKey, frameMac, verifyFrame, buildFrame, Conn, DIR_APP_TO_EXT, DIR_EXT_TO_APP } = await import('../extension/protocol.js');

const vectors = JSON.parse(readFileSync(new URL('../extension/protocol-vectors.json', import.meta.url), 'utf8'));

let failures = 0;
function assert(cond, msg) { if (!cond) { console.error('FAIL:', msg); failures++; } }

// --- векторы ---
for (const c of vectors.cases) {
  assert((await deriveK(c.code)) === c.K, `K mismatch for ${c.code}`);
  assert((await sessionKey(c.K, c.salt_app, c.salt_ext)) === c.S, 'S mismatch');
  assert((await frameMac(c.S, c.dir, c.n, c.body)) === c.mac, 'mac mismatch');
}
{
  const c = vectors.cases[0];
  const flipped = c.body.slice(0, 1) + (c.body.charAt(1) === '{' ? '[' : '{') + c.body.slice(2);
  assert((await verifyFrame(c.S, c.dir, c.n, flipped, c.mac)) === false, 'flipped body byte should fail');
  const other = c.dir === DIR_APP_TO_EXT ? DIR_EXT_TO_APP : DIR_APP_TO_EXT;
  assert((await verifyFrame(c.S, other, c.n, c.body, c.mac)) === false, 'dir swapped should fail');
  assert((await verifyFrame(c.S, c.dir, c.n + 1, c.body, c.mac)) === false, 'other n should fail');
  const f = await buildFrame(c.S, c.dir, c.n, JSON.parse(c.body));
  assert(f.body === c.body && f.mac === c.mac && f.n === c.n, 'buildFrame matches the vector');
}

// --- подключение: app-сторона (JS-зеркало) против расширения (Conn) ---
const K = '07'.repeat(32);
const SALT_APP = '01'.repeat(16);
const SALT_EXT = '02'.repeat(16);
const appConn = (k = K) => new Conn({ k, localSalt: SALT_APP, dirIn: DIR_EXT_TO_APP, dirOut: DIR_APP_TO_EXT });
const extConn = (k = K) => new Conn({ k, localSalt: SALT_EXT });

async function readyPair() {
  const app = appConn();
  const ext = extConn();
  const a1 = await app.onMessage(ext.helloMessage());
  const e1 = await ext.onMessage(app.helloMessage());
  assert(a1.action === 'session_ready' && e1.action === 'session_ready', 'hello derives the session');
  const appReady = await app.seal({ type: 'ready' });
  const extReady = await ext.seal({ type: 'ready' });
  assert((await ext.onMessage(appReady)).action === 'deliver', 'ext accepts the app ready');
  assert((await app.onMessage(extReady)).action === 'deliver', 'app accepts the ext ready');
  assert(app.ready && ext.ready, 'both ready');
  return { app, ext };
}

// круг в обе стороны
{
  const { app, ext } = await readyPair();
  const r1 = await ext.onMessage(await app.seal({ type: 'state', bots: [] }));
  assert(r1.action === 'deliver' && r1.msg.type === 'state', 'app -> ext delivered');
  const r2 = await app.onMessage(await ext.seal({ type: 'cdp', bot: 'b' }));
  assert(r2.action === 'deliver' && r2.msg.bot === 'b', 'ext -> app delivered');
}
// n повторён
{
  const { app, ext } = await readyPair();
  const frame = await app.seal({ type: 'state' });
  assert((await ext.onMessage(frame)).action === 'deliver', 'first n accepted');
  assert((await ext.onMessage(frame)).action === 'close', 'repeated n closes');
}
// n пропущен
{
  const { app, ext } = await readyPair();
  await app.seal({ type: 'state', i: 1 });
  const third = await app.seal({ type: 'state', i: 2 });
  assert((await ext.onMessage(third)).action === 'close', 'skipped n closes');
}
// второй hello (с солью и без)
{
  const ext = extConn();
  const app = appConn();
  assert((await ext.onMessage(app.helloMessage())).action === 'session_ready', 'first hello');
  assert((await ext.onMessage(app.helloMessage())).action === 'close', 'second hello with salt closes');
  const ext2 = extConn();
  await ext2.onMessage(app.helloMessage());
  assert((await ext2.onMessage({ type: 'hello' })).action === 'close', 'second hello without salt closes');
}
// плохая подпись
{
  const { app, ext } = await readyPair();
  const frame = await app.seal({ type: 'state' });
  const bad = { ...frame, body: '{"type":"stop_all"}' };
  assert((await ext.onMessage(bad)).action === 'close', 'bad MAC closes');
  const { app: app2, ext: ext2 } = await readyPair();
  const frame2 = await app2.seal({ type: 'state' });
  const bad2 = { ...frame2, mac: '00'.repeat(32) };
  assert((await ext2.onMessage(bad2)).action === 'close', 'zero MAC closes');
}
// плохая соль
for (const salt of ['0102', '01'.repeat(8), 'zz' + '01'.repeat(15), '01'.repeat(17), 5, undefined]) {
  const ext = extConn();
  assert((await ext.onMessage({ type: 'hello', salt })).action === 'close', `invalid salt ${String(salt)} closes`);
}
// кадр до ready игнорируется
{
  const app = appConn();
  const ext = extConn();
  await app.onMessage(ext.helloMessage());
  await ext.onMessage(app.helloMessage());
  assert((await app.seal({ type: 'state' })) === null, 'before its own ready only ready goes out');
  // кадр от приложения, подписанный верно, но не ready: расширение игнорирует
  const body = JSON.stringify({ type: 'cdp', bot: 'b', msg: { id: 1, method: 'Page.enable' } });
  const s = await sessionKey(K, SALT_APP, SALT_EXT);
  const frame = { n: 1, body, mac: await frameMac(s, DIR_APP_TO_EXT, 1, body) };
  assert((await ext.onMessage(frame)).action === 'ignore', 'frame before ready is ignored');
  assert(!ext.ready, 'still not ready');
}
// не-кадр после hello при паре
{
  const ext = extConn();
  await ext.onMessage(appConn().helloMessage());
  assert((await ext.onMessage({ type: 'cdp', bot: 'b' })).action === 'close', 'non-frame message closes when paired');
  assert((await extConn().onMessage({ n: 1, body: '{}' })).action === 'close', 'frame before hello closes when paired');
}
// status после hello: только справка, связь не рвётся
{
  const ext = extConn();
  const app = appConn();
  await ext.onMessage(app.helloMessage());
  assert((await ext.onMessage({ type: 'status', paired: false })).action === 'ignore', 'status after hello is ignored, not closed');
  assert(ext.peerStatus && ext.peerStatus.paired === false, 'status is remembered for display');
  await app.onMessage(ext.helloMessage());
  const appReady = await app.seal({ type: 'ready' });
  assert((await ext.onMessage(appReady)).action === 'deliver', 'the link still works after a status');
}
// отражённый кадр
{
  const { ext } = await readyPair();
  const own = await ext.seal({ type: 'cdp' });
  assert((await ext.onMessage(own)).action === 'close', 'reflected own frame closes');
}
// без пары: только hello и status
{
  const ext = extConn(null);
  assert((await ext.onMessage(appConn().helloMessage())).action === 'ignore', 'unpaired hello ignored');
  assert((await ext.onMessage({ type: 'status', paired: false })).action === 'ignore', 'unpaired status ignored');
  assert((await ext.onMessage({ type: 'cdp', bot: 'b' })).action === 'ignore', 'unpaired cdp ignored');
  assert((await ext.onMessage({ n: 1, body: '{}', mac: '00' })).action === 'ignore', 'unpaired frame ignored');
  assert((await ext.seal({ type: 'ready' })) === null, 'unpaired sends nothing sealed');
}
// исходящие уходят строго по номерам даже при одновременных вызовах
{
  const { app, ext } = await readyPair();
  const posted = [];
  await Promise.all([1, 2, 3, 4, 5].map((i) => app.seal({ type: 'state', i }, (f) => posted.push(f))));
  assert(posted.map((f) => f.n).join() === '2,3,4,5,6', `outgoing n in posting order: ${posted.map((f) => f.n)}`);
  for (const f of posted) assert((await ext.onMessage(f)).action === 'deliver', 'concurrent frames verify in order');
}

if (failures > 0) {
  console.error(`check_protocol: ${failures} failure(s)`);
  process.exit(1);
} else {
  console.log('check_protocol: OK');
}
