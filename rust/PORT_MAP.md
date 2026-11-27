# Port map: rust-port

1 of 14 units have a Rust port; no original changed since it was recorded.

## Ports

| Unit | Crate | In Rust | Kept in the original | Cases | Last diff | Original |
| --- | --- | --- | --- | ---: | ---: | --- |
| `dashboard/backend/ark.js` | `rust/ark-rs` | `suggestPorts` | `discoverMaps`, `createMap`, `deleteMap` | 31 | 31/31 | unchanged |

## Not ported yet

In the order `analyze` suggests.

| Unit | Status | Lines | Exports | Readiness |
| --- | --- | ---: | ---: | ---: |
| `dashboard/backend/auth.js` | ready | 37 | 1 | 89 |
| `dashboard/backend/pinned-agent.js` | ready | 32 | 1 | 96 |
| `dashboard/backend/_wait.js` | ready | 11 | 0 | 90 |
| `dashboard/backend/unifi.js` | ready | 38 | 0 | 84 |
| `dashboard/backend/test-map-name-guard.js` | ready | 63 | 0 | 95 |
| `dashboard/backend/config.js` | ready | 70 | 0 | 92 |
| `dashboard/backend/local.js` | ready | 139 | 0 | 70 |
| `dashboard/backend/proxmox.js` | ready | 94 | 0 | 78 |
| `dashboard/backend/config-catalog.js` | ready | 43 | 0 | 97 |
| `dashboard/frontend/vendor/` | keep | 279 | 0 | 33 |
| `rust/harness/` | blocked | 419 | 0 | 29 |
| `dashboard/backend/server.js` | caution | 416 | 0 | 43 |
| `dashboard/frontend/` | blocked | 2449 | 0 | 0 |

3 of these units have no port on purpose (2 blocked, 1 kept in the original), so 10 units are left to port:

- `dashboard/frontend/vendor/`: keep, UI code: keep in the original language
- `rust/harness/`: blocked, Uses dynamic code (eval, new Function or computed imports)
- `dashboard/frontend/`: blocked, Uses dynamic code (eval, new Function or computed imports)

Regenerate with `rust-anything map --write`. Commit this file: when the original changes upstream, the next map marks every port whose recording it outdated.
