#!/bin/sh
# Start the Rust dashboard against the in-memory fake VM (nothing is executed), with the same
# dummy settings the recording harness uses. Used by `rust-anything app replay --build`.
here=$(cd "$(dirname "$0")" && pwd)
bin="${ARK_DASHBOARD_BIN:-$here/target/release/ark-dashboard}"
[ -x "$bin" ] || bin="$here/target/debug/ark-dashboard"
ARK_EXEC_MODE=fake DASH_USER=admin DASH_PASS=dummy-dash-pass BIND=127.0.0.1 POLL_INTERVAL_MS=600000 exec "$bin" "$@"
