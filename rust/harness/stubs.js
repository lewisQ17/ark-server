// stubs.js — local HTTPS stand-ins for the Proxmox VE API and the UniFi controller, for recording the ORIGINAL
// dashboard backend in proxmox mode and replaying the Rust one against the same behaviour. Both listen on
// 127.0.0.1 only, with a self-signed certificate generated at start (never committed), and answer from memory:
// the guest-exec endpoints are served by the in-memory fake VM (fake-vm.js), so no real host is ever touched.
// All credentials are dummies. VM 1050 here is a synthetic id.
'use strict';
const https = require('https');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');
const { FakeVm } = require('./fake-vm');

const PVE_USER = 'root@pam';
const PVE_PASS = 'dummy-pve-pass';
const UNIFI_KEY = 'dummy-unifi-key';
const NODE = 'pve';
const VMID = 1050;

function makeCert() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ark-stub-cert-'));
  const key = path.join(dir, 'k.pem');
  const crt = path.join(dir, 'c.pem');
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', key, '-out', crt,
    '-days', '2', '-subj', '/CN=ark-stub', '-addext', 'subjectAltName=IP:127.0.0.1'], { stdio: 'ignore' });
  const cert = fs.readFileSync(crt);
  const fingerprint = new crypto.X509Certificate(cert).fingerprint256.replace(/:/g, '').toLowerCase();
  const out = { key: fs.readFileSync(key), cert, fingerprint };
  fs.rmSync(dir, { recursive: true, force: true });
  return out;
}

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
  });
}

function send(res, status, obj) {
  const body = typeof obj === 'string' ? obj : JSON.stringify(obj);
  res.writeHead(status, { 'content-type': 'application/json;charset=UTF-8' });
  res.end(body);
}

