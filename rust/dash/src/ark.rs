//! ARK: Survival Ascended operations, driven through an [`Exec`] (`ark.js`).
//! Maps are discovered from `maps/*/map.conf`; nothing is hardcoded. Every shell script here is
//! byte-for-byte what the Node backend sends, because the fake VM and the real manager parse them.

use std::sync::{Arc, Mutex};

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::config::ArkCfg;
use crate::exec::{Exec, ShellOpts};
use crate::util::{
    ascii_safe, conf_get, js_number, js_round, js_str, js_trim, match_kv_greedy, now_ms, num,
    num_str, parse_conf, parse_float, parse_int, redact_secrets, shq, slice_units, truthy,
};

/// Inline Source-RCON client (password over stdin, never in argv).
pub const RCON_PY: &str = r#"
import sys, socket, struct
data = sys.stdin.read().split("\n")
pw = data[0]
cmd = "\n".join(data[1:]).strip("\n")
HOST, PORT = "127.0.0.1", int(sys.argv[1])
def pkt(pid, ptype, body):
    payload = struct.pack("<ii", pid, ptype) + body.encode("utf-8") + b"\x00\x00"
    return struct.pack("<i", len(payload)) + payload
def recv(sock):
    raw = b""
    while len(raw) < 4:
        c = sock.recv(4 - len(raw))
        if not c: raise IOError("closed")
        raw += c
    ln = struct.unpack("<i", raw)[0]
    buf = b""
    while len(buf) < ln:
        c = sock.recv(ln - len(buf))
        if not c: raise IOError("closed")
        buf += c
    pid, ptype = struct.unpack("<ii", buf[:8])
    return pid, ptype, buf[8:-2].decode("utf-8", "replace")
try:
    s = socket.create_connection((HOST, PORT), timeout=8)
    s.settimeout(8)
    s.sendall(pkt(1, 3, pw))
    rid, rtype, _ = recv(s)
    if rtype == 0:
        rid, rtype, _ = recv(s)
    if rid == -1 or rtype != 2:
        print("__RCON_AUTH_FAIL__"); sys.exit(2)
    s.sendall(pkt(7, 2, cmd))
    out = []
    _, _, resp = recv(s); out.append(resp)
    s.settimeout(1.0)
    try:
        while True:
            _, _, more = recv(s)
            if more: out.append(more)
    except Exception:
        pass
    sys.stdout.write("".join(out))
    s.close()
except Exception as e:
    sys.stderr.write("__RCON_ERR__ " + str(e)); sys.exit(3)
"#;

pub const PLATFORMS: [&str; 4] = ["PC", "XSX", "PS5", "WINGDK"];

const PY_SET_KEY: &str = r#"import sys,re,os
path,key,val,section=sys.argv[1:5]
lines=open(path,encoding='utf-8',errors='replace').read().split('\n') if os.path.exists(path) else []
done=False
for i,l in enumerate(lines):
    if re.match(r'^\s*'+re.escape(key)+r'\s*=', l):
        lines[i]=key+'='+val; done=True; break
if not done:
    try: idx=next(i for i,l in enumerate(lines) if l.strip().lower()==section.lower())
    except StopIteration:
        lines=[section]+lines; idx=0
    lines.insert(idx+1, key+'='+val)
open(path,'w',encoding='utf-8').write('\n'.join(lines))
print('OK')
"#;

const PY_SET_PLATFORMS: &str = r#"import sys,os
path=sys.argv[1]; arg=sys.argv[2]
lines=open(path,encoding='utf-8',errors='replace').read().split('\n') if os.path.exists(path) else []
done=False
for i,l in enumerate(lines):
    if l.startswith('CustomStartParams='):
        toks=[t for t in l.split('=',1)[1].split() if t and t!='-crossplay' and not t.startswith('-ServerPlatform=')]
        toks.append('-ServerPlatform='+arg)
        lines[i]='CustomStartParams='+(' '.join(toks)); done=True; break
if not done:
    lines.append('CustomStartParams=-ServerPlatform='+arg)
open(path,'w',encoding='utf-8').write('\n'.join(lines))
print('OK')
"#;

const PY_SET_MAPCONF: &str = r#"import sys,os,re
path,key,val=sys.argv[1:4]
lines=open(path,encoding='utf-8',errors='replace').read().split('\n') if os.path.exists(path) else []
done=False
for i,l in enumerate(lines):
    if re.match(r'^\s*'+re.escape(key)+r'\s*=', l):
        lines[i]=key+'='+val; done=True; break
if not done: lines.append(key+'='+val)
open(path,'w',encoding='utf-8').write('\n'.join(lines))
print('OK')
"#;

const BACKUP_HELPER: &str = r#"#!/bin/bash
# usage: dashboard-backup.sh <MAPDIR> <SERVERDIR> <DISPLAY> [TAG]
MD="$1"; SERVER="$2"; DISP="$3"; TAG="$4"
SAVED="$SERVER/ShooterGame/Saved"
BK="$MD/backups"; mkdir -p "$BK"
SR=$(find "$SAVED" -maxdepth 3 -name '*.ark' -printf '%P\n' 2>/dev/null | head -1 | cut -d/ -f1)
[ -z "$SR" ] && SR=SavedArks
TS=$(date +%Y%m%d_%H%M%S)
F="$BK/${DISP}${TAG:+_$TAG}_$TS.tar.gz"
tar -czf "$F" -C "$SAVED" --exclude='*_[0-9][0-9].[0-9][0-9].[0-9][0-9][0-9][0-9]_*.ark' "$SR" 2>&1
echo "OK saved=$SR size=$(du -h "$F" 2>/dev/null | cut -f1)"
"#;

const RATE_HELPER: &str = r#"#!/bin/bash
# usage: dashboard-set-rate.sh <KEY> <VALUE> <gus|game> <MAPDIR>
KEY="$1"; VAL="$2"; F="$3"; MD="$4"
if [ "$F" = "game" ]; then P="$MD/Game.ini"; SEC="[/script/shootergame.shootergamemode]"; else P="$MD/GameUserSettings.ini"; SEC="[ServerSettings]"; fi
python3 - "$P" "$KEY" "$VAL" "$SEC" <<'PYEOF'
import sys,re,os
path,key,val,section=sys.argv[1:5]
lines=open(path,encoding='utf-8',errors='replace').read().split('\n') if os.path.exists(path) else []
done=False
for i,l in enumerate(lines):
    if re.match(r'^\s*'+re.escape(key)+r'\s*=', l):
        lines[i]=key+'='+val; done=True; break
if not done:
    try: idx=next(i for i,l in enumerate(lines) if l.strip().lower()==section.lower())
    except StopIteration:
        lines=[section]+lines; idx=0
    lines.insert(idx+1, key+'='+val)
open(path,'w',encoding='utf-8').write('\n'.join(lines))
print('OK')
PYEOF
"#;

/// A map as discovered from its `map.conf`.
#[derive(Debug, Clone)]
pub struct DiscoveredMap {
    pub display: String,
    pub internal: String,
    pub save_dir: String,
    pub game_port: Option<String>,
    pub query_port: Option<String>,
    pub rcon_port: Option<String>,
    pub max_players: Option<String>,
}

impl DiscoveredMap {
    pub fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("display".into(), self.display.clone().into());
        m.insert("internal".into(), self.internal.clone().into());
        m.insert("saveDir".into(), self.save_dir.clone().into());
        for (k, v) in [
            ("gamePort", &self.game_port),
            ("queryPort", &self.query_port),
            ("rconPort", &self.rcon_port),
            ("maxPlayers", &self.max_players),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v.clone().into());
            }
        }
        m
    }
}

#[derive(Default)]
struct Caches {
    paths: Option<Paths>,
    pw: Option<String>,
    cfg: Option<(Value, f64)>,
    bk_helper: bool,
    rate_helper: bool,
}

#[derive(Clone)]
struct Paths {
    log: String,
    gus: String,
    game: String,
    backups: String,
}

pub struct Status {
    pub state: String,
    pub dash: String,
    pub start_ts: String,
}

pub struct ArkInstance {
    exec: Arc<dyn Exec>,
    cfg: ArkCfg,
    pub user: String,
    dir: String,
    pub id: String,
    pub display: String,
    pub name: String,
    pub map: String,
    pub service: String,
    pub rcon_port: i64,
    pub game_port: Option<String>,
    pub query_port: Option<String>,
    pub max_players: i64,
    pub ram_alloc: i64,
    caches: Mutex<Caches>,
}

fn is_json_ok(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Object(o) => truthy(o.get("ok")),
        _ => false,
    }
}

