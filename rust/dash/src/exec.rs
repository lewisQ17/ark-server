//! How commands reach the game server. One trait, two implementations:
//! [`LocalExec`] runs the manager on this machine (sudo -u, like `local.js`), and
//! [`crate::fake::FakeVm`] answers from memory so nothing is ever executed.
//! [`crate::proxmox::Proxmox`] reaches the VM through the PVE API (guest-exec over the guest agent).

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
pub struct ExecOut {
    pub exitcode: i32,
    pub out: String,
    pub err: String,
}

#[derive(Debug, Clone)]
pub struct ShellOpts {
    pub as_user: Option<String>,
    pub input: Option<String>,
    pub timeout_ms: u64,
}

impl Default for ShellOpts {
    fn default() -> Self {
        Self {
            as_user: None,
            input: None,
            timeout_ms: 20_000,
        }
    }
}

impl ShellOpts {
    pub fn user(u: &str) -> Self {
        Self {
            as_user: Some(u.to_string()),
            ..Self::default()
        }
    }
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
    pub fn input(mut self, s: impl Into<String>) -> Self {
        self.input = Some(s.into());
        self
    }
}

#[async_trait]
pub trait Exec: Send + Sync {
    /// `Err` is a transport failure (the JS original throws): the host cannot be reached, the guest agent is down, ...
    async fn guest_shell(&self, script: &str, opts: ShellOpts) -> Result<ExecOut, String>;
    async fn guest_exec(&self, args: &[&str], opts: ShellOpts) -> Result<ExecOut, String>;
    /// Host status in the shape of Proxmox' `status/current`.
    async fn vm_status(&self) -> Result<Value, String>;
    async fn rrddata(&self) -> Result<Vec<Value>, String>;
    async fn vm_power(&self, action: &str) -> Result<(), String>;
}

/// argv for running `args` as `user`, mirroring `_spawnAs` of local.js. `uid_is_root` is the
/// current process' uid check. Pure, so it is unit tested without spawning anything.
pub fn spawn_argv(user: Option<&str>, args: &[String], uid_is_root: bool) -> Vec<String> {
    let mut out = Vec::new();
    match user {
        Some(u) if uid_is_root && u != "root" => {
            out.extend(["sudo", "-u", u, "-H", "--"].map(String::from));
        }
        Some(u) if !uid_is_root => {
            out.extend(["sudo", "-n", "-u", u, "-H", "--"].map(String::from));
        }
        _ => {}
    }
    out.extend(args.iter().cloned());
    out
}

/// The `runuser -u <user> -- cmd...` wrapper the Proxmox path uses becomes a plain sudo -u here.
pub fn unwrap_runuser(args: &[String]) -> Option<(String, Vec<String>)> {
    if args.first().map(String::as_str) == Some("/usr/sbin/runuser")
        && args.get(1).map(String::as_str) == Some("-u")
    {
        let user = args.get(2)?.clone();
        let dd = args.iter().position(|a| a == "--")?;
        return Some((user, args[dd + 1..].to_vec()));
    }
    None
}

pub struct LocalExec {
    run_as_user: String,
    allow_host_power: bool,
    queue: tokio::sync::Mutex<()>,
    hist: Mutex<Vec<Value>>,
    last_cpu: Mutex<Option<(f64, f64)>>,
}

impl LocalExec {
    pub fn new(run_as_user: &str, allow_host_power: bool) -> Self {
        Self {
            run_as_user: run_as_user.to_string(),
            allow_host_power,
            queue: tokio::sync::Mutex::new(()),
            hist: Mutex::new(Vec::new()),
            last_cpu: Mutex::new(None),
        }
    }

    async fn spawn_as(&self, user: Option<&str>, args: &[String], opts: &ShellOpts) -> ExecOut {
        use tokio::io::AsyncWriteExt;
        let root = std::env::var("USER").is_ok_and(|u| u == "root");
        let argv = spawn_argv(user, args, root);
        let Some((prog, rest)) = argv.split_first() else {
            return ExecOut {
                exitcode: 1,
                out: String::new(),
                err: "empty command".into(),
            };
        };
        let mut cmd = tokio::process::Command::new(prog);
        cmd.args(rest)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ExecOut {
                    exitcode: 1,
                    out: String::new(),
                    err: e.to_string(),
                };
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let data = opts.input.clone().unwrap_or_default();
            // The child may exit before we finish writing; ignore a broken pipe.
            let _ = stdin.write_all(data.as_bytes()).await;
            drop(stdin);
        }
        let wait = child.wait_with_output();
        match tokio::time::timeout(std::time::Duration::from_millis(opts.timeout_ms), wait).await {
            Ok(Ok(o)) => ExecOut {
                exitcode: o.status.code().unwrap_or(1),
                out: String::from_utf8_lossy(&o.stdout).into_owned(),
                err: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Ok(Err(e)) => ExecOut {
                exitcode: 1,
                out: String::new(),
                err: e.to_string(),
            },
            Err(_) => ExecOut {
                exitcode: 1,
                out: String::new(),
                err: "command timed out".into(),
            },
        }
    }

