//! Environment and defaults (`config.js`). Read once at start; `from_env` takes a lookup so tests need no process env.

use crate::util::{parse_float, parse_int};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The game server runs on this machine; commands go through `sudo -u`.
    Local,
    /// In-memory fake VM. For recording, replay and tests only.
    Fake,
    /// Proxmox guest agent over the PVE API (`proxmox.rs`).
    Proxmox,
}

/// Proxmox connection (`config.proxmox`). Host, node and vmid are mandatory; the certificate pin too
/// unless `tls_insecure` is set.
#[derive(Debug, Clone, Default)]
pub struct Pve {
    pub host: String,
    pub port: u16,
    pub node: String,
    pub vmid: i64,
    pub user: String,
    pub password: String,
    pub tls_fingerprint: String,
    pub tls_insecure: bool,
}

/// UniFi/UDM WAN port-forward management (`config.unifi`).
#[derive(Debug, Clone, Default)]
pub struct UnifiCfg {
    pub enabled: bool,
    pub host: String,
    pub api_key: String,
    pub site: String,
    pub ark_vm_ip: String,
    pub proto: String,
    pub tls_fingerprint: String,
    pub tls_insecure: bool,
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
    pub pve: Pve,
    pub unifi: UnifiCfg,
    /// `TRUSTED_PROXIES`: peers whose X-Forwarded-For / CF-Connecting-IP is believed.
    pub trusted_proxies: Vec<String>,
    /// `ALLOWED_ORIGINS`: extra Origins allowed on POST/PUT/PATCH/DELETE.
    pub allowed_origins: Vec<String>,
    pub leak_guard: LeakGuard,
    pub ark: ArkCfg,
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
        let pve = Pve {
            host: get("PVE_HOST").unwrap_or_default(),
            port: u16::try_from(int("PVE_PORT", 8006)).map_err(|_| "PVE_PORT out of range".to_string())?,
            node: get("PVE_NODE").unwrap_or_default(),
            vmid: int("PVE_VMID", 0),
            user: or("PVE_USER", "root@pam"),
            password: get("PVE_PASSWORD").unwrap_or_default(),
            tls_fingerprint: get("PVE_TLS_FINGERPRINT").unwrap_or_default(),
            tls_insecure: get("PVE_TLS_INSECURE").as_deref() == Some("1"),
        };
        if mode == Mode::Proxmox {
            let missing: Vec<&str> = [
                ("PVE_HOST", s("PVE_HOST").is_none()),
                ("PVE_NODE", s("PVE_NODE").is_none()),
                ("PVE_VMID", s("PVE_VMID").is_none_or(|v| v == "0")),
            ]
            .iter()
            .filter(|(_, m)| *m)
            .map(|(k, _)| *k)
            .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "ARK_EXEC_MODE=proxmox needs {} in the environment. Copy .env.example to .env and fill in your own Proxmox host/node/VM id.",
                    missing.join(", ")
                ));
            }
        }
        let unifi = UnifiCfg {
            enabled: s("UNIFI_API_KEY").is_some()
                && s("UNIFI_HOST").is_some()
                && s("ARK_VM_IP").is_some(),
            host: get("UNIFI_HOST").unwrap_or_default(),
            api_key: get("UNIFI_API_KEY").unwrap_or_default(),
            site: or("UNIFI_SITE", "default"),
            ark_vm_ip: get("ARK_VM_IP").unwrap_or_default(),
            proto: or("UNIFI_FORWARD_PROTO", "udp"),
            tls_fingerprint: get("UNIFI_TLS_FINGERPRINT").unwrap_or_default(),
            tls_insecure: get("UNIFI_TLS_INSECURE").as_deref() == Some("1"),
        };
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
            pve,
            unifi,
            trusted_proxies: crate::security::parse_list(&get("TRUSTED_PROXIES").unwrap_or_default()),
            allowed_origins: crate::security::parse_list(&get("ALLOWED_ORIGINS").unwrap_or_default()),
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
            poll_interval_ms: u64::try_from(int("POLL_INTERVAL_MS", 6000))
                .unwrap_or(6000)
                .max(1),
            discover_every_n: 5,
            hist_len: 44,
            frontend_dir: s("ARK_FRONTEND_DIR").unwrap_or_else(|| {
                format!("{}/../frontend", env!("CARGO_MANIFEST_DIR"))
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
    fn proxmox_mode_needs_host_node_and_vmid() {
        let e = cfg(&[("ARK_EXEC_MODE", "proxmox")]).expect_err("incomplete");
        assert!(e.contains("PVE_HOST, PVE_NODE, PVE_VMID"), "{e}");
        let c = cfg(&[
            ("PVE_PASSWORD", "x"),
            ("PVE_HOST", "h"),
            ("PVE_NODE", "n"),
            ("PVE_VMID", "100"),
        ])
        .expect("auto-detected proxmox");
        assert_eq!((c.mode, c.pve.vmid, c.pve.port), (Mode::Proxmox, 100, 8006));
    }

    #[test]
    fn unifi_needs_all_three() {
        assert!(!cfg(&[("UNIFI_API_KEY", "k"), ("UNIFI_HOST", "h")]).expect("cfg").unifi.enabled);
        let c = cfg(&[("UNIFI_API_KEY", "k"), ("UNIFI_HOST", "h"), ("ARK_VM_IP", "i")]).expect("cfg");
        assert!(c.unifi.enabled);
        assert_eq!((c.unifi.site.as_str(), c.unifi.proto.as_str()), ("default", "udp"));
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