fn re_service() -> &'static Regex {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| Regex::new(r"^ark-[a-z0-9_.-]+$").expect("re"))
}

pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_snapshot(name: &str) -> bool {
    // /^[\w.\- ]+\.tar\.gz$/
    match name.strip_suffix(".tar.gz") {
        Some(stem) => {
            !stem.is_empty()
                && stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' '))
        }
        None => false,
    }
}

fn num_re_ok(s: &str) -> bool {
    // /^-?\d+(\.\d+)?$/
    let t = s.strip_prefix('-').unwrap_or(s);
    let (a, b) = match t.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (t, None),
    };
    let digits = |x: &str| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit());
    digits(a) && b.is_none_or(digits)
}

fn ok_cron(c: &str) -> bool {
    // /^[\d*/,\- ]+$/ on the trimmed value, and exactly 5 whitespace-separated fields
    let t = js_trim(c);
    !t.is_empty()
        && t.chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '*' | '/' | ',' | '-' | ' '))
        && t.split_whitespace().count() == 5
}

impl ArkInstance {
    pub fn new(exec: Arc<dyn Exec>, cfg: &ArkCfg, m: &DiscoveredMap) -> Result<Self, String> {
        let id = m.display.to_lowercase();
        let service = format!("{}{}", cfg.service_prefix, m.display.to_lowercase());
        if service.is_empty()
            || !service
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return Err(format!(
                "unsafe service name refused: {}",
                Value::String(service)
            ));
        }
        let pick = |o: &Option<String>, d: i64| -> i64 {
            match o {
                Some(s) if !s.is_empty() => parse_int(s).unwrap_or(0),
                _ => d,
            }
        };
        Ok(Self {
            exec,
            cfg: cfg.clone(),
            user: cfg.user.clone(),
            dir: cfg.dir(),
            id,
            display: m.display.clone(),
            name: m.display.clone(),
            map: if m.internal.is_empty() {
                m.display.clone()
            } else {
                m.internal.clone()
            },
            service,
            rcon_port: pick(&m.rcon_port, 27020),
            game_port: m.game_port.clone(),
            query_port: m.query_port.clone(),
            max_players: pick(&m.max_players, 70),
            ram_alloc: cfg.ram_alloc_gb,
            caches: Mutex::new(Caches::default()),
        })
    }

    pub fn map_conf_path(&self) -> String {
        format!("{}/maps/{}/map.conf", self.dir, self.display)
    }
    pub fn map_dir(&self) -> String {
        format!("{}/maps/{}", self.dir, self.display)
    }

    async fn rcon_pw(&self) -> Result<String, String> {
        if !self.cfg.rcon_password_override.is_empty() {
            return Ok(self.cfg.rcon_password_override.clone());
        }
        if let Some(p) = self.caches.lock().expect("lock").pw.clone() {
            return Ok(p);
        }
        let def_conf = format!("{}/config/server-defaults.conf", self.dir);
        let script = format!(
            "V=$(grep -E '^AdminPassword=' {} 2>/dev/null | tail -1 | cut -d= -f2-); [ -z \"$V\" ] && V=$(grep -E '^DefaultAdminPassword=' {} 2>/dev/null | tail -1 | cut -d= -f2-); printf 'PW<%s>PW' \"$V\"",
            shq(&self.map_conf_path()),
            shq(&def_conf)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(15_000))
            .await?;
        let pw = match (r.out.find("PW<"), r.out.rfind(">PW")) {
            (Some(s), Some(e)) if e >= s + 3 => {
                r.out[s + 3..e].trim_end_matches(['\r', '\n']).to_string()
            }
            _ => String::new(),
        };
        self.caches.lock().expect("lock").pw = Some(pw.clone());
        Ok(pw)
    }

    /// Run one RCON command. Re-reads the admin password once when auth fails (it may have been rotated).
    pub async fn rcon(&self, command: &str) -> Result<Value, String> {
        let mut retried = false;
        loop {
            let pw = self.rcon_pw().await?;
            if pw.is_empty() {
                return Ok(json!({"ok": false, "error": "admin password not found"}));
            }
            let script = format!("python3 -c {} {}", shq(RCON_PY), self.rcon_port);
            let r = self
                .exec
                .guest_shell(
                    &script,
                    ShellOpts::user(&self.user)
                        .input(format!("{pw}\n{command}"))
                        .timeout(15_000),
                )
                .await?;
            let out = js_trim(&r.out).to_string();
            let err = js_trim(&r.err).to_string();
            if out.contains("__RCON_AUTH_FAIL__") {
                if !retried && self.cfg.rcon_password_override.is_empty() {
                    self.caches.lock().expect("lock").pw = None;
                    retried = true;
                    continue;
                }
                return Ok(json!({"ok": false, "error": "RCON auth failed (bad admin password)"}));
            }
            if err.contains("__RCON_ERR__") || r.exitcode == 3 {
                let e = js_trim(&err.replacen("__RCON_ERR__", "", 1)).to_string();
                return Ok(json!({"ok": false, "error": if e.is_empty() { "RCON connection failed".to_string() } else { e }}));
            }
            return Ok(json!({"ok": true, "output": out}));
        }
    }

    pub async fn status(&self) -> Result<Status, String> {
        let script = format!(
            "systemctl is-active {s} 2>/dev/null; echo \"---\"; systemctl show {s} -p SubState,ExecMainStartTimestamp 2>/dev/null",
            s = self.service
        );
        let r = self.exec.guest_shell(&script, ShellOpts::default()).await?;
        let mut parts = r.out.split("---");
        let active = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        let state = js_trim(active).to_string();
        let props = parse_conf(rest);
        let sub = conf_get(&props, "SubState").unwrap_or("").to_string();
        let dash = match state.as_str() {
            "active" => {
                if sub == "running" {
                    "running"
                } else {
                    "starting"
                }
            }
            "activating" => "starting",
            "failed" => "crashed",
            _ => "stopped",
        };
        Ok(Status {
            state,
            dash: dash.into(),
            start_ts: conf_get(&props, "ExecMainStartTimestamp")
                .unwrap_or("")
                .to_string(),
        })
    }

    pub async fn service_action(&self, action: &str) -> Result<Value, String> {
        if !["start", "stop", "restart"].contains(&action) {
            return Err("bad action".into());
        }
        let r = self
            .exec
            .guest_shell(
                &format!("systemctl {action} {} 2>&1; echo \"exit=$?\"", self.service),
                ShellOpts::default(),
            )
            .await?;
        static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = R.get_or_init(|| Regex::new(r"exit=0\b").expect("re"));
        Ok(json!({"ok": re.is_match(&r.out), "output": js_trim(&r.out)}))
    }

    /// Current per-world caps and live RAM use (`ok`, `cores`, `ramGB`, `ramUsedGB`).
    pub async fn resources(&self) -> Result<Value, String> {
        if !re_service().is_match(&self.service) {
            return Ok(json!({"ok": false, "cores": null, "ramGB": null, "ramUsedGB": null}));
        }
        let script = format!(
            "systemctl show {} -p CPUQuotaPerSecUSec -p MemoryMax -p MemoryCurrent 2>/dev/null",
            self.service
        );
        let r = self.exec.guest_shell(&script, ShellOpts::default()).await?;
        let p = parse_conf(&r.out);
        Ok(json!({
            "ok": true,
            "cores": span_to_cores(conf_get(&p, "CPUQuotaPerSecUSec")),
            "ramGB": bytes_to_gb(conf_get(&p, "MemoryMax")),
            "ramUsedGB": bytes_to_gb(conf_get(&p, "MemoryCurrent")),
        }))
    }

    /// Apply a CPU-core cap and RAM ceiling (GB), clamped to the VM; refuses an unsafe shrink unless forced.
    pub async fn set_resources(
        &self,
        cores: Option<&Value>,
        ram_gb: Option<&Value>,
        vm_cores: f64,
        vm_ram: f64,
    ) -> Result<Value, String> {
        if !re_service().is_match(&self.service) {
            return Ok(json!({"ok": false, "error": "unsafe service name"}));
        }
        let cur = self.resources().await.unwrap_or(Value::Null);
        let g = |k: &str| cur.get(k).and_then(Value::as_f64);
        let cur_cores = g("cores").unwrap_or(vm_cores);
        let cur_ram = g("ramGB").unwrap_or(vm_ram);
        let req_c = js_number(cores);
        let req_r = js_number(ram_gb);
        let c = vm_cores.min(0.25f64.max(if req_c.is_finite() && req_c > 0.0 {
            req_c
        } else {
            cur_cores
        }));
        let r = vm_ram.min(1f64.max(js_round(if req_r.is_finite() && req_r > 0.0 {
            req_r
        } else {
            cur_ram
        })));
        if let Some(live) = g("ramUsedGB")
            && live > 0.1
            && r < live * 1.15
        {
            return Ok(json!({
                "ok": false, "needsForce": true, "liveGB": num(live), "cores": num(c), "ramGB": num(r),
                "error": format!("Requested {} GB is below this world's live use ({} GB). Stop the world first or raise the limit.", num_str(r), num_str(live))
            }));
        }
        let quota = format!("{}%", num_str(js_round(c * 100.0)));
        let high = format!("{}M", num_str(256f64.max(js_round(r * 0.9 * 1024.0))));
        let max = format!("{}G", num_str(r));
        let script = format!(
            "systemctl set-property {} CPUQuota={quota} MemoryHigh={high} MemoryMax={max} 2>&1; echo \"rc=$?\"",
            self.service
        );
        let res = self.exec.guest_shell(&script, ShellOpts::default()).await?;
        static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re_rc = R.get_or_init(|| Regex::new(r"rc=0\b").expect("re"));
        static R2: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re_tail = R2.get_or_init(|| Regex::new(r"rc=\d+\s*$").expect("re"));
        Ok(json!({
            "ok": re_rc.is_match(&res.out), "cores": num(c), "ramGB": num(r), "quota": quota, "memHigh": high, "memMax": max,
            "output": js_trim(&re_tail.replace(&res.out, "")),
        }))
    }

    pub async fn players(&self) -> Result<Value, String> {
        let r = self.rcon("ListPlayers").await?;
        if !truthy(r.get("ok")) {
            return Ok(json!({"ok": false, "error": r.get("error").cloned().unwrap_or(Value::Null), "players": []}));
        }
        let txt = r.get("output").and_then(Value::as_str).unwrap_or("");
        if txt.to_ascii_lowercase().contains("no players") {
            return Ok(json!({"ok": true, "players": []}));
        }
        static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = R.get_or_init(|| {
            Regex::new(r"^\s*([0-9]+)\.\s*(.+?),\s*([0-9a-fA-FxX]+)\s*$").expect("re")
        });
        let mut players = Vec::new();
        for line in txt.split('\n') {
            if let Some(c) = re.captures(line) {
                players.push(json!({"slot": c[1].parse::<i64>().unwrap_or(0), "name": js_trim(&c[2]), "id": &c[3]}));
            }
        }
        Ok(json!({"ok": true, "players": players}))
    }

    async fn paths(&self) -> Result<Paths, String> {
        if let Some(p) = self.caches.lock().expect("lock").paths.clone() {
            return Ok(p);
        }
        let md = self.map_dir();
        let script = format!(
            "L=$(find {}/server-files -type f -name ShooterGame.log 2>/dev/null | head -1); echo \"LOG=$L\"; echo \"GUS={md}/GameUserSettings.ini\"; echo \"GAME={md}/Game.ini\"; echo \"BKP={md}/backups\"",
            shq(&self.dir)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(20_000))
            .await?;
        let p = parse_conf(&r.out);
        let g = |k: &str| conf_get(&p, k).unwrap_or("").to_string();
        let paths = Paths {
            log: g("LOG"),
            gus: g("GUS"),
            game: g("GAME"),
            backups: g("BKP"),
        };
        self.caches.lock().expect("lock").paths = Some(paths.clone());
        Ok(paths)
    }

    /// Last `n` log lines with secrets hidden. `n` is passed on as text (`NaN` stays `NaN`, as in the original).
    pub async fn log_tail(&self, n: &str) -> Result<Value, String> {
        let log = self.paths().await?.log;
        if log.is_empty() {
            return Ok(json!({"ok": false, "error": "server log not found", "lines": []}));
        }
        let r = self
            .exec
            .guest_shell(
                &format!("tail -n {n} {} 2>/dev/null", shq(&log)),
                ShellOpts::user(&self.user),
            )
            .await?;
        let lines: Vec<Value> = r
            .out
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|l| Value::String(redact_secrets(l)))
            .collect();
        Ok(json!({"ok": true, "lines": lines}))
    }

    pub async fn read_config(&self, which: &str) -> Result<Value, String> {
        let paths = self.paths().await?;
        let path = if which == "game" {
            paths.game
        } else {
            paths.gus
        };
        if path.is_empty() {
            return Ok(json!({"ok": false, "error": "config file not found", "ini": ""}));
        }
        let r = self
            .exec
            .guest_shell(
                &format!("cat {} 2>/dev/null", shq(&path)),
                ShellOpts::user(&self.user),
            )
            .await?;
        Ok(json!({"ok": true, "path": path, "ini": redact_secrets(&r.out)}))
    }

    /// Overwrite a config file. Lines the editor still shows redacted are restored from disk.
    pub async fn write_config_raw(&self, which: &str, content: &str) -> Result<Value, String> {
        let paths = self.paths().await?;
        let path = if which == "game" {
            paths.game
        } else {
            paths.gus
        };
        if path.is_empty() {
            return Ok(json!({"ok": false, "error": "config file not found"}));
        }
        let cur = self
            .exec
            .guest_shell(
                &format!("cat {} 2>/dev/null", shq(&path)),
                ShellOpts::user(&self.user),
            )
            .await?;
        let mut real: Vec<(String, String)> = Vec::new();
        for l in cur.out.split('\n') {
            if let Some((_, k, _, v)) = match_kv_greedy(l) {
                // value of `^\s*KEY\s*=\s*(.*)$` (leading blanks already consumed)
                if let Some(e) = real.iter_mut().find(|(rk, _)| rk == k) {
                    e.1 = v.to_string();
                } else {
                    real.push((k.to_string(), v.to_string()));
                }
            }
        }
        let merged: Vec<String> = content
            .split('\n')
            .map(|l| match match_kv_greedy(l) {
                Some((lead, k, mid, v)) if v.contains('\u{2022}') => {
                    match real.iter().find(|(rk, _)| rk == k) {
                        Some((_, rv)) => format!("{lead}{k}{mid}{rv}"),
                        None => l.to_string(),
                    }
                }
                _ => l.to_string(),
            })
            .collect();
        let merged = merged.join("\n");
        self.exec
            .guest_exec(
                &["/bin/sh", "-c", &format!("cat > {}", shq(&path))],
                ShellOpts::user(&self.user)
                    .input(ascii_safe(&merged))
                    .timeout(15_000),
            )
            .await?;
        self.caches.lock().expect("lock").cfg = None;
        Ok(json!({"ok": true, "path": path}))
    }

    /// Write one `key=value` under its section of GameUserSettings.ini (or Game.ini).
    pub async fn set_config_key(&self, key: &str, value: &str, file: &str) -> Result<Value, String> {
        if !valid_name(key) {
            return Ok(json!({"ok": false, "error": "invalid key"}));
        }
        let paths = self.paths().await?;
        let path = if file == "game" {
            paths.game
        } else {
            paths.gus
        };
        if path.is_empty() {
            return Ok(json!({"ok": false, "error": "config file not found"}));
        }
        let val = ascii_safe(&value.replace(['\r', '\n'], ""));
        let section = if file == "game" {
            "[/script/shootergame.shootergamemode]"
        } else {
            "[ServerSettings]"
        };
        let script = format!(
            "python3 - {} {} {} {} <<'PYEOF'\n{PY_SET_KEY}PYEOF",
            shq(&path),
            shq(key),
            shq(&val),
            shq(section)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(15_000))
            .await?;
        self.caches.lock().expect("lock").cfg = None;
        let out = if r.out.is_empty() { &r.err } else { &r.out };
        Ok(json!({"ok": r.out.contains("OK"), "output": js_trim(out), "path": path}))
    }

    pub async fn read_platforms(&self) -> Map<String, Value> {
        let script = format!(
            "grep -E '^CustomStartParams=' {} 2>/dev/null | tail -1",
            shq(&self.map_conf_path())
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(12_000))
            .await
            .unwrap_or_default();
        static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = R.get_or_init(|| {
            Regex::new(r"-[Ss][Ee][Rr][Vv][Ee][Rr][Pp][Ll][Aa][Tt][Ff][Oo][Rr][Mm]=([A-Za-z0-9+]+)")
                .expect("re")
        });
        let mut o = Map::new();
        match re.captures(&r.out) {
            Some(c) if !c[1].eq_ignore_ascii_case("all") => {
                let on: Vec<String> = c[1].split('+').map(str::to_uppercase).collect();
                for p in PLATFORMS {
                    o.insert(p.into(), Value::Bool(on.iter().any(|t| t == p)));
                }
            }
            _ => {
                for p in PLATFORMS {
                    o.insert(p.into(), Value::Bool(true));
                }
            }
        }
        o
    }

    /// Which platforms may join, as the launch argument `-ServerPlatform=PC+XSX+...` (applies on next restart).
    pub async fn set_platforms(&self, list: &Value) -> Result<Value, String> {
        let valid: Vec<String> = match list {
            Value::Array(a) => a
                .iter()
                .map(|t| js_str(t).to_uppercase())
                .filter(|t| PLATFORMS.contains(&t.as_str()))
                .collect(),
            _ => Vec::new(),
        };
        let uniq: Vec<&str> = PLATFORMS
            .iter()
            .copied()
            .filter(|p| valid.iter().any(|v| v == p))
            .collect();
        let arg = if uniq.is_empty() || uniq.len() == PLATFORMS.len() {
            "ALL".to_string()
        } else {
            uniq.join("+")
        };
        let path = self.map_conf_path();
        let script = format!(
            "python3 - {} {} <<'PYEOF'\n{PY_SET_PLATFORMS}PYEOF",
            shq(&path),
            shq(&arg)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(15_000))
            .await?;
        self.caches.lock().expect("lock").cfg = None;
        let out = if r.out.is_empty() { &r.err } else { &r.out };
        Ok(json!({"ok": r.out.contains("OK"), "output": js_trim(out), "platform": arg}))
    }

    /// Settings summary (multipliers, rules, session name), cached for 60 s.
    pub async fn config_summary(&self) -> Result<Value, String> {
        if let Some((v, at)) = self.caches.lock().expect("lock").cfg.clone()
            && now_ms() - at < 60_000.0
        {
            return Ok(v);
        }
        let c = self
            .read_config("gus")
            .await
            .unwrap_or_else(|_| json!({"ok": false, "ini": ""}));
        let cg = self
            .read_config("game")
            .await
            .unwrap_or_else(|_| json!({"ok": false, "ini": ""}));
        let readable = |r: &Value| {
            r.get("ok") != Some(&Value::Bool(false))
                && r.get("ini")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
        };
        let (gus_ok, game_ok) = (readable(&c), readable(&cg));
        let platforms = self.read_platforms().await;
        let value = if !gus_ok && !game_ok {
            json!({
                "ok": false, "unreadable": true, "reason": "De serverconfiguratie kon niet gelezen worden.",
                "multipliers": [], "rules": null, "sessionName": "", "all": {}
            })
        } else {
            let ini = |r: &Value| {
                r.get("ini")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            // Game.ini first, GameUserSettings.ini wins on conflict
            let mut kv = parse_conf(&ini(&cg));
            for (k, v) in parse_conf(&ini(&c)) {
                kv.insert(k, v);
            }
            let mut lower: Map<String, Value> = Map::new();
            for (k, v) in &kv {
                lower.insert(k.to_lowercase(), v.clone());
            }
            let getf = |k: &str, d: f64| -> f64 {
                match lower.get(&k.to_lowercase()).and_then(Value::as_str) {
                    Some(s) => {
                        let v = parse_float(s);
                        if v.is_nan() { d } else { v }
                    }
                    None => d,
                }
            };
            let getb = |k: &str, d: bool| -> bool {
                match lower.get(&k.to_lowercase()).and_then(Value::as_str) {
                    None => d,
                    Some(s) => s.eq_ignore_ascii_case("true"),
                }
            };
            let mult = |label: &str, key: &str, base: f64, max: f64, file: &str| -> Value {
                let v = getf(key, base);
                json!({
                    "k": label, "key": key, "file": file, "v": format!("{}\u{d7}", num_str(v)), "val": num(v), "v2": num_str(v),
                    "base": format!("{}\u{d7}", num_str(base)), "baseNum": num(base), "max": num(max),
                    "pct": num(100f64.min(js_round(v / max * 100.0))), "hot": v > base,
                })
            };
            let multipliers = vec![
                mult("XP", "XPMultiplier", 1.0, 10.0, "gus"),
                mult("Harvest", "HarvestAmountMultiplier", 1.0, 10.0, "gus"),
                mult("Taming", "TamingSpeedMultiplier", 1.0, 10.0, "gus"),
                mult("Maturation", "BabyMatureSpeedMultiplier", 1.0, 30.0, "game"),
                mult(
                    "Mating interval",
                    "MatingIntervalMultiplier",
                    1.0,
                    5.0,
                    "game",
                ),
                mult("Egg hatch", "EggHatchSpeedMultiplier", 1.0, 30.0, "game"),
            ];
            let pf = |k: &str| platforms.get(k).and_then(Value::as_bool).unwrap_or(false);
            let rules = json!({
                "mode": if getb("ServerPVE", true) { "PvE" } else { "PvP" },
                "difficulty": num(getf("OverrideOfficialDifficulty", getf("DifficultyOffset", 1.0))),
                "maxWild": num(js_round(getf("OverrideOfficialDifficulty", 1.0) * 30.0)),
                "tameLimit": num(js_round(getf("MaxTamedDinos", 5000.0))),
                "crosshair": getb("ServerCrosshair", true),
                "thirdPerson": getb("AllowThirdPersonPlayer", true),
                "flyerCarry": getb("AllowFlyerCarryPvE", true),
                "platforms": Value::Object(platforms.clone()),
                "crossplay": pf("PC") || pf("XSX") || pf("PS5") || pf("WINGDK"),
            });
            json!({
                "ok": c.get("ok").cloned().unwrap_or(Value::Null),
                "unreadable": false,
                "read": {"gus": gus_ok, "game": game_ok},
                "multipliers": multipliers,
                "rules": rules,
                "sessionName": conf_get(&kv, "SessionName").filter(|s| !s.is_empty()).unwrap_or(""),
                "all": Value::Object(kv),
            })
        };
        self.caches.lock().expect("lock").cfg = Some((value.clone(), now_ms()));
        Ok(value)
    }

    pub async fn update_check(&self) -> Result<Value, String> {
        let mf = format!(
            "{}/server-files/steamapps/appmanifest_2430930.acf",
            self.dir
        );
        let sc = format!("{}/steamcmd/steamcmd.sh", self.dir);
        let r1 = self
            .exec
            .guest_shell(
                &format!(
                    "grep -aE '\"buildid\"' {} 2>/dev/null | head -1 | grep -oE '[0-9]+' | head -1",
                    shq(&mf)
                ),
                ShellOpts::user(&self.user),
            )
            .await?;
        let installed = r1.out.trim().split('\n').next().unwrap_or("").to_string();
        let script = format!(
            "{} +login anonymous +app_info_update 1 +app_info_print 2430930 +quit 2>/dev/null | tr -d '\\t' | awk '/\"public\"/{{f=1}} f&&/\"buildid\"/{{print; exit}}' | grep -oE '[0-9]+' | head -1",
            shq(&sc)
        );
        let (latest, latest_err) = match self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(90_000))
            .await
        {
            Ok(r2) => (
                r2.out.trim().split('\n').next().unwrap_or("").to_string(),
                Value::Null,
            ),
            Err(_) => (String::new(), json!("SteamCMD query timed out")),
        };
        Ok(json!({
            "ok": true, "appId": "2430930", "installed": installed, "latest": latest, "latestErr": latest_err,
            "updateAvailable": !installed.is_empty() && !latest.is_empty() && installed != latest,
        }))
    }

    pub async fn apply_update(&self) -> Result<Value, String> {
        let _ = self
            .rcon("Broadcast Server update starting \u{2014} saving and restarting shortly")
            .await;
        let _ = self.rcon("SaveWorld").await;
        let script = format!(
            "systemctl stop {s}; runuser -u {u} -- bash -lc 'cd {d} && ./ark-manager.sh update' ; systemctl start {s}",
            s = self.service,
            u = self.user,
            d = shq(&self.dir)
        );
        let r = self
            .exec
            .guest_shell(
                &format!(
                    "nohup bash -c {} > /tmp/ark-dashboard-update.log 2>&1 & echo STARTED",
                    shq(&script)
                ),
                ShellOpts::default().timeout(15_000),
            )
            .await?;
        Ok(json!({"ok": r.out.contains("STARTED"), "output": js_trim(&r.out)}))
    }

    async fn ensure_backup_helper(&self) -> Result<String, String> {
        let path = format!("{}/dashboard-backup.sh", self.dir);
        let done = self.caches.lock().expect("lock").bk_helper;
        if !done {
            self.exec
                .guest_exec(
                    &[
                        "/bin/sh",
                        "-c",
                        &format!("cat > {} && chmod +x {}", shq(&path), shq(&path)),
                    ],
                    ShellOpts::user(&self.user)
                        .input(ascii_safe(BACKUP_HELPER))
                        .timeout(15_000),
                )
                .await?;
            self.caches.lock().expect("lock").bk_helper = true;
        }
        Ok(path)
    }

    pub async fn create_backup(&self) -> Result<Value, String> {
        let helper = self.ensure_backup_helper().await?;
        let script = format!(
            "runuser -u {} -- bash {} {} {} {} 2>&1 | tail -3",
            self.user,
            shq(&helper),
            shq(&self.map_dir()),
            shq(&format!("{}/server-files", self.dir)),
            shq(&self.display)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::default().timeout(120_000))
            .await?;
        let out = js_trim(&r.out).to_string();
        Ok(json!({"ok": out.contains("OK saved="), "output": out}))
    }

    pub async fn restore_backup(&self, name: &str) -> Result<Value, String> {
        if !valid_snapshot(name) {
            return Ok(json!({"ok": false, "error": "invalid snapshot name"}));
        }
        let bk = format!("{}/backups", self.map_dir());
        let saved = format!("{}/server-files/ShooterGame/Saved", self.dir);
        let excl = "--exclude='*_[0-9][0-9].[0-9][0-9].[0-9][0-9][0-9][0-9]_*.ark'";
        let script = format!(
            "systemctl stop {svc}; sleep 2; SAVED={saved}; BK={bk}; SR=$(find \"$SAVED\" -maxdepth 3 -name '*.ark' -printf '%P\\n' 2>/dev/null | head -1 | cut -d/ -f1); [ -n \"$SR\" ] && runuser -u {user} -- tar -czf \"$BK/{disp}_pre-restore_$(date +%Y%m%d_%H%M%S).tar.gz\" -C \"$SAVED\" {excl} \"$SR\" 2>/dev/null; runuser -u {user} -- tar -xzf {arch} -C \"$SAVED\" 2>&1 | tail -2; systemctl start {svc}",
            svc = self.service,
            saved = shq(&saved),
            bk = shq(&bk),
            user = self.user,
            disp = self.display,
            arch = shq(&format!("{bk}/{name}")),
        );
        let r = self
            .exec
            .guest_shell(
                &format!(
                    "nohup bash -c {} > /tmp/ark-dashboard-restore.log 2>&1 & echo STARTED",
                    shq(&script)
                ),
                ShellOpts::default().timeout(15_000),
            )
            .await?;
        Ok(json!({"ok": r.out.contains("STARTED"), "output": js_trim(&r.out)}))
    }

    pub async fn backups_list(&self) -> Result<Value, String> {
        let backups = self.paths().await?.backups;
        if backups.is_empty() {
            return Ok(json!({"ok": true, "backups": []}));
        }
        let b = shq(&backups);
        let script = format!(
            "ls -1t {b} 2>/dev/null | head -40 | while read f; do sz=$(du -h {b}/\"$f\" 2>/dev/null | cut -f1); ts=$(stat -c %y {b}/\"$f\" 2>/dev/null | cut -d. -f1); echo \"$f|$sz|$ts\"; done"
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user))
            .await?;
        let list: Vec<Value> = r
            .out
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut p = l.split('|');
                let name = p.next().unwrap_or("");
                let size = p.next().unwrap_or("");
                let when = p.next().unwrap_or("");
                let lc = name.to_ascii_lowercase();
                let trigger = if lc.contains("_pre-restore_") {
                    "auto"
                } else if lc.contains("_scheduled_") {
                    "scheduled"
                } else if lc.contains("_pre-update_") {
                    "update"
                } else {
                    "manual"
                };
                json!({"name": name, "size": size, "when": when, "trigger": trigger})
            })
            .collect();
        Ok(json!({"ok": true, "dir": backups, "backups": list}))
    }

    pub async fn delete_backup(&self, name: &str) -> Result<Value, String> {
        if !valid_snapshot(name) {
            return Ok(json!({"ok": false, "error": "invalid snapshot name"}));
        }
        let backups = self.paths().await?.backups;
        if backups.is_empty() {
            return Ok(json!({"ok": false, "error": "backup dir not found"}));
        }
        let r = self
            .exec
            .guest_shell(
                &format!(
                    "rm -f {} && echo DELETED",
                    shq(&format!("{backups}/{name}"))
                ),
                ShellOpts::user(&self.user).timeout(15_000),
            )
            .await?;
        Ok(json!({"ok": r.out.contains("DELETED"), "output": js_trim(&r.out)}))
    }

    async fn set_map_conf_key(&self, key: &str, value: &str) -> Result<bool, String> {
        if !valid_name(key) {
            return Ok(false);
        }
        let val = ascii_safe(&value.replace(['\r', '\n'], ""));
        let script = format!(
            "python3 - {} {} {} <<'PYEOF'\n{PY_SET_MAPCONF}PYEOF",
            shq(&self.map_conf_path()),
            shq(key),
            shq(&val)
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user).timeout(15_000))
            .await?;
        Ok(r.out.contains("OK"))
    }

    /// Session name and admin/server passwords (`sessionName`, `adminPassword`, `serverPassword`).
    pub async fn update_settings(
        &self,
        session_name: Option<&Value>,
        admin_pw: Option<&Value>,
        server_pw: Option<&Value>,
    ) -> Result<Value, String> {
        let mut changed: Vec<&str> = Vec::new();
        let mut failed: Vec<&str> = Vec::new();
        let given = |v: Option<&Value>, trim: bool| -> Option<String> {
            match v {
                None | Some(Value::Null) => None,
                Some(x) => {
                    let s = js_str(x);
                    let probe = if trim {
                        js_trim(&s).to_string()
                    } else {
                        s.clone()
                    };
                    if probe.is_empty() { None } else { Some(s) }
                }
            }
        };
        if let Some(n) = given(session_name, true) {
            if is_json_ok(&self.set_config_key("SessionName", &n, "gus").await?) {
                changed.push("name")
            } else {
                failed.push("name")
            }
        }
        if let Some(p) = given(admin_pw, false) {
            let a = self.set_map_conf_key("AdminPassword", &p).await?;
            let b = is_json_ok(&self.set_config_key("ServerAdminPassword", &p, "gus").await?);
            if a && b {
                changed.push("adminPassword")
            } else {
                failed.push("adminPassword")
            }
        }
        if let Some(p) = given(server_pw, false) {
            let a = self.set_map_conf_key("ServerPassword", &p).await?;
            let b = is_json_ok(&self.set_config_key("ServerPassword", &p, "gus").await?);
            if a && b {
                changed.push("serverPassword")
            } else {
                failed.push("serverPassword")
            }
        }
        self.caches.lock().expect("lock").cfg = None;
        if changed.is_empty() && failed.is_empty() {
            return Ok(json!({"ok": false, "error": "nothing to change", "changed": []}));
        }
        if !failed.is_empty() {
            return Ok(json!({"ok": false, "error": format!("failed to write: {}", failed.join(", ")), "changed": changed, "failed": failed}));
        }
        Ok(json!({"ok": true, "changed": changed, "note": "applies on next restart"}))
    }

    pub async fn schedule(&self) -> Result<Value, String> {
        let script = "crontab -l 2>/dev/null | grep -vE '^\\s*#' | grep -iE 'ark-manager|ark|healthcheck' | head -30";
        let r = self
            .exec
            .guest_shell(script, ShellOpts::user(&self.user))
            .await?;
        static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re =
            R.get_or_init(|| Regex::new(r"^(\S+\s+\S+\s+\S+\s+\S+\s+\S+)\s+(.*)$").expect("re"));
        let jobs: Vec<Value> = r
            .out
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|line| {
                let (cron, cmd) = match re.captures(line) {
                    Some(c) => (c[1].to_string(), c[2].to_string()),
                    None => (String::new(), line.to_string()),
                };
                let lc = cmd.to_ascii_lowercase();
                let ty = if lc.contains("dashboard-set-rate") {
                    "multiplier"
                } else if lc.contains("backup") {
                    "backup"
                } else if lc.contains("restart") {
                    "restart"
                } else if lc.contains("healthcheck") {
                    "healthcheck"
                } else if lc.contains("broadcast") {
                    "broadcast"
                } else if lc.contains("rcon") {
                    "rcon"
                } else {
                    "task"
                };
                // cmd.replace(/\s*>.*$/, '').slice(0, 80)
                let cut = match cmd.find('>') {
                    Some(i) => cmd[..i].trim_end_matches(crate::util::js_ws).to_string(),
                    None => cmd.clone(),
                };
                json!({"cron": cron, "cmd": slice_units(&cut, 80), "type": ty})
            })
            .collect();
        Ok(json!({"ok": true, "jobs": jobs}))
    }

    async fn ensure_rate_helper(&self) -> Result<(), String> {
        if self.caches.lock().expect("lock").rate_helper {
            return Ok(());
        }
        let path = format!("{}/dashboard-set-rate.sh", self.dir);
        self.exec
            .guest_exec(
                &[
                    "/bin/sh",
                    "-c",
                    &format!("cat > {} && chmod +x {}", shq(&path), shq(&path)),
                ],
                ShellOpts::user(&self.user)
                    .input(ascii_safe(RATE_HELPER))
                    .timeout(15_000),
            )
            .await?;
        self.caches.lock().expect("lock").rate_helper = true;
        Ok(())
    }

    /// Add a scheduled task to the crontab. `extra` holds `key,value,file,restart,endCron,revertValue,rates`.
    pub async fn add_schedule(
        &self,
        task_type: Option<&Value>,
        cron: Option<&Value>,
        message: Option<&Value>,
        extra: &Map<String, Value>,
    ) -> Result<Value, String> {
        let cron_s = js_str_opt_local(cron);
        let clean = |m: Option<&Value>| -> String {
            let raw = if truthy(m) {
                js_str_opt_local(m)
            } else {
                "Scheduled announcement".to_string()
            };
            slice_units(&ascii_safe(&raw.replace(['"', '\r', '\n'], "")), 120)
        };
        let msg = clean(message);
        if !ok_cron(&cron_s) {
            return Ok(json!({"ok": false, "error": "invalid cron (need 5 fields)"}));
        }
        let tt = task_type.and_then(Value::as_str).unwrap_or("");
        let mut lines: Vec<String> = Vec::new();
        let dir = &self.dir;
        let md = self.map_dir();
        if tt == "multiplier" {
            let raw: Vec<Value> = match extra.get("rates") {
                Some(Value::Array(a)) if !a.is_empty() => a.clone(),
                _ => vec![
                    json!({"key": extra.get("key"), "value": extra.get("value"), "file": extra.get("file"), "revertValue": extra.get("revertValue")}),
                ],
            };
            struct Rate {
                key: String,
                file: &'static str,
                val: String,
                rev: String,
            }
            let mut rates: Vec<Rate> = Vec::new();
            for r in &raw {
                let key = if truthy(r.get("key")) {
                    js_str_opt_local(r.get("key"))
                } else {
                    String::new()
                };
                let file = if r.get("file").and_then(Value::as_str) == Some("game") {
                    "game"
                } else {
                    "gus"
                };
                let val = match r.get("value") {
                    Some(v) if !v.is_null() => js_str(v),
                    _ => String::new(),
                };
                let rev = match r.get("revertValue") {
                    Some(v) if !v.is_null() => js_str(v),
                    _ => "1.0".to_string(),
                };
                if !valid_name(&key) {
                    return Ok(json!({"ok": false, "error": format!("invalid multiplier key: {}", if key.is_empty() { "(empty)" } else { &key })}));
                }
                if !num_re_ok(&val) {
                    return Ok(json!({"ok": false, "error": format!("invalid value for {key}")}));
                }
                if !num_re_ok(&rev) {
                    return Ok(json!({"ok": false, "error": format!("invalid revert value for {key}")}));
                }
                rates.push(Rate {
                    key,
                    file,
                    val,
                    rev,
                });
            }
            if rates.is_empty() {
                return Ok(json!({"ok": false, "error": "no rates given"}));
            }
            self.ensure_rate_helper().await?;
            let restart = if truthy(extra.get("restart")) {
                format!(" ; ./ark-manager.sh restart {}", self.display)
            } else {
                String::new()
            };
            let set_cmds = |use_revert: bool| -> String {
                rates
                    .iter()
                    .map(|r| {
                        format!(
                            "bash {dir}/dashboard-set-rate.sh {} {} {} {md}",
                            r.key,
                            if use_revert { &r.rev } else { &r.val },
                            r.file
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ; ")
            };
            let mk = |c: &str, m: &str, use_revert: bool| -> String {
                format!(
                    "{} cd {dir} && ./ark-manager.sh rcon {} \"Broadcast {m}\" ; {}{restart}",
                    js_trim(c),
                    self.display,
                    set_cmds(use_revert)
                )
            };
            lines.push(mk(&cron_s, &msg, false));
            if let Some(end) = extra.get("endCron") {
                let end_s = js_str(end);
                if ok_cron(&end_s) {
                    lines.push(mk(
                        &end_s,
                        &clean(Some(&Value::String(
                            "Event ended \u{2014} rates back to normal".into(),
                        ))),
                        true,
                    ));
                }
            }
        } else if tt == "backup" {
            let helper = self.ensure_backup_helper().await?;
            lines.push(format!(
                "{} bash {helper} {md} {dir}/server-files {} scheduled",
                js_trim(&cron_s),
                self.display
            ));
        } else {
            let cmd = match tt {
                "restart" => format!("./ark-manager.sh restart {}", self.display),
                "save" => format!("./ark-manager.sh rcon {} \"SaveWorld\"", self.display),
                "broadcast" => {
                    format!("./ark-manager.sh rcon {} \"Broadcast {msg}\"", self.display)
                }
                _ => return Ok(json!({"ok": false, "error": "unknown task type"})),
            };
            lines.push(format!("{} cd {dir} && {cmd}", js_trim(&cron_s)));
        }
        let echoes = lines
            .iter()
            .map(|l| format!("echo {}", shq(l)))
            .collect::<Vec<_>>()
            .join("; ");
        let frag: String = {
            let first: Vec<char> = lines[0].chars().collect();
            first[first.len().saturating_sub(45)..].iter().collect()
        };
        let script = format!(
            "( crontab -l 2>/dev/null; {echoes} ) | crontab - && echo ADDED; {}",
            if frag.is_empty() {
                String::new()
            } else {
                format!(
                    "crontab -l 2>/dev/null | grep -qF {} && echo VERIFIED",
                    shq(&frag)
                )
            }
        );
        let r = self
            .exec
            .guest_shell(&script, ShellOpts::user(&self.user))
            .await?;
        Ok(json!({"ok": r.out.contains("ADDED"), "verified": r.out.contains("VERIFIED"), "lines": lines}))
    }

    pub async fn delete_schedule(&self, m: Option<&Value>) -> Result<Value, String> {
        let s = if truthy(m) {
            js_str_opt_local(m)
        } else {
            String::new()
        };
        if !truthy(m) || s.encode_utf16().count() < 4 {
            return Ok(json!({"ok": false, "error": "bad match"}));
        }
        let r = self
            .exec
            .guest_shell(
                &format!(
                    "crontab -l 2>/dev/null | grep -vF {} | crontab - && echo DELETED",
                    shq(&s)
                ),
                ShellOpts::user(&self.user),
            )
            .await?;
        Ok(json!({"ok": r.out.contains("DELETED")}))
    }
}

fn js_str_opt_local(v: Option<&Value>) -> String {
    crate::util::js_str_opt(v)
}

/// systemd CPU-time span per second to cores (`1s` is one core, `500ms` half).
pub fn span_to_cores(v: Option<&str>) -> Value {
    let Some(v) = v else { return Value::Null };
    if v.is_empty() || v.to_ascii_lowercase().contains("infinity") {
        return Value::Null;
    }
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"([0-9]+)(min|ms|us|s)").expect("re"));
    let mut us = 0f64;
    let mut any = false;
    for c in re.captures_iter(v) {
        any = true;
        let n: f64 = c[1].parse().unwrap_or(0.0);
        us += match &c[2] {
            "min" => n * 6e7,
            "s" => n * 1e6,
            "ms" => n * 1e3,
            _ => n,
        };
    }
    if !any {
        match parse_int(v) {
            Some(n) => us = n as f64,
            None => return Value::Null,
        }
    }
    num(js_round(us / 1e6 * 100.0) / 100.0)
}

