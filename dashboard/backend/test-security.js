// Tests voor auth.js + security.js (node:test, geen extra dependencies).
const test = require('node:test');
const assert = require('node:assert');
const http = require('node:http');
const express = require('express');
const { basicAuth } = require('./auth');
const { securityHeaders, sameOrigin, authThrottle, clientIp, isTrusted } = require('./security');

function start(cfg, opts = {}) {
  const app = express();
  app.use(securityHeaders);
  app.use(authThrottle(basicAuth(cfg), { stepMs: 5, maxDelayMs: 20, ...opts }));
  app.use(sameOrigin(['https://ok.example']));
  app.use(express.json());
  app.get('/x', (_q, r) => r.json({ ok: true }));
  app.post('/x', (_q, r) => r.json({ ok: true }));
  return new Promise((res) => { const s = app.listen(0, '127.0.0.1', () => res(s)); });
}
function call(srv, method, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request({ port: srv.address().port, host: '127.0.0.1', method, path: '/x', headers }, (r) => {
      let b = ''; r.on('data', (d) => { b += d; }); r.on('end', () => resolve({ status: r.statusCode, headers: r.headers, body: b }));
    });
    req.on('error', reject);
    req.end(method === 'POST' ? '{}' : undefined);
  });
}
const good = 'Basic ' + Buffer.from('u:p').toString('base64');
const bad = 'Basic ' + Buffer.from('u:nope').toString('base64');

test('fail-closed zonder wachtwoord', async () => {
  const s = await start({ user: 'u', pass: '' });
  assert.strictEqual((await call(s, 'GET')).status, 503); s.close();
});
test('geen/verkeerde/juiste login', async () => {
  const s = await start({ user: 'u', pass: 'p' });
  assert.strictEqual((await call(s, 'GET')).status, 401);
  assert.strictEqual((await call(s, 'GET', { Authorization: bad })).status, 401);
  const ok = await call(s, 'GET', { Authorization: good });
  assert.strictEqual(ok.status, 200);
  assert.strictEqual(ok.headers['x-content-type-options'], 'nosniff');
  assert.strictEqual(ok.headers['x-frame-options'], 'DENY');
  s.close();
});
test('mislukte logins worden vertraagd, geldige niet', async () => {
  const s = await start({ user: 'u', pass: 'p' }, { stepMs: 40, maxDelayMs: 200 });
  const t0 = Date.now();
  for (let i = 0; i < 3; i++) await call(s, 'GET', { Authorization: bad });
  assert.ok(Date.now() - t0 >= 40 + 80 + 120 - 20, 'oplopende vertraging');
  const t1 = Date.now();
  assert.strictEqual((await call(s, 'GET', { Authorization: good })).status, 200);
  assert.ok(Date.now() - t1 < 40, 'geldige login niet vertraagd');
  s.close();
});
test('Origin-controle op muterende requests', async () => {
  const s = await start({ user: 'u', pass: 'p' });
  const h = { Authorization: good, 'Content-Type': 'application/json' };
  assert.strictEqual((await call(s, 'POST', h)).status, 200);                                   // geen Origin
  assert.strictEqual((await call(s, 'POST', { ...h, Origin: `http://127.0.0.1:${s.address().port}` })).status, 200); // zelfde host
  assert.strictEqual((await call(s, 'POST', { ...h, Origin: 'https://ok.example' })).status, 200); // toegestaan
  assert.strictEqual((await call(s, 'POST', { ...h, Origin: 'https://evil.example' })).status, 403);
  assert.strictEqual((await call(s, 'GET', { Authorization: good, Origin: 'https://evil.example' })).status, 200); // GET mag
  s.close();
});
test('client-IP: forwarded headers enkel van vertrouwde proxy', () => {
  const mk = (peer, h) => ({ socket: { remoteAddress: peer }, headers: h });
  assert.strictEqual(clientIp(mk('10.0.0.5', { 'x-forwarded-for': '1.2.3.4' }), []), '10.0.0.5');
  assert.strictEqual(clientIp(mk('10.0.0.5', { 'x-forwarded-for': '1.2.3.4' }), ['10.0.0.5']), '1.2.3.4');
  assert.strictEqual(clientIp(mk('::ffff:10.0.0.5', { 'cf-connecting-ip': '5.6.7.8' }), ['10.0.0.0/24']), '5.6.7.8');
  assert.strictEqual(clientIp(mk('9.9.9.9', { 'x-forwarded-for': '1.2.3.4' }), ['10.0.0.5']), '9.9.9.9');
  assert.strictEqual(clientIp(mk('10.0.0.5', { 'x-forwarded-for': 'rommel' }), ['10.0.0.5']), '10.0.0.5');
  assert.ok(isTrusted('10.0.0.7', ['10.0.0.0/24']) && !isTrusted('10.0.1.7', ['10.0.0.0/24']));
});
