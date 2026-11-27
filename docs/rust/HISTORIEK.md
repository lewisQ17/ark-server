# History

## 2026-11-27: first port (branch work, not live)
`rust-anything` 5.0.1 used to record the Express backend offline (fake VM, 148 cases) and to port `suggestPorts` through its
napi bridge (diff 31/31, gate passed). `rust/dash` (axum) reproduced the backend for local mode (replay 148/148);
`rust/ark-ctl` ported the pure manager parts. proxmox.js and unifi.js were not ported (needed a live host to prove).
A security review of the generated napi bridge found marker injection (`$bigint`/`$map`/`$set`/`$buffer`), `__proto__`
assignment and an unbounded JSON depth; the bridge was therefore never enabled.

## 2026-11-27: owner's decision
"Rust where Rust can be and is better; the rest original; one version." No per-function bridge in production;
whole programs only; replaced parts lose their original; recordings stay as regression tests.

## 2026-11-28: dashboard backend replaced
- `security.js` (headers, Origin check, trusted proxies, login delay) ported; it had been added to the Node backend after the
  first recording.
- `proxmox.js`, `unifi.js` and `pinned-agent.js` ported (`proxmox.rs`, `unifi.rs`, `pinned.rs`), proven against in-process
  HTTPS stubs (9 tests) and by replay of a new recording of the original in proxmox mode (167 cases) against stubs.
- Transport failures became `Err` values all the way to the routes (ADR-003).
- Crate moved from `rust/dash` to `dashboard/server`; `dashboard/backend/*.js` deleted; Dockerfile builds the static
  musl binary; `deploy.sh` builds the image on the workstation and ships a tar; `install.sh` builds the server on the game host.
- `rust/ark-ctl`, `rust/ark-rs`, the generated front door `rust/app` and the first recording were removed (ADR-007).