/// Bytes (as text) to GB with one decimal; `infinity`, empty or unset give null.
pub fn bytes_to_gb(v: Option<&str>) -> Value {
    match v {
        None | Some("") => Value::Null,
        Some(s) if s.to_ascii_lowercase().contains("infinity") => Value::Null,
        Some(s) => match parse_int(s) {
            Some(n) => num(js_round(n as f64 / 1_073_741_824.0 * 10.0) / 10.0),
            None => Value::Null, // NaN is not valid JSON; JS prints null too
        },
    }
}

/// Parse the output of the map discovery script into maps (hostile directory names are dropped).
pub fn parse_discovery(out: &str) -> Vec<DiscoveredMap> {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"===MAP:(.+?)===").expect("re"));
    let mut maps = Vec::new();
    let mut last_end = 0;
    let mut cur: Option<String> = None;
    let mut push = |name: Option<String>, body: &str| {
        let Some(name) = name else { return };
        let display = js_trim(&name).to_string();
        if !valid_name(&display) {
            eprintln!(
                "[ark] map-dir met onveilige naam genegeerd: {}",
                Value::String(display)
            );
            return;
        }
        let conf = parse_conf(body);
        let g = |k: &str| conf_get(&conf, k).map(String::from);
        maps.push(DiscoveredMap {
            internal: g("MapName")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| display.clone()),
            save_dir: g("SaveDir")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| display.clone()),
            game_port: g("GamePort"),
            query_port: g("QueryPort"),
            rcon_port: g("RCONPort"),
            max_players: g("MaxPlayers"),
            display,
        });
    };
    for c in re.captures_iter(out) {
        let m = c.get(0).expect("match");
        push(cur.take(), &out[last_end..m.start()]);
        cur = Some(c[1].to_string());
        last_end = m.end();
    }
    push(cur.take(), &out[last_end..]);
    maps
}

