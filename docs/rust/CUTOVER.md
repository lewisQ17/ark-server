# Cutover: Node backend to the Rust server on CT120

For the person who does the live switch (the manager). Nothing in this repo has been deployed or run against a real host:
all proof is tests plus the replay against local stubs (`docs/rust/DECISIONS.md` ADR-005). Expected downtime: about 1 minute.

## What changes and what must stay

| | Now (live, July backend + frontend of 2026-11-17) | After |
|---|---|---|
| Image | `ark-front:20261117` (Node, derived from the July image) | `ark-dashboard-rs:<sha>` (static musl binary on Alpine, built from `dashboard/Dockerfile`) |
| Process | `node server.js` as `node` (uid 1000) | `/usr/local/bin/ark-dashboard` as uid 1000 |
| Frontend | in the image (`COPY frontend`), identical to repo `dashboard/frontend` (unchanged since a86ad6b) | same files, `ARK_FRONTEND_DIR=/app/frontend` (set by the image) |
| Listens | `PORT` (8787), `BIND` | same variables, same defaults |
| Auth | none in the July backend; the nginx gate (`ark-dashboard-gate`) asks the login | the server itself requires `DASH_USER`/`DASH_PASS` (fail closed), or `DASH_AUTH_DISABLED=1` |
| TLS to Proxmox | unpinned (July code) | **pinned**: `PVE_TLS_FINGERPRINT` is mandatory, or `PVE_TLS_INSECURE=1` |

Must stay exactly as it is live: container name `ark-dashboard`, the compose service name, `env_file: ./deploy/.env`, the **gate**
(`gate/default.conf`, `gate/htpasswd`) and `expose: 8787` instead of `ports:` (do not put `ports:` back: the app would stand
next to the gate unprotected), the Traefik `dash-basic` middleware, host port 8790 served by the gate, no volumes and no
extra capabilities (the Rust image needs none), uid 1000 (writes nothing), the env keys of `deploy/.env` (below). The server
keeps its state in memory only (audit log, history), exactly like the Node one: a restart empties both.

**Guest agent rule** (VM 1050, see `reference_ark_vm_guest_agent`): never two dashboards at once and no parallel guest-exec.
`--force-recreate` stops the old container before the new one starts. Do not start the new image next to the old one "to compare".

## 0. Before touching CT120

1. Build and keep the image on the workstation (CT120 has no internet), from the repo at the commit being deployed:
   ```sh
   cd ~/Wall-LE/code/ark-server/dashboard
   TAG=$(git rev-parse --short HEAD)
   /opt/podman/bin/podman --connection rustbuild build --format docker --platform linux/amd64 -t localhost/ark-dashboard-rs:$TAG .
   /opt/podman/bin/podman --connection rustbuild save -o /tmp/ark-dashboard-rs.tar localhost/ark-dashboard-rs:$TAG
   ```
   (the image was proven to build; it was not started against anything.) Before that: `cd server && cargo test --locked &&
   cargo clippy --locked --all-targets -- -D warnings && sh tests/replay.sh | tail -1` must say `167/167 cases passed`.
2. Read what runs now (read-only), via PVE-M like the other CT120 deploys:
   ```sh
   ssh pve "pct exec 120 -- sh -c 'cd /opt/ark-dashboard && cat docker-compose.yml && docker ps --filter name=ark-dashboard --format \"{{.Names}} {{.Image}} {{.Status}}\" && docker inspect ark-dashboard --format \"{{.Image}}\" && ls gate && grep -n -i \"authorization\\|proxy_pass\\|proxy_set_header\" gate/default.conf && sed \"s/=.*/=<set>/\" deploy/.env | sort'"
   ```
3. Check `deploy/.env` against what the Rust server needs (it **exits with code 2 and a clear ERROR line** on a bad config, so a
   mistake shows as a restart loop in `docker logs`, not as a half-working dashboard):

   | Key | Needed | Note |
   |---|---|---|
   | `ARK_EXEC_MODE=proxmox` | set it explicitly | auto-detected from `PVE_PASSWORD` otherwise |
   | `PVE_HOST`, `PVE_NODE`, `PVE_VMID`, `PVE_PASSWORD` | yes | `PVE_PORT` (8006), `PVE_USER` (root@pam) default |
   | `PVE_TLS_FINGERPRINT` | yes (new) | SHA-256 of the PVE certificate, colons optional: `echo \| openssl s_client -connect <PVE_HOST>:8006 2>/dev/null \| openssl x509 -fingerprint -sha256 -noout`. If that node is down, use `PVE_TLS_INSECURE=1` for now and pin later |
   | `UNIFI_HOST`, `UNIFI_API_KEY`, `ARK_VM_IP` | only if the UDM forward is wanted | all three or none; then `UNIFI_TLS_FINGERPRINT` (or `UNIFI_TLS_INSECURE=1`) is mandatory too. Leave them out if unsure: a half set is silently "off" |
   | `BIND=0.0.0.0`, `PORT=8787` | yes | the gate proxies to 8787; the default bind is 127.0.0.1 and would make the gate answer 502 |
   | `DASH_USER`, `DASH_PASS` **or** `DASH_AUTH_DISABLED=1` | one of the two | see below |
   | `POLL_INTERVAL_MS` | optional | 6000 default |
   | `TRUSTED_PROXIES`, `ALLOWED_ORIGINS` | optional | only needed for the Origin check when the browser origin differs from the `Host` the app sees (see below) |

   Auth: the gate forwards the browser's `Authorization` header to the app unless `gate/default.conf` overrides it
   (`proxy_set_header Authorization`). If it does **not** override it, set `DASH_USER`/`DASH_PASS` to the same values as the gate
   (`~/.claude/bin/secret get dashboards/basic-user` / `dashboards/basic-pw`): then the login works through the gate and Traefik.
   If it overrides or strips the header, set `DASH_AUTH_DISABLED=1` (the container is only reachable through the gate because of
   `expose:`; the server prints a WARNING at start, that is expected).
   Origin check: POST/PUT/PATCH/DELETE with an `Origin` header are refused (403) unless the host in `Origin` equals `Host` or
   `X-Forwarded-Host` as the app sees it. If the dashboard is used through a name the gate/Traefik does not pass on as `Host`,
   add that origin to `ALLOWED_ORIGINS` (e.g. `https://ark.lan.quick-corp.org`), else buttons (start/stop/save) answer 403.
