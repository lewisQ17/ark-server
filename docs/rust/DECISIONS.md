# Decisions: Rust in ARK-Server

Goal set by the owner (2026-11-27): "Rust where Rust can be and is better; the rest original; one version."
That gives three rules for this repo:

1. No per-function napi bridge in production (measured 2-10x slower and it leaks in the generated bridge).
   A part moves to Rust as a whole program (here: the dashboard server, axum) or it stays as it was.
2. Where a part is really replaced (proven equal and switched on), the original of that part is deleted and the
   Rust code lives where it naturally belongs (`dashboard/server/`). What was not adopted is removed too; it stays in git history.
3. Recordings that served as proof stay as regression tests next to the new code (`dashboard/server/tests/replay/`).

Status of every ADR: accepted.

## ADR-001: Scope
The dashboard backend (`dashboard/backend/*.js`, Express, about 2000 lines) is replaced by `dashboard/server` (axum).
Not replaced, kept as it was: `ark-manager.sh`, `rcon.py`, `install.sh` (adapted to build the server), `healthcheck.sh`,
the frontend (`dashboard/frontend`, UI plus vendored bundles; the Rust server serves it from disk).

## ADR-002: Server shape
One axum fallback handler routes by hand (`server.rs::dispatch`), in the order Express had: security headers on every
response, auth (with the delay on failed logins), Origin check on mutating methods, JSON body, routes, static folder, SPA
fallback, default 404 page. Routes are case-insensitive with an optional trailing slash, like Express. Parity is proven by
replay, not by types.

## ADR-003: Execution behind a trait, errors are values
`Exec` has five operations (`guest_shell`, `guest_exec`, `vm_status`, `rrddata`, `vm_power`). Three implementations:
`LocalExec` (sudo -u on the same machine), `Proxmox` (PVE API + qemu-guest-agent, port of `proxmox.js`) and `FakeVm`
(in memory, tests and replays only).
`guest_shell`/`guest_exec` return `Result<ExecOut, String>`: a transport failure (PVE unreachable, agent down, ticket
refused, timeout) is an `Err`, exactly where the JavaScript original threw. Every `ArkInstance` method that talks to the
host is fallible and every route maps `Err` to the 500 JSON the original sent. This matters in an outage: the poller
shows a world as `unknown` (not `stopped`), keeps the discovered maps, and recovers on its own (test
`an_unreachable_host_gives_unknown_not_stopped_and_500s_with_the_reason`).
Like the original, only `guest_shell` wraps a command in `runuser -u <user>`; `guest_exec` runs as the agent's user (root),
and guest calls are serialized (the guest agent handles one exec at a time; see CUTOVER.md).

## ADR-004: TLS pinning with a rustls verifier
`pinned.rs` replaces `pinned-agent.js`: the chain check is swapped for an exact SHA-256 match on the peer certificate
(colons and case ignored), signatures are still verified with the ring provider. Fails closed without a pin unless
`*_TLS_INSECURE=1`. Test `wrong_pin_never_sends_the_password` proves no request reaches a peer with another certificate.

## ADR-005: Proof is a recording of the original in proxmox mode
The original server was run in proxmox mode against two local HTTPS stubs (Proxmox API and UniFi controller, answering from
an in-memory fake VM, `tests/replay/stubs.js`), driven by 167 HTTP cases, and recorded with rust-anything (`app record`).
The Rust server replays that recording (`sh dashboard/server/tests/replay.sh`) against the same stubs.
Equivalence bar: status, content-type, the headers `app record` keeps and the body, with the original's own noise
(timestamps) as wildcards. The recording is not edited to hide a mismatch.
Not covered by any case: the leak guard against the original (covered by a Rust test), range requests and ETag, the real
`LocalExec` (argv construction is unit tested, nothing was spawned in this work), outage behaviour (Rust test only).

## ADR-006: The server is replaced as one piece
Routes share state (instance cache, audit log, poller, per-instance caches). Answering one route in Rust and proxying the
rest would read different state. So there is no front door and no kill switch: the whole backend is one binary.

## ADR-007: ark-ctl and ark-rs are not adopted
- `ark-rs` (`suggestPorts` through the napi bridge): the logic is part of the server now (`util.rs::suggest_ports`);
  the bridge crate is removed (rule 1).
- `ark-ctl` (pure parts of `ark-manager.sh` and `rcon.py` as a clap CLI that only prints plans): removed. The manager runs
  on the game VM as bash and shells out to docker, systemctl, proton and steamcmd; the pure parts (reading a conf value,
  an RCON packet) are a few lines each, and a Rust binary would add a build and deploy step on a machine without a
  toolchain, plus a process spawn per conf read, for no gain. Nothing called it. It stays in git history
  (commit `b4ca7e8`, directory `rust/ark-ctl`, 29 unit tests and differential tests against bash and python).
  `ark-manager.sh` and `rcon.py` stay as they are.

## ADR-008: Deliberate differences from the Node server
- `.env` files are not read (the Node server called dotenv); use real environment variables or systemd `EnvironmentFile=`
  (docker compose `env_file:` already does this).
- Transport errors read `fetch failed: <cause>` where Node had only `fetch failed`.
- HTTP calls to Proxmox/UniFi time out after 60 s (connect 10 s); Node's fetch waited up to 5 minutes.
- Express' development error pages (malformed JSON, undecodable `%`) are answered 400 with the same page layout but
  without a stack trace; `Last-Modified`/`ETag`, range requests and Express' prototype-key quirks are not reproduced.
- Time zone abbreviations in `ExecMainStartTimestamp` are all treated as UTC.
- `ARK_EXEC_MODE=fake` exists only in the Rust server, for tests and the replay of the in-memory VM.