pub async fn discover_maps(exec: &Arc<dyn Exec>, cfg: &ArkCfg) -> Result<Vec<DiscoveredMap>, String> {
    let dir = cfg.dir();
    let script = format!(
        "for d in {}/maps/*/; do n=$(basename \"$d\"); [ -f \"$d/map.conf\" ] || continue; echo \"===MAP:$n===\"; cat \"$d/map.conf\" 2>/dev/null; done",
        shq(&dir)
    );
    let r = exec
        .guest_shell(&script, ShellOpts::user(&cfg.user).timeout(20_000))
        .await?;
    Ok(parse_discovery(&r.out))
}

/// Create a map by writing its files in small steps (content over stdin, passwords never in argv).
pub async fn create_map(exec: &Arc<dyn Exec>, cfg: &ArkCfg, o: &Map<String, Value>) -> Result<Value, String> {
    let dir = cfg.dir();
    let display_v = o.get("display");
    let display = js_str_opt_local(display_v);
    let internal = js_str_opt_local(o.get("internal"));
    if !valid_name(&display) {
        return Ok(json!({"ok": false, "error": "invalid display name (letters/numbers/underscore only)"}));
    }
    if !truthy(o.get("internal")) {
        return Ok(json!({"ok": false, "error": "internal map name required"}));
    }
    if !valid_name(&internal) {
        return Ok(json!({"ok": false, "error": "invalid internal map name (letters/numbers/underscore only)"}));
    }
    if !truthy(o.get("adminPassword")) {
        return Ok(json!({"ok": false, "error": "admin password required"}));
    }
    let mut ports = [0i64; 4];
    for (i, (nm, hi)) in [
        ("gamePort", 65535.0),
        ("queryPort", 65535.0),
        ("rconPort", 65535.0),
        ("maxPlayers", 255.0),
    ]
    .iter()
    .enumerate()
    {
        let v = js_number(o.get(*nm));
        if !(v.is_finite() && v.fract() == 0.0 && v >= 1.0 && v <= *hi) {
            return Ok(json!({"ok": false, "error": format!("invalid {nm}")}));
        }
        ports[i] = v as i64;
    }
    let [gp, qp, rp, _mp] = ports;
    let u = cfg.user.clone();
    let md = format!("{dir}/maps/{display}");
    let sess_src = match o.get("sessionName") {
        v if truthy(v) => js_str_opt_local(v),
        _ => display.clone(),
    };
    let sess = ascii_safe(&sess_src.replace(['\r', '\n', '"'], ""));
    let apw = ascii_safe(&js_str_opt_local(o.get("adminPassword"))).replace(['\r', '\n'], "");
    let spw_src = match o.get("serverPassword") {
        None => String::new(),
        Some(v) => js_str(v),
    };
    let spw = ascii_safe(&spw_src).replace(['\r', '\n'], "");

    let chk = exec
        .guest_shell(
            &format!("[ -d {} ] && echo EXISTS || echo OK", shq(&md)),
            ShellOpts::user(&u),
        )
        .await?;
    if chk.out.contains("EXISTS") {
        return Ok(json!({"ok": false, "error": "map already exists"}));
    }
    let write_file = |path: String, content: String| {
        let exec = exec.clone();
        let u = u.clone();
        async move {
            exec.guest_exec(
                &["/bin/sh", "-c", &format!("cat > {}", shq(&path))],
                ShellOpts::user(&u)
                    .input(ascii_safe(&content))
                    .timeout(15_000),
            )
            .await
        }
    };
    exec.guest_shell(
        &format!("mkdir -p {}/backups", shq(&md)),
        ShellOpts::user(&u),
    )
    .await?;

    let raw = |k: &str| js_str_opt_local(o.get(k));
    let map_conf = format!(
        "MapName={internal}\nDisplayName={display}\nSaveDir={display}\nGamePort={}\nQueryPort={}\nRCONPort={}\nMaxPlayers={}\nAdminPassword={apw}\nServerPassword={spw}\nCustomStartParams=-NoBattlEye -crossplay -NoHangDetection\nClusterID=\nBatchEnabled=true\n",
        raw("gamePort"),
        raw("queryPort"),
        raw("rconPort"),
        raw("maxPlayers")
    );
    write_file(format!("{md}/map.conf"), map_conf).await?;
    exec.guest_shell(
        &format!("chmod 600 {}/map.conf", shq(&md)),
        ShellOpts::user(&u),
    )
    .await?;
    write_file(
        format!("{md}/mods.conf"),
        format!("# Mod configuration for {display}\n# Format: id|name|enabled\n"),
    )
    .await?;
    write_file(format!("{md}/Game.ini"), String::new()).await?;

    // GUS: use an existing map's file as template
    let src = exec
        .guest_shell(
            &format!(
                "cat \"$(ls {}/maps/*/GameUserSettings.ini 2>/dev/null | head -1)\" 2>/dev/null",
                shq(&dir)
            ),
            ShellOpts::user(&u).timeout(15_000),
        )
        .await?;
    let mut gus = if src.out.is_empty() {
        "[ServerSettings]\nRCONEnabled=True\n".to_string()
    } else {
        src.out
    };
    let sets: Vec<(&str, String)> = vec![
        ("SessionName", sess),
        ("ServerAdminPassword", apw.clone()),
        ("ServerPassword", spw.clone()),
        ("RCONPort", raw("rconPort")),
        ("MaxPlayers", raw("maxPlayers")),
        ("RCONEnabled", "True".into()),
    ];
    let mut seen: Vec<&str> = Vec::new();
    let mapped: Vec<String> = gus
        .split('\n')
        .map(|l| {
            let t = l.trim_start_matches(crate::util::js_ws);
            let kend = t
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(t.len());
            if kend > 0
                && t[kend..]
                    .trim_start_matches(crate::util::js_ws)
                    .starts_with('=')
                && let Some((k, v)) = sets.iter().find(|(k, _)| *k == &t[..kend])
            {
                seen.push(k);
                return format!("{k}={v}");
            }
            l.to_string()
        })
        .collect();
    gus = mapped.join("\n");
    let mut lines: Vec<String> = gus.split('\n').map(String::from).collect();
    let idx = match lines
        .iter()
        .position(|l| js_trim(l).eq_ignore_ascii_case("[serversettings]"))
    {
        Some(i) => i,
        None => {
            lines.insert(0, "[ServerSettings]".into());
            0
        }
    };
    for (k, v) in &sets {
        if !seen.contains(k) {
            lines.insert(idx + 1, format!("{k}={v}"));
        }
    }
    write_file(format!("{md}/GameUserSettings.ini"), lines.join("\n")).await?;

    exec.guest_shell(
        &format!("MC={}/config/maps.conf; grep -q \"^{display}|\" \"$MC\" 2>/dev/null || echo \"{display}|{internal}|enabled\" >> \"$MC\"", shq(&dir)),
        ShellOpts::user(&u),
    )
    .await?;

    let svc = format!("ark-{}.service", display.to_lowercase());
    let svc_body = format!(
        "[Unit]\nDescription=ARK ASA Server - {display}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=forking\nUser={}\nWorkingDirectory={dir}\nEnvironment=PROTON_LOG=1\nEnvironment=PROTON_LOG_DIR=/home/{}\nExecStart={dir}/ark-manager.sh start {display}\nExecStop={dir}/ark-manager.sh stop {display}\nRestart=on-failure\nRestartSec=30\nTimeoutStartSec=300\nTimeoutStopSec=180\nMemoryAccounting=true\nCPUAccounting=true\n\n[Install]\nWantedBy=multi-user.target\n",
        cfg.user, cfg.user
    );
    exec.guest_exec(
        &["/bin/sh", "-c", &format!("cat > /etc/systemd/system/{svc}")],
        ShellOpts::default().input(svc_body).timeout(15_000),
    )
    .await?;
    let rs = exec
        .guest_shell(
            &format!("systemctl daemon-reload; systemctl enable {svc} 2>/dev/null; command -v ufw >/dev/null && {{ ufw allow {gp}/udp; ufw allow {qp}/udp; ufw allow {rp}/tcp; }} 2>/dev/null; echo SVCOK"),
            ShellOpts::default().timeout(20_000),
        )
        .await?;
    Ok(json!({"ok": rs.out.contains("SVCOK"), "display": display, "service": svc, "output": js_trim(&rs.out)}))
}

