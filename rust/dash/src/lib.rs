//! Rust port of the ARK dashboard backend (`dashboard/backend/*.js`).
//!
//! * [`util`]   JavaScript-semantics helpers (coercions, number printing, secret redaction, parsers)
//! * [`config`] environment and defaults
//! * [`exec`]   how commands reach the game server: the [`exec::Exec`] trait and the local implementation
//! * [`fake`]   an in-memory VM implementing the same trait, so nothing is executed in tests and replays
//! * [`ark`]    per-map operations on top of an `Exec`
//! * [`server`] state, poller, leak guard and the HTTP routes

pub mod ark;
pub mod config;
pub mod exec;
pub mod fake;
pub mod pinned;
pub mod proxmox;
pub mod security;
pub mod unifi;
pub mod server;
pub mod util;