4. Back up: `cp -p docker-compose.yml docker-compose.pre-rust.yml; cp -p deploy/.env deploy/.env.pre-rust; docker tag <image id from step 2> ark-dashboard:pre-rust-20261128`
   (the current image also stays under its tag `ark-front:20261117`).

## 1. Switch

```sh
scp /tmp/ark-dashboard-rs.tar pve:/tmp/ && ssh pve "pct push 120 /tmp/ark-dashboard-rs.tar /root/ark-dashboard-rs.tar && rm /tmp/ark-dashboard-rs.tar"
ssh pve "pct exec 120 -- sh -c 'set -e; cd /opt/ark-dashboard
  docker load -i /root/ark-dashboard-rs.tar && rm -f /root/ark-dashboard-rs.tar
  # edit deploy/.env as decided in step 0.3, then point the compose file at the new image:
  #   image: localhost/ark-dashboard-rs:<TAG>      (replace the old image: line; delete any build: line)
  docker compose config -q
  docker compose up -d --no-build --force-recreate ark-dashboard'"
```
Only the `image:` line of the compose file changes; keep the gate service and `expose`. Never `--build`.

## 2. Check (in this order, each about a poll cycle apart; all read-only and served from the cache, no extra guest-exec)

```sh
docker ps --filter name=ark-dashboard --format '{{.Names}} {{.Image}} {{.Status}}'      # (healthy) after about 15 s
docker logs --tail 20 ark-dashboard        # first line: "ARK dashboard on 0.0.0.0:8787 → proxmox"; no "ERROR:", no "[syncMaps]" lines repeating
# through the gate, with the shared login:
curl -s -u "$(secret get dashboards/basic-user):$(secret get dashboards/basic-pw)" http://192.168.3.196:8790/api/health
```
Expected: `{"ok":true,"connected":true,...,"error":null,"maps":N}` with N equal to the number of worlds before the switch.
Then `/api/state`: `host` is not null (name, cpus, memory, disk), every world has the same `status` as before (running/stopped,
not `unknown`), `hist` fills up over the next cycles. Open the dashboard in the browser (Authentik + Traefik): worlds, graphs,
console (one click on a console or backups tab is one guest-exec: do it once, not in a loop). If maps are created or deleted
via the UI with `UNIFI_*` set, the answer carries `"forward":{"ok":true,...}`.
If PVE-B is down (VM 1050 unreachable) `connected` stays true but `host` is null and worlds show `unknown`: that is the
outage behaviour of the Node backend too; it recovers by itself.

## 3. Roll back (about 30 seconds)

```sh
ssh pve "pct exec 120 -- sh -c 'cd /opt/ark-dashboard && cp -p docker-compose.pre-rust.yml docker-compose.yml && cp -p deploy/.env.pre-rust deploy/.env && docker compose up -d --no-build --force-recreate ark-dashboard'"
```
This brings back `ark-front:20261117` (image still present; also tagged `ark-dashboard:pre-rust-20261128`). Roll back when: the
container does not become healthy within a minute, `connected` is false for more than two poll cycles while the VM is up, worlds show
`unknown` while the Node backend showed them correctly, or buttons answer 403/401 and fixing `ALLOWED_ORIGINS`/`DASH_*` does not help
in a few minutes.

## 4. Afterwards

Keep the old image and `*.pre-rust` files for a week, then remove them. Run `python3 -I bvw.py run --only diensten` and the
rest of the bron-van-waarheid routine in `~/Wall-LE/code/bron-van-waarheid` (the dashboard image and runtime changed), update
the ark-server-dashboard notes in the brein (diary + the "nooit --build" warning: the deploy is now a loaded image, the
`--build` problem is gone because there is no npm layer anymore), and `reference_ark_dashboard` in memory.

## What was not verified (be honest with yourself)

- The live `deploy/.env`, the compose file and `gate/default.conf` were not read (no access allowed); step 0.2 reads them.
- Nothing was started against Proxmox, UniFi or VM 1050. The Proxmox/UniFi clients are proven against local HTTPS stubs and a
  recording of the original; real PVE behaviours the stubs do not imitate (API tokens, 2FA realms, ticket expiry mid-call) are
  covered only by the reading of `proxmox.js` that the port follows line by line.
- `install.sh` (builds the server on a game host with rustup) was only syntax-checked, not run.