/// Stop and remove a map: service, unit file, firewall rules, directory and maps.conf entry.
pub async fn delete_map(exec: &Arc<dyn Exec>, cfg: &ArkCfg, display: &str) -> Result<Value, String> {
    if !valid_name(display) {
        return Ok(json!({"ok": false, "error": "invalid map name"}));
    }
    let dir = cfg.dir();
    let svc = format!("ark-{}.service", display.to_lowercase());
    let md = format!("{dir}/maps/{display}");
    let script = format!(
        "systemctl stop {svc} 2>/dev/null; systemctl disable {svc} 2>/dev/null; rm -f /etc/systemd/system/{svc}; systemctl daemon-reload; MCF={}/map.conf; if [ -f \"$MCF\" ]; then GP=$(grep -E '^GamePort=' \"$MCF\" | tail -1 | cut -d= -f2); QP=$(grep -E '^QueryPort=' \"$MCF\" | tail -1 | cut -d= -f2); RP=$(grep -E '^RCONPort=' \"$MCF\" | tail -1 | cut -d= -f2); [ -n \"$GP\" ] && ufw --force delete allow \"$GP\"/udp 2>/dev/null; [ -n \"$QP\" ] && ufw --force delete allow \"$QP\"/udp 2>/dev/null; [ -n \"$RP\" ] && ufw --force delete allow \"$RP\"/tcp 2>/dev/null; fi; runuser -u {} -- rm -rf {}; MC={}/config/maps.conf; [ -f \"$MC\" ] && grep -v \"^{display}|\" \"$MC\" > \"$MC.tmp\" && mv \"$MC.tmp\" \"$MC\"; echo DELETED",
        shq(&md),
        cfg.user,
        shq(&md),
        shq(&dir)
    );
    let r = exec
        .guest_shell(&script, ShellOpts::default().timeout(20_000))
        .await?;
    Ok(json!({"ok": r.out.contains("DELETED"), "output": js_trim(&r.out)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeVm;

    fn cfg() -> ArkCfg {
        ArkCfg {
            user: "arkadmin".into(),
            manager: "/home/arkadmin/ark-manager/ark-manager.sh".into(),
            service_prefix: "ark-".into(),
            ram_alloc_gb: 16,
            cores_total: 4,
            rcon_password_override: String::new(),
        }
    }

    async fn island() -> (Arc<dyn Exec>, ArkInstance) {
        let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
        let maps = discover_maps(&exec, &cfg()).await.expect("exec");
        let m = maps
            .iter()
            .find(|m| m.display == "TheIsland")
            .expect("map")
            .clone();
        let ark = ArkInstance::new(exec.clone(), &cfg(), &m).expect("ark");
        (exec, ark)
    }

    #[tokio::test]
    async fn discovery_drops_hostile_names() {
        let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
        let maps = discover_maps(&exec, &cfg()).await.expect("exec");
        assert_eq!(
            maps.iter().map(|m| m.display.as_str()).collect::<Vec<_>>(),
            ["Ragnarok", "TheIsland"]
        );
    }

    #[test]
    fn unsafe_service_name_is_refused() {
        let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
        let m = DiscoveredMap {
            display: "x y".into(),
            internal: String::new(),
            save_dir: String::new(),
            game_port: None,
            query_port: None,
            rcon_port: None,
            max_players: None,
        };
        assert!(ArkInstance::new(exec, &cfg(), &m).is_err());
    }

    #[tokio::test]
    async fn rcon_lists_players_and_survives_a_rotated_password() {
        let (_e, ark) = island().await;
        let p = ark.players().await.expect("exec");
        assert_eq!(p["players"].as_array().map(Vec::len), Some(2));
        assert_eq!(p["players"][0]["name"], "Alice");
        // rotate the password behind the cached one: the next call re-reads map.conf once
        assert!(is_json_ok(
            &ark.update_settings(None, Some(&json!("new-pw")), None)
                .await.expect("exec")
        ));
        let r = ark.rcon("SaveWorld").await.expect("exec");
        assert_eq!(r["ok"], true);
    }

    #[tokio::test]
    async fn rcon_on_a_stopped_map_reports_connection_error() {
        let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
        let maps = discover_maps(&exec, &cfg()).await.expect("exec");
        let m = maps.iter().find(|m| m.display == "Ragnarok").expect("map");
        let ark = ArkInstance::new(exec, &cfg(), m).expect("ark");
        let r = ark.rcon("ListPlayers").await.expect("exec");
        assert_eq!(r["ok"], false);
        assert!(
            r["error"]
                .as_str()
                .unwrap_or("")
                .contains("Connection refused")
        );
    }

    #[tokio::test]
    async fn resources_round_trip_and_oom_guard() {
        let (_e, ark) = island().await;
        // 8 GB is above the 6 GB the world uses right now (x1.15 = 6.9): allowed
        let r = ark
            .set_resources(Some(&json!(2)), Some(&json!(8)), 4.0, 14.0)
            .await.expect("exec");
        assert_eq!(r["ok"], true);
        assert_eq!(ark.resources().await.expect("exec")["ramGB"], 8);
        // 2 GB would make the cgroup OOM-kill a running world: refused
        let g = ark.set_resources(None, Some(&json!(2)), 4.0, 14.0).await.expect("exec");
        assert_eq!(g["needsForce"], true);
    }

    #[tokio::test]
    async fn config_key_edit_and_redaction() {
        let (_e, ark) = island().await;
        let r = ark.set_config_key("XPMultiplier", "4.5", "gus").await.expect("exec");
        assert_eq!(r["ok"], true);
        let ini = ark.read_config("gus").await.expect("exec");
        assert!(
            ini["ini"]
                .as_str()
                .unwrap_or("")
                .contains("XPMultiplier=4.5")
        );
        assert!(
            ini["ini"]
                .as_str()
                .unwrap_or("")
                .contains("ServerAdminPassword=\u{2022}")
        );
        assert_eq!(
            ark.set_config_key("bad key", "1", "gus").await.expect("exec")["error"],
            "invalid key"
        );
    }

    #[tokio::test]
    async fn schedule_validation() {
        let (_e, ark) = island().await;
        let none = Map::new();
        let bad = ark
            .add_schedule(
                Some(&json!("restart")),
                Some(&json!("every day")),
                None,
                &none,
            )
            .await.expect("exec");
        assert_eq!(bad["error"], "invalid cron (need 5 fields)");
        let ok = ark
            .add_schedule(
                Some(&json!("restart")),
                Some(&json!("30 5 * * *")),
                None,
                &none,
            )
            .await.expect("exec");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["verified"], true, "{ok}");
        let unk = ark
            .add_schedule(Some(&json!("nuke")), Some(&json!("0 0 * * *")), None, &none)
            .await.expect("exec");
        assert_eq!(unk["error"], "unknown task type");
    }

    #[test]
    fn span_and_bytes() {
        assert_eq!(span_to_cores(Some("infinity")), Value::Null);
        assert_eq!(span_to_cores(Some("500ms")), json!(0.5));
        assert_eq!(span_to_cores(Some("2s")), json!(2));
        assert_eq!(bytes_to_gb(Some("6442450944")), json!(6));
        assert_eq!(bytes_to_gb(Some("[not set]")), Value::Null);
    }

    #[test]
    fn name_validators() {
        assert!(valid_snapshot("TheIsland_2026 01.tar.gz"));
        assert!(!valid_snapshot("../x.tar.gz"));
        assert!(!valid_snapshot("x.zip"));
        assert!(ok_cron("0 */6 * * 1-5"));
        assert!(!ok_cron("0 0 * *"));
        assert!(num_re_ok("-1.5") && !num_re_ok("1.") && !num_re_ok("x"));
    }
}
