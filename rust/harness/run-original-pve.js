// run-original-pve.js — the ORIGINAL dashboard backend (dashboard/backend/server.js) in proxmox mode, talking to the
// local HTTPS stubs of stubs.js (a Proxmox API and a UniFi controller answering from an in-memory fake VM).
// Nothing leaves 127.0.0.1; dummy credentials only. Used to record the behaviour the Rust server must reproduce.
'use strict';
const path = require('path');
const root = path.resolve(__dirname, '..', '..');
const be = path.join(root, 'dashboard', 'backend');
const { startStubs } = require('./stubs');

(async () => {
  const stubs = await startStubs();
  Object.assign(process.env, stubs.env);
  // Let the startup poll finish before the port opens, so the first recorded request sees a settled state.
  const http = require('http');
  const origListen = http.Server.prototype.listen;
  http.Server.prototype.listen = function (...a) { setTimeout(() => origListen.apply(this, a), 1500); return this; };
  process.chdir(be);
  require(path.join(be, 'server.js'));
})();
