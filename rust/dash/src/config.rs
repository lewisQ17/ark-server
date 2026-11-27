//! Environment and defaults (`config.js`). Read once at start; `from_env` takes a lookup so tests need no process env.

use crate::util::{parse_float, parse_int};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The game server runs on this machine; commands go through `sudo -u`.
    Local,
    /// In-memory fake VM. For recording, replay and tests only.
    Fake,
    /// Proxmox guest agent. Not ported to Rust.
    Proxmox,
}

#[derive(Debug, Clone)]
pub struct Auth {
    pub enabled: bool,
    pub user: String,
    pub pass: String,
}

#[derive(Debug, Clone)]
pub struct LeakGuard {
    pub enabled: bool,
    pub ram_gb: f64,
    pub min_uptime_h: f64,
    pub cooldown_h: f64,
}

#[derive(Debug, Clone)]
pub struct ArkCfg {
    pub user: String,
    pub manager: String,
    pub service_prefix: String,
    pub ram_alloc_gb: i64,
    pub cores_total: i64,
    pub rcon_password_override: String,
}

impl ArkCfg {
    /// Directory of the manager script (`managerDir`).
    pub fn dir(&self) -> String {
        match self.manager.rfind('/') {
            Some(i) if i + 1 < self.manager.len() => self.manager[..i].to_string(),
            _ => self.manager.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub bind: String,
    pub mode: Mode,
    pub auth: Auth,
    pub run_as_user: String,
    pub allow_host_power: bool,
    pub pve_vmid: i64,
    pub leak_guard: LeakGuard,
    pub ark: ArkCfg,
    pub unifi_enabled: bool,
    pub poll_interval_ms: u64,
    pub discover_every_n: u64,
    pub hist_len: usize,
    pub frontend_dir: String,
}

pub const OFFICIAL_MAPS: [(&str, &str); 12] = [
    ("TheIsland", "TheIsland_WP"),
    ("ScorchedEarth", "ScorchedEarth_WP"),
    ("Aberration", "Aberration_WP"),
    ("Extinction", "Extinction_WP"),
    ("TheCenter", "TheCenter_WP"),
    ("Ragnarok", "Ragnarok_WP"),
    ("Valguero", "Valguero_WP"),
    ("CrystalIsles", "CrystalIsles_WP"),
    ("LostIsland", "LostIsland_WP"),
    ("Fjordur", "Fjordur_WP"),
    ("GenesisPart1", "Genesis_WP"),
    ("GenesisPart2", "Gen2_WP"),
];

impl Config {
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Config, String> {
        let s = |k: &str| get(k).filter(|v| !v.is_empty());
        let or = |k: &str, d: &str| s(k).unwrap_or_else(|| d.to_string());
        let int = |k: &str, d: i64| s(k).and_then(|v| parse_int(&v)).unwrap_or(d);
        let flt = |k: &str, d: f64| {
            let v = s(k).map_or(d, |v| parse_float(&v));
            if v.is_nan() { d } else { v }
        };
        let mode_name = s("ARK_EXEC_MODE")
            .unwrap_or_else(|| {
                if s("PVE_PASSWORD").is_some() {
                    "proxmox".into()
                } else {
                    "local".into()
                }
            })
            .to_lowercase();
        let mode = match mode_name.as_str() {
            "proxmox" => Mode::Proxmox,
            "fake" => Mode::Fake,
            _ => Mode::Local,
        };
        if mode == Mode::Proxmox {
            return Err("ARK_EXEC_MODE=proxmox is not ported to Rust yet (proxmox.js); use the Node backend for it, or ARK_EXEC_MODE=local on the game machine.".into());
        }
        let unifi_enabled =
            s("UNIFI_API_KEY").is_some() && s("UNIFI_HOST").is_some() && s("ARK_VM_IP").is_some();
        if unifi_enabled {
            return Err("UniFi port-forward management (unifi.js) is not ported to Rust yet; unset UNIFI_API_KEY/UNIFI_HOST/ARK_VM_IP.".into());
        }
        let user = or("ARK_USER", "arkadmin");
        Ok(Config {
            port: u16::try_from(int("PORT", 8787)).map_err(|_| "PORT out of range".to_string())?,
            bind: or("BIND", "127.0.0.1"),
            mode,
            auth: Auth {
                enabled: get("DASH_AUTH_DISABLED").as_deref() != Some("1"),
                user: or("DASH_USER", "admin"),
                pass: get("DASH_PASS").unwrap_or_default(),
            },
            run_as_user: user.clone(),
            allow_host_power: get("ALLOW_HOST_POWER").as_deref() == Some("1"),
            pve_vmid: int("PVE_VMID", 0),
            leak_guard: LeakGuard {
                enabled: get("ARK_LEAK_GUARD").as_deref() != Some("0"),
                ram_gb: flt("ARK_LEAK_GUARD_GB", 12.0),
                min_uptime_h: flt("ARK_LEAK_GUARD_MIN_UPTIME_H", 12.0),
                cooldown_h: flt("ARK_LEAK_GUARD_COOLDOWN_H", 6.0),
            },
            ark: ArkCfg {
                user,
                manager: or("ARK_MANAGER", "/home/arkadmin/ark-manager/ark-manager.sh"),
                service_prefix: "ark-".into(),
                ram_alloc_gb: int("ARK_RAM_ALLOC", 16),
                cores_total: int("PVE_VM_CORES", 4),
                rcon_password_override: get("ARK_RCON_PASSWORD").unwrap_or_default(),
            },
            unifi_enabled,
            poll_interval_ms: u64::try_from(int("POLL_INTERVAL_MS", 6000))
                .unwrap_or(6000)
                .max(1),
            discover_every_n: 5,
            hist_len: 44,
            frontend_dir: s("ARK_FRONTEND_DIR").unwrap_or_else(|| {
                format!("{}/../../dashboard/frontend", env!("CARGO_MANIFEST_DIR"))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        Config::from_env(&|k| m.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = cfg(&[]).expect("defaults");
        assert_eq!(
            (c.port, c.bind.as_str(), c.mode),
            (8787, "127.0.0.1", Mode::Local)
        );
        assert_eq!(c.ark.dir(), "/home/arkadmin/ark-manager");
        assert!(c.auth.enabled && c.auth.pass.is_empty());
        assert_eq!(c.leak_guard.ram_gb, 12.0);
    }

    #[test]
    fn proxmox_and_unifi_are_refused_not_ignored() {
        assert!(cfg(&[("PVE_PASSWORD", "x")]).is_err());
        assert!(
            cfg(&[
                ("UNIFI_API_KEY", "k"),
                ("UNIFI_HOST", "h"),
                ("ARK_VM_IP", "i")
            ])
            .is_err()
        );
    }

    #[test]
    fn leak_guard_can_be_switched_off() {
        assert!(
            !cfg(&[("ARK_LEAK_GUARD", "0")])
                .expect("cfg")
                .leak_guard
                .enabled
        );
    }
}
