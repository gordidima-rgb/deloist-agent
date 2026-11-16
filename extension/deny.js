// Matcher for extension/deny.json. Used by background.js and tools/check_deny.mjs.
export let denyPatterns = [];
export let denyLoadFailed = false;

export function normHost(h) {
  try { h = (new URL('http://' + h)).hostname; } catch {}
  return String(h || '').toLowerCase().replace(/\.$/, '');
}

export function hostMatchesPattern(host, pattern) {
  host = normHost(host);
  pattern = String(pattern || '').toLowerCase().trim().replace(/\.$/, '');
  if (pattern.startsWith('*.')) {
    const suffix = pattern.slice(2).replace(/^\./, '');
    return host === suffix || host.endsWith('.' + suffix);
  }
  return host === pattern;
}

export function normPath(p) {
  let out = '';
  try { out = decodeURIComponent(String(p || '/')); } catch { out = String(p || '/'); }
  out = out.replace(/\/\/+/g, '/').toLowerCase();
  if (!out.startsWith('/')) out = '/' + out;
  return out;
}

export function isDeniedUrl(url) {
  if (denyLoadFailed) return true;
  let parsed;
  try { parsed = new URL(url); } catch { return true; }
  const host = parsed.hostname.toLowerCase().replace(/\.$/, '');
  const path = normPath(parsed.pathname || '/');
  for (const pat of denyPatterns) {
    const p = String(pat).trim().toLowerCase();
    const slash = p.indexOf('/');
    if (slash >= 0) {
      const phost = p.slice(0, slash);
      const ppath = normPath('/' + p.slice(slash + 1).replace(/^\/+/, ''));
      if (hostMatchesPattern(host, phost) && path.startsWith(ppath)) return true;
    } else if (hostMatchesPattern(host, p)) {
      return true;
    }
  }
  return false;
}

export async function loadDeny(fetchImpl, baseUrl) {
  try {
    const url = baseUrl || 'deny.json';
    const res = await fetchImpl(url);
    const data = await res.json();
    if (!Array.isArray(data.deny)) throw new Error('bad deny');
    denyPatterns = data.deny;
    denyLoadFailed = false;
  } catch {
    denyPatterns = [];
    denyLoadFailed = true;
  }
}