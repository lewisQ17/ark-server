# Port map: ark-server

Goal: the dashboard backend and the pure parts of the ARK manager in Rust, proven against the originals without touching any
real host. Decisions: [DECISIONS.md](DECISIONS.md). Measured state: [PORT_MAP.md](PORT_MAP.md) (`rust-anything map --write`).
Next session: read [HANDOFF.md](HANDOFF.md) first.

## Units

| Unit | Crate / path | Status | Proof |
| --- | --- | --- | --- |
| `dashboard/backend/ark.js` `suggestPorts` | `rust/ark-rs` | done | diff 31/31, gate passed |
| dashboard backend (server.js, ark.js, auth.js, config.js, local.js, config-catalog.js) | `rust/dash` | done for local mode | replay 148/148 against the original on the fake VM; 28 unit + 8 integration tests; clippy `-D warnings` clean |
| `ark-manager.sh` pure parts + `rcon.py` | `rust/ark-ctl` | done | 29 unit tests + differential tests vs bash/python (6) ; clippy clean |
| `dashboard/backend/proxmox.js`, `unifi.js`, `pinned-agent.js` | not ported | blocked: needs live PVE/UDM to prove; VM1050 is off limits | - |
| `dashboard/backend/test-map-name-guard.js` | not ported | keep: a Node regression test of `ark.js` | the same guard is `discovery_drops_hostile_names` + `unsafe_service_name_is_refused` in `rust/dash` |
| `dashboard/frontend/` | not ported | keep / out of scope (UI + vendored bundles) | served unchanged by the Rust server |
| `install.sh`, `healthcheck.sh`, interactive menus, install/update/start/stop/restart, `create_optimized_game_settings`, graphics presets, mods UI | not ported | out of scope: execute docker/systemctl/proton/ssh or are interactive | - |

## Numbers

- `app replay` of the original: 148/148. Of the Rust server (`--build "sh rust/dash/run-fake.sh"`): 148/148.
- Fallback rate of the Rust server: 0% (it never starts the original). The generated front door `rust/app` still shows 100% and is
  kept only as the baseline.
- Not covered by any recorded case: leak guard against the original (covered by a Rust integration test only), `Last-Modified`/range,
  HEAD, the real `LocalExec` (argv construction is unit tested, nothing was spawned), Proxmox/UniFi modes.

## Open
- Port `proxmox.js` (reqwest + pinned TLS fingerprint) and prove it against a recorded PVE stub, then the Rust backend can replace
  the Node one on CT120. Needs a decision: record against a stub of the PVE API.
- `app lift` was not run: the Rust server is a hand-written crate, not a wired front door.
- `ark-ctl`: `create_optimized_game_settings`, mod name lookups (network), `check_map_health` and the dashboard table are not ported.
## ⚠️ Niet `enable`n vóór upstream-fix van de gegenereerde napi-brug (2026-11-27)

Een beveiligingsreview van de door rust-anything 5.0.1 **gegenereerde** brugcode (`rust/*/index.cjs`, `rust/*/src/js.rs`) vond:

- **Marker-injectie**: objecten met één sleutel `$bigint` / `$map` / `$set` / `$buffer` worden bij het decoderen als BigInt/Map/Set/Buffer "herleefd": invoer die er zo uitziet verandert van type.
- **`__proto__`**: `rDyn` bouwt objecten met `o[k] = …`; een sleutel `__proto__` zet het prototype van dat object.
- **Geen dieptegrens**: `serde_json`-lezer met `disable_recursion_limit()`: diep genest JSON → stack overflow → proces-abort (niet op te vangen).

Zolang de port **niet** ingeschakeld is (geen `enable`), raakt geen enkel verzoek deze code: ze draait enkel tijdens `diff`/`gate`. Deze bestanden worden door `refresh` overschreven, dus de fix hoort upstream in rust-anything. **Niet inschakelen** voordat dat opgelost is (of de brug per crate vervangen is door een eigen, veilige codec).

## Verificatie 2026-11-27 (app-replay)
Release-build van `rust/dash`: `cargo test` 28 unit + 8 integratie groen; `app replay --build "sh rust/dash/run-fake.sh"` 148/148 (enkel fake VM, geen Proxmox).
