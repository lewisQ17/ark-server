# Glossary

| Term (original) | Meaning | Rust |
| --- | --- | --- |
| pmx / `Proxmox` / `LocalExec` | the executor the backend talks to | `exec::Exec`, `exec::LocalExec`, `fake::FakeVm` |
| `guestShell` / `guestExec` | run a shell script / an argv on the game machine | `Exec::guest_shell`, `Exec::guest_exec` |
| map, world, instance | one game server (map dir + systemd service `ark-<lowercase>`) | `ark::ArkInstance`, `ark::DiscoveredMap` |
| display / internal name | `TheIsland` / `TheIsland_WP` | `DiscoveredMap::display`, `::internal` |
| `map.conf` | per-map KEY=VALUE settings (ports, passwords, start params) | `ark-ctl::conf`, `ark::parse_conf` (`util`) |
| GUS | `GameUserSettings.ini` | `ark::read_config("gus")` |
| `cache` | what the poller knows (host, instances, hist, feed) | `server::Cache` |
| audit | in-memory log of admin actions, max 60, newest first | `AppState::log_audit` |
| leak guard | save + restart an empty world whose RAM passes a ceiling | `AppState::leak_guard_check` |
| `vmBudget` | cores/RAM of the VM, ceiling for per-world caps | `AppState::vm_budget` |
| front door | generated Rust server that proxies to the original | `rust/app` |
| fake VM | in-memory stand-in for the machine | `rust/harness/fake-vm.js`, `rust/dash/src/fake.rs` |