    fn cpu_pct(&self) -> f64 {
        let Ok(text) = std::fs::read_to_string("/proc/stat") else {
            return 0.0;
        };
        let v: Vec<f64> = text
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse().ok())
            .collect();
        if v.len() < 5 {
            return 0.0;
        }
        let idle = v[3] + v[4];
        let total: f64 = v.iter().sum();
        let mut last = self.last_cpu.lock().expect("cpu lock");
        let prev = last.replace((idle, total));
        match prev {
            None => 0.0,
            Some((pi, pt)) => {
                let dt = total - pt;
                let di = idle - pi;
                if dt > 0.0 {
                    (1.0 - di / dt).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
        }
    }

    fn mem() -> (f64, f64) {
        let Ok(t) = std::fs::read_to_string("/proc/meminfo") else {
            return (0.0, 0.0);
        };
        let kb = |k: &str| -> f64 {
            t.lines()
                .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(':')))
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse::<f64>().ok())
                .map_or(0.0, |n| n * 1024.0)
        };
        let total = kb("MemTotal");
        let avail = if kb("MemAvailable") > 0.0 {
            kb("MemAvailable")
        } else {
            kb("MemFree") + kb("Cached")
        };
        ((total - avail).max(0.0), total)
    }

    fn net() -> (f64, f64) {
        let Ok(t) = std::fs::read_to_string("/proc/net/dev") else {
            return (0.0, 0.0);
        };
        let (mut rx, mut tx) = (0.0, 0.0);
        for l in t.lines().skip(2) {
            let Some((iface, rest)) = l.split_once(':') else {
                continue;
            };
            if iface.trim() == "lo" {
                continue;
            }
            let f: Vec<f64> = rest
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            rx += f.first().copied().unwrap_or(0.0);
            tx += f.get(8).copied().unwrap_or(0.0);
        }
        (rx, tx)
    }
}

#[async_trait]
impl Exec for LocalExec {
    async fn guest_shell(&self, script: &str, opts: ShellOpts) -> Result<ExecOut, String> {
        let _g = self.queue.lock().await;
        let user = opts
            .as_user
            .clone()
            .unwrap_or_else(|| self.run_as_user.clone());
        let args = ["/bin/bash", "-lc", script].map(String::from);
        Ok(self.spawn_as(Some(&user), &args, &opts).await)
    }

    async fn guest_exec(&self, args: &[&str], opts: ShellOpts) -> Result<ExecOut, String> {
        let _g = self.queue.lock().await;
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        if let Some((user, real)) = unwrap_runuser(&owned) {
            return Ok(self.spawn_as(Some(&user), &real, &opts).await);
        }
        let user = self.run_as_user.clone();
        Ok(self.spawn_as(Some(&user), &owned, &opts).await)
    }

    async fn vm_status(&self) -> Result<Value, String> {
        let cpu = self.cpu_pct();
        let (used, total) = Self::mem();
        let (netin, netout) = Self::net();
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|x| x.parse::<f64>().ok())
            })
            .unwrap_or(0.0)
            .floor();
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        let name = std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".into());
        let now = (crate::util::now_ms() / 1000.0).floor();
        let mut h = self.hist.lock().expect("hist lock");
        h.push(json!({"cpu": cpu, "mem": used, "maxmem": total, "netin": netin, "netout": netout, "time": now}));
        if h.len() > 44 {
            h.remove(0);
        }
        Ok(json!({
            "name": name, "status": "running", "cpus": cpus, "cpu": cpu, "mem": used, "maxmem": total,
            "disk": 0, "maxdisk": 0, "uptime": uptime
        }))
    }

    async fn rrddata(&self) -> Result<Vec<Value>, String> {
        Ok(self.hist.lock().expect("hist lock").clone())
    }

    async fn vm_power(&self, action: &str) -> Result<(), String> {
        if !self.allow_host_power {
            return Err(format!(
                "Refusing to {action} the machine: in local mode \"the VM\" is this server itself. Set ALLOW_HOST_POWER=1 in the dashboard env if you really want the panel to be able to do that."
            ));
        }
        let verb = match action {
            "reboot" => "reboot",
            "shutdown" => "poweroff",
            _ => {
                return Err(format!(
                    "\"{action}\" is not available in local mode \u{2014} the host is always running."
                ));
            }
        };
        let args = ["systemctl", verb].map(String::from);
        let o = self
            .spawn_as(Some("root"), &args, &ShellOpts::default().timeout(10_000))
            .await;
        if o.exitcode == 0 { Ok(()) } else { Err(o.err) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn root_steps_down_with_sudo_u() {
        assert_eq!(
            spawn_argv(Some("arkadmin"), &s(&["ls"]), true),
            s(&["sudo", "-u", "arkadmin", "-H", "--", "ls"])
        );
        assert_eq!(spawn_argv(Some("root"), &s(&["ls"]), true), s(&["ls"]));
    }

    #[test]
    fn non_root_uses_sudo_n() {
        assert_eq!(
            spawn_argv(Some("arkadmin"), &s(&["ls"]), false),
            s(&["sudo", "-n", "-u", "arkadmin", "-H", "--", "ls"])
        );
        assert_eq!(spawn_argv(None, &s(&["ls"]), false), s(&["ls"]));
    }

    #[test]
    fn runuser_wrapper_is_unwrapped() {
        let a = s(&["/usr/sbin/runuser", "-u", "bob", "--", "id", "-u"]);
        assert_eq!(unwrap_runuser(&a), Some(("bob".into(), s(&["id", "-u"]))));
        assert_eq!(unwrap_runuser(&s(&["id"])), None);
    }
}
