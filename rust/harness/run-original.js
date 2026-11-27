// run-original.js — start the ORIGINAL dashboard backend (dashboard/backend/server.js)
// against an in-memory fake VM. Offline-safe: LocalExec (which would spawn sudo /
// systemctl) is swapped for FakeVm before server.js loads; there is no Proxmox,
// UniFi, RCON or SSH. Dummy credentials only.
'use strict';
const path = require('path');
const root = path.resolve(__dirname, '..', '..');
const be = path.join(root, 'dashboard', 'backend');

const env = process.env;
env.ARK_EXEC_MODE = 'local';
env.DASH_USER = env.DASH_USER || 'admin';
env.DASH_PASS = env.DASH_PASS || 'dummy-dash-pass';
env.BIND = '127.0.0.1';
env.POLL_INTERVAL_MS = env.POLL_INTERVAL_MS || '600000';   // only the startup poll; keeps recordings deterministic
for (const k of ['PVE_PASSWORD', 'PVE_HOST', 'UNIFI_API_KEY', 'UNIFI_HOST', 'ARK_VM_IP', 'ARK_RCON_PASSWORD']) delete env[k];

const localPath = require.resolve(path.join(be, 'local.js'));
require.cache[localPath] = { id: localPath, filename: localPath, loaded: true, exports: require('./fake-vm') };

// Let the startup poll finish before the port opens (the fake VM answers instantly).
const http = require('http');
const origListen = http.Server.prototype.listen;
http.Server.prototype.listen = function (...a) { setTimeout(() => origListen.apply(this, a), 700); return this; };

process.chdir(be);
require(path.join(be, 'server.js'));
