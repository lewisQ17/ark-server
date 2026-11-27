// run-rust-pve.js <binary> — start the stubs of stubs.js, then the Rust dashboard pointed at them (same environment
// the original gets in run-original-pve.js). Passes SIGTERM/SIGINT on and exits with the child.
'use strict';
const { spawn } = require('child_process');
const { startStubs } = require('./stubs');

(async () => {
  const stubs = await startStubs();
  const child = spawn(process.argv[2], process.argv.slice(3), { env: { ...process.env, ...stubs.env }, stdio: 'inherit' });
  const stop = (sig) => () => { child.kill(sig); };
  process.on('SIGTERM', stop('SIGTERM'));
  process.on('SIGINT', stop('SIGINT'));
  process.on('SIGHUP', stop('SIGTERM'));
  child.on('exit', (code, sig) => { stubs.close(); process.exit(code === null ? 1 : code); });
})();
