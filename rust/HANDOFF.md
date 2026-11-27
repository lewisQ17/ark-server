# Handoff

State (2026-11-27): `rust/dash` passes the full recording (148/148) on the fake VM; `rust/ark-ctl` and `rust/ark-rs` pass their tests/gate.

Run everything (Intel Mac: keep `CARGO_BUILD_JOBS=2`):

```sh
# rust-anything 5.0.1 on PATH (a local checkout, not installed globally)
export CARGO_BUILD_JOBS=2
(cd rust/dash && cargo build --release && cargo test && cargo clippy --all-targets -- -D warnings)
(cd rust/ark-ctl && cargo test && cargo clippy --all-targets -- -D warnings)
rust-anything app replay . --build "sh rust/dash/run-fake.sh"        # needs rust/dash built
rust-anything gate rust/ark-rs
node rust/harness/gen-cases.js && rust-anything app record . --http --no-traps --inputs rust/harness/cases.jsonl -- node rust/harness/run-original.js   # re-record (needs `npm ci` in dashboard/backend)
```

Never: `rust-anything enable`/`setup`, running `LocalExec` for real, pointing anything at a real host.
Next: port `proxmox.js` (see MAP.md "Open"). Read DECISIONS.md ADR-003/005/006 first.
