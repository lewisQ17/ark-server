#!/bin/sh
# Plays the recording of the original Node server (167 HTTP cases, made with rust-anything in proxmox mode against
# local HTTPS stubs of the Proxmox API and the UniFi controller) against the Rust server, same stubs.
# Usage (from the repo):  sh dashboard/server/tests/replay.sh      needs node, openssl and rust-anything (RA=<path>).
# Nothing leaves 127.0.0.1; the stubs answer from an in-memory fake VM with dummy credentials.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RA=${RA:-$HOME/Wall-LE/code/_extern/ra-bin/bin/rust-anything}
BIN=$ROOT/server/target/release/ark-dashboard
[ -x "$BIN" ] || (cd "$ROOT/server" && cargo build --release --locked)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/rust"
cp -R "$ROOT/server/tests/replay/app-cases" "$T/rust/app-cases"
export ARK_FRONTEND_DIR="$ROOT/frontend"
"$RA" app replay "$T" --build "node $ROOT/server/tests/replay/run-rust-pve.js $BIN"
