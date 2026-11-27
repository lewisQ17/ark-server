# Regression recording

`app-cases/` is a recording of the original Node/Express backend (`dashboard/backend`, removed in the Rust port, see
`docs/rust/DECISIONS.md` ADR-005) taken with rust-anything (`app record --http --no-traps --inputs cases.jsonl`) in proxmox mode.
The original ran against `stubs.js`: a local HTTPS Proxmox API and UniFi controller (self-signed certificate generated at start,
never committed) that answer from the in-memory VM of `fake-vm.js`. Credentials are dummies, everything stays on 127.0.0.1.

- `cases.jsonl` is the list of 167 requests (`gen-cases.js` writes it): every route, error paths, Express quirks, the Origin
  check, UniFi forwards (ok, replaced, refused by the controller), VM power, outage-free proxmox flows.
- `sh ../replay.sh` plays the recording against the Rust release binary behind the same stubs: 167/167.
- `run-rust-pve.js <binary>` starts the stubs, then the binary with the matching environment.

The recording is never edited to hide a difference. If the server's behaviour is changed on purpose, re-record is not possible
anymore (the original is gone): change the case in `app-cases/http/` by hand and explain the reason in the commit.