function pveHandler(vm) {
  const ticket = 'PVE:root@pam:DUMMYTICKET';
  const csrf = 'DUMMYCSRF';
  const results = new Map();   // pid -> { script, polls, result }
  let nextPid = 4000;
  const base = `/api2/json/nodes/${NODE}/qemu/${VMID}`;
  return async (req, res) => {
    const url = new URL(req.url, 'https://stub');
    const raw = await readBody(req);
    if (req.method === 'POST' && url.pathname === '/api2/json/access/ticket') {
      const p = new URLSearchParams(raw);
      if (p.get('username') !== PVE_USER || p.get('password') !== PVE_PASS) return send(res, 401, { data: null });
      return send(res, 200, { data: { ticket, CSRFPreventionToken: csrf, username: PVE_USER } });
    }
    if (!(req.headers.cookie || '').includes(`PVEAuthCookie=${ticket}`)) return send(res, 401, { data: null });
    if (req.method !== 'GET' && req.headers.csrfpreventiontoken !== csrf) return send(res, 403, { data: null, message: 'Permission check failed (invalid CSRF token)' });

    if (req.method === 'GET' && url.pathname === `${base}/status/current`) return send(res, 200, { data: await vm.vmStatus() });
    if (req.method === 'GET' && url.pathname === `${base}/rrddata`) {
      if (url.searchParams.get('timeframe') !== 'hour') return send(res, 400, { data: null });
      return send(res, 200, { data: await vm.rrddata() });
    }
    const pw = new RegExp(`^${base}/status/(start|stop|shutdown|reboot|suspend|resume)$`).exec(url.pathname);
    if (req.method === 'POST' && pw) return send(res, 200, { data: `UPID:${NODE}:0000:${pw[1]}:${VMID}:root@pam:` });

    if (req.method === 'POST' && url.pathname === `${base}/agent/exec`) {
      // PVE takes a repeated `command` and an optional `input-data` (raw to stdin).
      const form = new URLSearchParams(raw);
      const cmd = form.getAll('command');
      const inputData = form.has('input-data') ? form.get('input-data') : null;
      let out;
      if (cmd.length === 7 && cmd[0] === '/usr/sbin/runuser' && cmd[1] === '-u' && cmd[3] === '--' && cmd[4] === '/bin/bash' && cmd[5] === '-lc') {
        out = await vm.guestShell(cmd[6], { asUser: cmd[2], inputData });
      } else if (cmd.length === 3 && cmd[0] === '/bin/bash' && cmd[1] === '-lc') {
        out = await vm.guestShell(cmd[2], { inputData });
      } else {
        out = await vm.guestExec(cmd, { inputData });
      }
      const pid = nextPid++;
      // the disk probe is answered only on the second poll, so the clients' polling loop runs for real
      results.set(pid, { slow: /^df -B1 \//.test(cmd[cmd.length - 1] || ''), polls: 0, out });
      return send(res, 200, { data: { pid } });
    }
    if (req.method === 'GET' && url.pathname === `${base}/agent/exec-status`) {
      const r = results.get(Number(url.searchParams.get('pid')));
      if (!r) return send(res, 500, { data: null, message: 'no such pid' });
      r.polls += 1;
      if (r.slow && r.polls < 2) return send(res, 200, { data: { exited: 0 } });
      return send(res, 200, { data: { exited: 1, exitcode: r.out.exitcode, 'out-data': r.out.out, 'err-data': r.out.err } });
    }
    return send(res, 404, { data: null, message: `no route ${req.method} ${url.pathname}` });
  };
}

function unifiHandler() {
  const forwards = [];
  let seq = 0;
  const base = '/proxy/network/api/s/default/rest/portforward';
  const ok = (data) => ({ meta: { rc: 'ok' }, data });
  return async (req, res) => {
    const url = new URL(req.url, 'https://stub');
    const raw = await readBody(req);
    if (req.headers['x-api-key'] !== UNIFI_KEY) return send(res, 401, { meta: { rc: 'error', msg: 'api.err.LoginRequired' }, data: [] });
    if (req.method === 'GET' && url.pathname === base) return send(res, 200, ok(forwards));
    if (req.method === 'POST' && url.pathname === base) {
      let b;
      try { b = JSON.parse(raw); } catch (e) { return send(res, 400, { meta: { rc: 'error', msg: 'api.err.InvalidPayload' }, data: [] }); }
      if (b.name === 'ARK-Failmap') return send(res, 400, { meta: { rc: 'error', msg: 'api.err.PortInUse' }, data: [] });
      const rec = { _id: `pf${++seq}`, ...b };
      forwards.push(rec);
      return send(res, 200, ok([rec]));
    }
    const m = new RegExp(`^${base}/([^/]+)$`).exec(url.pathname);
    if (req.method === 'DELETE' && m) {
      const i = forwards.findIndex((f) => f._id === m[1]);
      if (i < 0) return send(res, 404, { meta: { rc: 'error', msg: 'api.err.ObjectNotFound' }, data: [] });
      forwards.splice(i, 1);
      return send(res, 200, ok([]));
    }
    return send(res, 404, { meta: { rc: 'error', msg: 'api.err.NotFound' }, data: [] });
  };
}

function listen(server) {
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));
}

// Starts both stubs; returns the environment the dashboard needs to talk to them.
async function startStubs() {
  const { key, cert, fingerprint } = makeCert();
  const vm = new FakeVm({ allowHostPower: true });
  const pve = https.createServer({ key, cert }, pveHandler(vm));
  const udm = https.createServer({ key, cert }, unifiHandler());
  const [pvePort, udmPort] = [await listen(pve), await listen(udm)];
  const env = {
    ARK_EXEC_MODE: 'proxmox',
    PVE_HOST: '127.0.0.1', PVE_PORT: String(pvePort), PVE_NODE: NODE, PVE_VMID: String(VMID),
    PVE_USER, PVE_PASSWORD: PVE_PASS, PVE_TLS_FINGERPRINT: fingerprint,
    UNIFI_HOST: `127.0.0.1:${udmPort}`, UNIFI_API_KEY: UNIFI_KEY, ARK_VM_IP: '10.9.9.9', UNIFI_TLS_FINGERPRINT: fingerprint,
    DASH_USER: 'admin', DASH_PASS: 'dummy-dash-pass', BIND: '127.0.0.1', POLL_INTERVAL_MS: '600000',
  };
  return { env, close: () => { pve.close(); udm.close(); } };
}

module.exports = { startStubs };
