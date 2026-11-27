// security.js — kleine hardening-laag zonder extra dependencies.
//   * securityHeaders: vaste veilige response-headers
//   * sameOrigin:      weigert muterende requests met een vreemde Origin (CSRF;
//                      Basic-auth wordt door de browser automatisch meegestuurd)
//   * clientIp:        X-Forwarded-For/CF-Connecting-IP enkel vertrouwen als de
//                      directe peer in TRUSTED_PROXIES zit
//   * authThrottle:    vertraagt herhaalde mislukte logins per client-IP (geen
//                      lockout: geldige logins worden nooit vertraagd)

const net = require('net');

function securityHeaders(_req, res, next) {
  res.set('X-Content-Type-Options', 'nosniff');
  res.set('X-Frame-Options', 'DENY');
  res.set('Referrer-Policy', 'no-referrer');
  res.set('Cross-Origin-Opener-Policy', 'same-origin');
  next();
}

const MUTATING = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

// allowed: extra toegestane origins (komma-lijst, bv. "https://ark.example.org").
function sameOrigin(allowed = []) {
  const extra = new Set(allowed.map((s) => String(s).trim()).filter(Boolean));
  return function (req, res, next) {
    if (!MUTATING.has(req.method)) return next();
    const origin = req.headers.origin;
    if (!origin) return next(); // geen browser-cross-site request (curl, scripts)
    if (extra.has(origin)) return next();
    let host = '';
    try { host = new URL(origin).host; } catch { host = ''; }
    const own = [req.headers.host, req.headers['x-forwarded-host']]
      .filter(Boolean).map((h) => String(h).split(',')[0].trim());
    if (host && own.includes(host)) return next();
    res.status(403).type('text/plain').send('Cross-origin request geweigerd.\n');
  };
}

function parseTrusted(list) {
  return String(list || '').split(',').map((s) => s.trim()).filter(Boolean);
}

function normalize(ip) {
  const s = String(ip || '');
  return s.startsWith('::ffff:') ? s.slice(7) : s;
}

function isTrusted(peer, trusted) {
  const p = normalize(peer);
  return trusted.some((t) => {
    if (t.includes('/')) {
      const [base, bitsRaw] = t.split('/');
      const bits = Number(bitsRaw);
      if (net.isIPv4(base) && net.isIPv4(p) && bits >= 0 && bits <= 32) {
        const toInt = (a) => a.split('.').reduce((n, o) => (n * 256) + Number(o), 0);
        const mask = bits === 0 ? 0 : (~0 << (32 - bits)) >>> 0;
        return ((toInt(base) & mask) >>> 0) === ((toInt(p) & mask) >>> 0);
      }
      return false;
    }
    return normalize(t) === p;
  });
}

function clientIp(req, trusted) {
  const peer = normalize(req.socket && req.socket.remoteAddress);
  if (trusted && trusted.length && isTrusted(peer, trusted)) {
    const cf = req.headers['cf-connecting-ip'];
    if (cf && net.isIP(String(cf).trim())) return String(cf).trim();
    const xff = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim();
    if (xff && net.isIP(xff)) return xff;
  }
  return peer;
}

// Wraps de auth-middleware: wordt `next` niet aangeroepen (= login mislukt, 401),
// dan wacht het antwoord oplopend. Geldige logins gaan meteen door.
function authThrottle(authMw, { trusted = [], windowMs = 60000, stepMs = 250, maxDelayMs = 3000 } = {}) {
  const fails = new Map(); // ip -> { n, t }
  return function (req, res, next) {
    const ip = clientIp(req, trusted);
    const now = Date.now();
    const rec = fails.get(ip);
    if (rec && now - rec.t > windowMs) fails.delete(ip);
    let passed = false;
    const origSend = res.send.bind(res);
    res.send = (...args) => {
      if (!passed && res.statusCode === 401 && req.headers.authorization) {
        const r = fails.get(ip) || { n: 0, t: now };
        r.n += 1; r.t = now; fails.set(ip, r);
        if (fails.size > 5000) fails.clear();
        const delay = Math.min(maxDelayMs, r.n * stepMs);
        res.send = origSend;
        setTimeout(() => origSend(...args), delay);
        return res;
      }
      return origSend(...args);
    };
    authMw(req, res, () => { passed = true; next(); });
  };
}

module.exports = { securityHeaders, sameOrigin, clientIp, authThrottle, parseTrusted, isTrusted };
