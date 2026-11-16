// Deloist user-chrome pipe protocol v1.
// Shared by background.js, popup.js and tools/check_protocol.mjs.

const SALT_TEXT = 'deloist-user-chrome-v1';
const ITERATIONS = 200000;

export const DIR_APP_TO_EXT = 0x01;
export const DIR_EXT_TO_APP = 0x02;

export function hexToBytes(hex) {
  if (typeof hex !== 'string' || hex.length % 2 !== 0) throw new Error('bad hex');
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    const byte = parseInt(hex.substr(i * 2, 2), 16);
    if (Number.isNaN(byte)) throw new Error('bad hex');
    out[i] = byte;
  }
  return out;
}

export function bytesToHex(bytes) {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export function normalizeCode(code) {
  let collapsed = '';
  let prevSpace = false;
  for (const c of String(code).trim().toLowerCase()) {
    if (/\s/.test(c)) {
      if (!prevSpace) collapsed += ' ';
      prevSpace = true;
    } else {
      collapsed += c;
      prevSpace = false;
    }
  }
  return collapsed.trim();
}

export async function deriveK(code) {
  const enc = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey('raw', enc.encode(normalizeCode(code)), 'PBKDF2', false, ['deriveBits']);
  const bits = await crypto.subtle.deriveBits(
    { name: 'PBKDF2', salt: enc.encode(SALT_TEXT), iterations: ITERATIONS, hash: 'SHA-256' },
    keyMaterial,
    256
  );
  return bytesToHex(new Uint8Array(bits));
}

export async function hmacSha256(keyHex, dataBytes) {
  const key = await crypto.subtle.importKey('raw', hexToBytes(keyHex), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const sig = await crypto.subtle.sign('HMAC', key, dataBytes);
  return new Uint8Array(sig);
}

export async function sessionKey(kHex, saltAppHex, saltExtHex) {
  const enc = new TextEncoder();
  const sa = hexToBytes(saltAppHex);
  const se = hexToBytes(saltExtHex);
  const data = new Uint8Array(7 + sa.length + se.length);
  data.set(enc.encode('session'), 0);
  data.set(sa, 7);
  data.set(se, 7 + sa.length);
  const mac = await hmacSha256(kHex, data);
  return bytesToHex(mac);
}

export async function frameMac(sHex, dir, n, body) {
  const enc = new TextEncoder();
  const bodyBytes = enc.encode(body);
  const data = new Uint8Array(1 + 8 + bodyBytes.length);
  data[0] = dir;
  const view = new DataView(data.buffer);
  view.setBigUint64(1, BigInt(n)); // big-endian
  data.set(bodyBytes, 9);
  const mac = await hmacSha256(sHex, data);
  return bytesToHex(mac);
}

export async function buildFrame(sHex, dir, n, bodyObj) {
  const body = typeof bodyObj === 'string' ? bodyObj : JSON.stringify(bodyObj);
  const mac = await frameMac(sHex, dir, n, body);
  return { n, body, mac };
}

export async function verifyFrame(sHex, dir, n, body, macHex) {
  let calc;
  try { calc = await frameMac(sHex, dir, n, body); } catch { return false; }
  if (typeof macHex !== 'string' || calc.length !== macHex.length) return false;
  // без раннего выхода
  let d = 0;
  for (let i = 0; i < calc.length; i++) d |= calc.charCodeAt(i) ^ macHex.charCodeAt(i);
  return d === 0;
}

export function parseFrame(raw) {
  if (!raw || typeof raw !== 'object') return null;
  if (typeof raw.n !== 'number' || typeof raw.body !== 'string' || typeof raw.mac !== 'string') return null;
  return { n: raw.n, body: raw.body, mac: raw.mac };
}

export function randomSaltHex() {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return bytesToHex(bytes);
}

/** Соль пира: ровно 32 hex-символа. */
export function validSalt(salt) {
  return typeof salt === 'string' && /^[0-9a-fA-F]{32}$/.test(salt);
}

/**
 * Состояние одного подключения (зеркало Conn в src/pipe_auth.rs).
 * Сторона расширения: dirIn = DIR_APP_TO_EXT, dirOut = DIR_EXT_TO_APP.
 * onMessage(raw) -> { action: 'ignore' | 'close' | 'deliver' | 'session_ready', msg? }.
 * Входящие обрабатываются по одному, в порядке прихода; исходящие получают n и уходят
 * через post() в одной цепочке, поэтому порядок n совпадает с порядком в канале.
 */
export class Conn {
  constructor({ k = null, localSalt = randomSaltHex(), dirIn = DIR_APP_TO_EXT, dirOut = DIR_EXT_TO_APP } = {}) {
    this.k = k || null;
    this.localSalt = localSalt;
    this.dirIn = dirIn;
    this.dirOut = dirOut;
    this.s = null;
    this.lastIn = 0;
    this.outN = 1;
    this.helloSeen = false;
    this.localReadySent = false;
    this.peerReady = false;
    this.peerHello = null;
    this.peerStatus = null;
    this._in = Promise.resolve();
    this._out = Promise.resolve();
  }

  get paired() { return !!this.k; }
  get ready() { return !!this.s && this.peerReady; }

  helloMessage(extra = {}) { return { type: 'hello', salt: this.localSalt, ...extra }; }

  onMessage(raw) {
    const run = this._in.then(() => this._handle(raw)).catch(() => ({ action: 'close' }));
    this._in = run.then(() => undefined, () => undefined);
    return run;
  }

  async _handle(raw) {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return { action: 'close' };
    if (raw.type === 'hello') {
      if (this.helloSeen) return { action: 'close' };
      this.helloSeen = true;
      if (!validSalt(raw.salt)) return { action: 'close' };
      this.peerHello = raw;
      if (!this.k) return { action: 'ignore' };
      const appSalt = this.dirOut === DIR_APP_TO_EXT ? this.localSalt : raw.salt;
      const extSalt = this.dirOut === DIR_APP_TO_EXT ? raw.salt : this.localSalt;
      this.s = await sessionKey(this.k, appSalt, extSalt);
      return { action: 'session_ready' };
    }
    // status без подписи — справка для человека: запоминаем, не закрываем и не действуем
    if (raw.type === 'status') {
      this.peerStatus = raw;
      return { action: 'ignore' };
    }
    // без пары обрабатываются только hello и status
    if (!this.k) return { action: 'ignore' };
    if (!this.s) return { action: 'close' };
    const frame = parseFrame(raw);
    if (!frame || !Number.isSafeInteger(frame.n) || frame.n < 1) return { action: 'close' };
    if (frame.n !== this.lastIn + 1) return { action: 'close' };
    if (!(await verifyFrame(this.s, this.dirIn, frame.n, frame.body, frame.mac))) return { action: 'close' };
    this.lastIn = frame.n;
    let msg;
    try { msg = JSON.parse(frame.body); } catch { return { action: 'close' }; }
    if (msg && msg.type === 'ready') {
      if (this.peerReady) return { action: 'ignore' };
      this.peerReady = true;
      return { action: 'deliver', msg };
    }
    if (!this.peerReady) return { action: 'ignore' };
    return { action: 'deliver', msg };
  }

  /**
   * Кадр для отправки. n назначается, подписывается и отдаётся в post() внутри одной цепочки.
   * До своего ready уходит только ready. Возвращает кадр или null.
   */
  seal(message, post) {
    const run = this._out.then(async () => {
      if (!this.s) return null;
      const isReady = !!message && message.type === 'ready';
      if (!isReady && !this.localReadySent) return null;
      const frame = await buildFrame(this.s, this.dirOut, this.outN, message);
      this.outN += 1;
      if (isReady) this.localReadySent = true;
      if (post) post(frame);
      return frame;
    }).catch(() => null);
    this._out = run.then(() => undefined);
    return run;
  }
}
