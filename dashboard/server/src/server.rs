//! State, poller, leak guard and HTTP routes (`server.js`, `auth.js`).
//!
//! Routing is done by hand in [`dispatch`] so the behavior of Express stays visible: auth first, JSON
//! body parsing for every request, case-insensitive routes with an optional trailing slash, then the
//! static frontend, then the single-page-app fallback, then the default 404 page.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::ark::{self, ArkInstance, DiscoveredMap};
use crate::config::{Config, OFFICIAL_MAPS};
use crate::exec::{Exec, ShellOpts};
use crate::util::{
    emblem_for, js_number, js_round, js_str, js_str_opt, lenient_base64, now_ms, num, parse_feed,
    parse_int, parse_systemd_ts, suggest_ports, to_fixed, truthy, uptime_str,
};

const CATALOG: &str = include_str!("catalog.json");

#[derive(Clone)]
enum Rsc {
    Unset,
    Null,
    Val(Value),
}

struct Cache {
    updated_at: f64,
    connected: bool,
    error: Option<String>,
    host: Option<Map<String, Value>>,
    instances: Vec<(String, Value)>,
    hist: HashMap<String, (Vec<f64>, Vec<f64>)>,
    feed: Vec<Value>,
    disk_used: Option<f64>,
    disk_total: Option<f64>,
    rsc: HashMap<String, Rsc>,
}

struct Inner {
    arks: Vec<(String, Arc<ArkInstance>)>,
    discovered: Vec<DiscoveredMap>,
    cache: Cache,
    audit: Vec<Value>,
    guard: HashMap<String, f64>,
    cycle: u64,
}

pub struct AppState {
    pub cfg: Config,
    pub exec: Arc<dyn Exec>,
    pub udm: Option<crate::unifi::Unifi>,
    throttle: crate::security::Throttle,
    inner: Mutex<Inner>,
}

impl AppState {
    pub fn new(cfg: Config, exec: Arc<dyn Exec>) -> Arc<Self> {
        Self::with_udm(cfg, exec, None)
    }

    pub fn with_udm(
        cfg: Config,
        exec: Arc<dyn Exec>,
        udm: Option<crate::unifi::Unifi>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            exec,
            udm,
            throttle: crate::security::Throttle::standard(),
            inner: Mutex::new(Inner {
                arks: Vec::new(),
                discovered: Vec::new(),
                cache: Cache {
                    updated_at: 0.0,
                    connected: false,
                    error: None,
                    host: None,
                    instances: Vec::new(),
                    hist: HashMap::new(),
                    feed: Vec::new(),
                    disk_used: None,
                    disk_total: None,
                    rsc: HashMap::new(),
                },
                audit: Vec::new(),
                guard: HashMap::new(),
                cycle: 0,
            }),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.inner.lock().expect("state lock"))
    }

    fn ark(&self, id: &str) -> Option<Arc<ArkInstance>> {
        self.with(|i| i.arks.iter().find(|(k, _)| k == id).map(|(_, a)| a.clone()))
    }

    pub fn log_audit(&self, who: &str, action: &str, target: &str, danger: bool) {
        self.with(|i| {
            i.audit.insert(0, json!({"who": who, "action": action, "target": target, "danger": danger, "ts": num(now_ms())}));
            if i.audit.len() > 60 {
                i.audit.pop();
            }
        });
    }

    /// Re-scan the maps on the VM; keep existing instances (and their caches), add new, drop removed.
    pub async fn sync_maps(&self) {
        let found = match ark::discover_maps(&self.exec, &self.cfg.ark).await {
            Ok(f) => f,
            Err(e) => {
                // like the original: keep what we had, the next cycle tries again
                eprintln!("[syncMaps] {e}");
                return;
            }
        };
        let mut keep: Vec<String> = Vec::new();
        let mut fresh: Vec<(String, Arc<ArkInstance>)> = Vec::new();
        for m in &found {
            match ArkInstance::new(self.exec.clone(), &self.cfg.ark, m) {
                Ok(inst) => {
                    keep.push(inst.id.clone());
                    fresh.push((inst.id.clone(), Arc::new(inst)));
                }
                Err(e) => eprintln!("[syncMaps] map overgeslagen ({}): {e}", m.display),
            }
        }
        self.with(|i| {
            i.discovered = found;
            for (id, inst) in fresh {
                if !i.arks.iter().any(|(k, _)| *k == id) {
                    i.cache.hist.insert(id.clone(), (Vec::new(), Vec::new()));
                    i.arks.push((id, inst));
                }
            }
            let gone: Vec<String> = i
                .arks
                .iter()
                .map(|(k, _)| k.clone())
                .filter(|k| !keep.contains(k))
                .collect();
            for id in gone {
                i.arks.retain(|(k, _)| *k != id);
                i.cache.instances.retain(|(k, _)| *k != id);
            }
        });
    }

    fn vm_budget(&self) -> (f64, f64) {
        self.with(|i| {
            let host = i.cache.host.as_ref();
            let gb = host
                .and_then(|h| h.get("memTotal"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let cpus = host
                .and_then(|h| h.get("cpus"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let base = if cpus != 0.0 {
                cpus
            } else if self.cfg.ark.cores_total != 0 {
                self.cfg.ark.cores_total as f64
            } else {
                4.0
            };
            let vm_cores = 1f64.max(js_round(base));
            let vm_ram = if gb != 0.0 {
                1f64.max(((gb * 1e9) / 1_073_741_824.0).floor())
            } else if self.cfg.ark.ram_alloc_gb != 0 {
                self.cfg.ark.ram_alloc_gb as f64
            } else {
                16.0
            };
            (vm_cores, vm_ram)
        })
    }

    /// One poll cycle: host metrics, every world's status, players, config summary and the leak guard.
    pub async fn refresh(&self) {
        let cycle_before = self.with(|i| i.cycle);
        if cycle_before.is_multiple_of(self.cfg.discover_every_n) {
            self.sync_maps().await;
            let dr = self
                .exec
                .guest_shell("df -B1 / 2>/dev/null | tail -1", ShellOpts::default())
                .await
                .unwrap_or_default();
            let parts: Vec<&str> = dr.out.split_whitespace().collect();
            if parts.len() >= 4 {
                let used = parse_int(parts[2]).map(|n| to_fixed(n as f64 / 1e9, 0));
                let total = parse_int(parts[1]).map(|n| to_fixed(n as f64 / 1e9, 0));
                self.with(|i| {
                    if used.is_some() {
                        i.cache.disk_used = used;
                    }
                    if total.is_some() {
                        i.cache.disk_total = total;
                    }
                });
            }
        }
        let cycle = self.with(|i| {
            i.cycle += 1;
            i.cycle
        });

        let status_cur = self.exec.vm_status().await.ok();
        let rrd = self.exec.rrddata().await.unwrap_or_default();
        let last = rrd
            .iter()
            .rfind(|p| !p.get("cpu").is_none_or(Value::is_null))
            .cloned()
            .unwrap_or(json!({}));
        let f = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let or0 = |a: f64, b: f64| if a != 0.0 { a } else { b };
        let host: Option<Map<String, Value>> = status_cur.as_ref().map(|s| {
            let mut h = Map::new();
            h.insert(
                "name".into(),
                s.get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .unwrap_or("ARK")
                    .into(),
            );
            h.insert(
                "vmStatus".into(),
                s.get("status").cloned().unwrap_or(Value::Null),
            );
            let cpus = or0(
                f(s, "cpus"),
                if self.cfg.ark.cores_total != 0 {
                    self.cfg.ark.cores_total as f64
                } else {
                    4.0
                },
            );
            h.insert("cpus".into(), num(js_round(cpus)));
            h.insert(
                "cpuPct".into(),
                num(js_round(or0(f(s, "cpu"), f(&last, "cpu")) * 100.0)),
            );
            h.insert(
                "memUsed".into(),
                num(to_fixed(or0(f(s, "mem"), f(&last, "mem")) / 1e9, 1)),
            );
            h.insert(
                "memTotal".into(),
                num(to_fixed(or0(f(s, "maxmem"), f(&last, "maxmem")) / 1e9, 1)),
            );
            let (du, dt) = self.with(|i| (i.cache.disk_used, i.cache.disk_total));
            h.insert(
                "diskUsed".into(),
                match du {
                    Some(v) => num(v),
                    None => {
                        if f(s, "disk") != 0.0 {
                            num(to_fixed(f(s, "disk") / 1e9, 0))
                        } else {
                            Value::Null
                        }
                    }
                },
            );
            h.insert(
                "diskTotal".into(),
                match dt {
                    Some(v) => num(v),
                    None => {
                        if f(s, "maxdisk") != 0.0 {
                            num(to_fixed(f(s, "maxdisk") / 1e9, 0))
                        } else {
                            Value::Null
                        }
                    }
                },
            );
            h.insert("uptimeS".into(), num(f(s, "uptime")));
            h.insert("netin".into(), num(f(&last, "netin")));
            h.insert("netout".into(), num(f(&last, "netout")));
            h
        });
        self.with(|i| i.cache.host = host.clone());
        let host_cpu = host
            .as_ref()
            .and_then(|h| h.get("cpuPct"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let host_mem = host
            .as_ref()
            .and_then(|h| h.get("memUsed"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let host_up = host
            .as_ref()
            .and_then(|h| h.get("uptimeS"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);

        let arks: Vec<(String, Arc<ArkInstance>)> = self.with(|i| i.arks.clone());
        let mut any_feed: Option<Vec<Value>> = None;
        for (id, ark) in &arks {
            let st = ark.status().await.unwrap_or_else(|_| ark::Status {
                state: String::new(),
                dash: "unknown".into(),
                start_ts: String::new(),
            });
            let cfg_summary = ark.config_summary().await.unwrap_or(Value::Null);
            let mut players: Vec<Value> = Vec::new();
            if st.dash == "running" {
                players = ark
                    .players()
                    .await
                    .unwrap_or_else(|_| json!({"players": []}))
                    .get("players")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let lg = ark
                    .log_tail("150")
                    .await
                    .unwrap_or_else(|_| json!({"ok": false, "lines": []}));
                if truthy(lg.get("ok")) && any_feed.is_none() {
                    let lines: Vec<String> = lg
                        .get("lines")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|l| l.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    any_feed = Some(parse_feed(&lines, &ark.name));
                }
            }
            let running = st.dash == "running" || st.dash == "starting";
            let cpu_pct = if running && host.is_some() {
                host_cpu
            } else {
                0.0
            };
            let ram_used = if running && host.is_some() {
                host_mem
            } else {
                0.0
            };
            let ram_alloc = if ark.ram_alloc != 0 {
                ark.ram_alloc as f64
            } else {
                16.0
            };
            let hist_ram = if host.is_some() {
                js_round(ram_used / ram_alloc * 100.0)
            } else {
                0.0
            };
            self.with(|i| {
                let h = i.cache.hist.entry(id.clone()).or_default();
                h.0.push(cpu_pct);
                h.1.push(hist_ram);
                if h.0.len() > self.cfg.hist_len {
                    h.0.remove(0);
                }
                if h.1.len() > self.cfg.hist_len {
                    h.1.remove(0);
                }
            });
            let mut svc_uptime_s = 0.0;
            if !st.start_ts.is_empty() {
                let stripped = match st.start_ts.split_once(' ') {
                    Some((w, rest)) if w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                        rest
                    }
                    _ => st.start_ts.as_str(),
                };
                if let Some(t) = parse_systemd_ts(stripped) {
                    svc_uptime_s = ((now_ms() - t) / 1000.0).floor().max(0.0);
                }
            }
            let need_rsc = self.with(|i| {
                matches!(
                    i.cache.rsc.get(id).cloned().unwrap_or(Rsc::Unset),
                    Rsc::Unset
                )
            }) || cycle.is_multiple_of(self.cfg.discover_every_n);
            if need_rsc {
                let r = ark.resources().await.map_or(Rsc::Null, Rsc::Val);
                self.with(|i| i.cache.rsc.insert(id.clone(), r));
            }
            let rsc = self.with(|i| i.cache.rsc.get(id).cloned().unwrap_or(Rsc::Null));
            let rv = match &rsc {
                Rsc::Val(v) => Some(v.clone()),
                _ => None,
            };
            let rget = |k: &str| {
                rv.as_ref()
                    .and_then(|v| v.get(k))
                    .filter(|x| !x.is_null())
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let mode = cfg_summary
                .get("rules")
                .and_then(|r| r.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("PvE")
                .to_string();
            let uptime = if st.dash == "running" {
                uptime_str(if svc_uptime_s != 0.0 {
                    svc_uptime_s
                } else {
                    host_up
                })
            } else if st.dash == "starting" {
                "booting".to_string()
            } else {
                "\u{2014}".to_string()
            };
            let last_ts = self
                .with(|i| i.guard.get(id).copied())
                .filter(|t| *t != 0.0);
            let mut ports = Map::new();
            if let Some(g) = &ark.game_port {
                ports.insert("game".into(), g.clone().into());
            }
            if let Some(q) = &ark.query_port {
                ports.insert("query".into(), q.clone().into());
            }
            ports.insert("rcon".into(), ark.rcon_port.into());
            let instance = json!({
                "id": id, "name": ark.name, "code": ark.display.chars().take(3).collect::<String>().to_uppercase(), "map": ark.map,
                "display": ark.display, "emblem": emblem_for(&ark.display), "gamemode": mode, "status": st.dash,
                "players": players.len(), "playerList": players, "max": ark.max_players, "cpuPct": num(cpu_pct), "ramUsed": num(ram_used),
                "ramAlloc": ark.ram_alloc, "uptime": uptime, "online": st.dash == "running", "provisioned": true,
                "config": cfg_summary, "ports": Value::Object(ports),
                "coresAlloc": rget("cores"), "ramLimitGB": rget("ramGB"), "ramLiveGB": rget("ramUsedGB"),
                "leakGuard": {"enabled": self.cfg.leak_guard.enabled, "ramGB": num(self.cfg.leak_guard.ram_gb), "lastTs": last_ts.map_or(Value::Null, num)},
            });
            self.with(|i| {
                if let Some(slot) = i.cache.instances.iter_mut().find(|(k, _)| k == id) {
                    slot.1 = instance;
                } else {
                    i.cache.instances.push((id.clone(), instance));
                }
            });
            let live = rv
                .as_ref()
                .and_then(|v| v.get("ramUsedGB"))
                .and_then(Value::as_f64);
            self.leak_guard_check(id, ark, &st.dash, players.len(), svc_uptime_s, live)
                .await;
        }
        self.with(|i| {
            if let Some(f) = any_feed {
                i.cache.feed = f;
            }
            i.cache.connected = true;
            i.cache.error = None;
            i.cache.updated_at = now_ms();
        });
    }

    /// ASA leaks memory over uptime: with no players online and RAM over the ceiling, save and restart that world.
    async fn leak_guard_check(
        &self,
        id: &str,
        ark: &Arc<ArkInstance>,
        dash: &str,
        players: usize,
        svc_uptime_s: f64,
        ram_live_gb: Option<f64>,
    ) {
        let g = &self.cfg.leak_guard;
        if !g.enabled || dash != "running" {
            return;
        }
        let Some(live) = ram_live_gb else { return };
        if live < g.ram_gb
            || players > 0
            || svc_uptime_s == 0.0
            || svc_uptime_s < g.min_uptime_h * 3600.0
        {
            return;
        }
        let last = self.with(|i| i.guard.get(id).copied()).unwrap_or(0.0);
        if now_ms() - last < g.cooldown_h * 3600e3 {
            return;
        }
        self.with(|i| i.guard.insert(id.to_string(), now_ms()));
        self.log_audit(
            "leak-guard",
            &format!(
                "auto save+restart \u{2014} RAM {} GB \u{2265} {} GB, 0 players, up {}",
                crate::util::num_str(live),
                crate::util::num_str(g.ram_gb),
                uptime_str(svc_uptime_s)
            ),
            &ark.name,
            true,
        );
        let _ = ark.rcon("SaveWorld").await;
        if let Err(e) = ark.service_action("restart").await {
            eprintln!("[leak-guard] {e}");
            return;
        }
        self.with(|i| {
            i.cache.rsc.insert(id.to_string(), Rsc::Unset);
            let item = json!({"who": "Leak-guard", "action": format!("saved the world + restarted (RAM {} GB, no players online)", crate::util::num_str(live)), "map": ark.name, "type": "sys"});
            let mut feed = vec![item];
            feed.extend(i.cache.feed.iter().cloned());
            feed.truncate(8);
            i.cache.feed = feed;
        });
    }
}

// ----------------------------------------------------------------------------------------------
// HTTP plumbing

fn resp(status: u16, content_type: &str, body: impl Into<Body>) -> Response {
    let mut r = Response::new(body.into());
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).expect("content type"),
    );
    r
}

fn jr(status: u16, v: &Value) -> Response {
    resp(
        status,
        "application/json; charset=utf-8",
        serde_json::to_string(v).unwrap_or_else(|_| "null".into()),
    )
}

/// 200 with the value, or the original's `catch`: 500 `{ok:false, error, ...extra}`.
fn done(r: Result<Value, String>, extra: Value) -> Response {
    match r {
        Ok(v) => jr(200, &v),
        Err(e) => {
            let mut o = json!({"ok": false, "error": e});
            if let (Some(m), Some(x)) = (o.as_object_mut(), extra.as_object()) {
                for (k, v) in x {
                    m.insert(k.clone(), v.clone());
                }
            }
            jr(500, &o)
        }
    }
}

fn fail(e: &str) -> Response {
    jr(500, &json!({"ok": false, "error": e}))
}

fn text(status: u16, s: &str) -> Response {
    resp(status, "text/plain; charset=utf-8", s.to_string())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Express' default error page.
fn error_page(status: u16, message: &str) -> Response {
    let body = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{}</pre>\n</body>\n</html>\n",
        html_escape(message)
    );
    let mut r = resp(status, "text/html; charset=utf-8", body);
    r.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'"),
    );
    r.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    r
}

fn unauthorized(msg: &str) -> Response {
    let mut r = text(401, msg);
    r.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"ARK dashboard\", charset=\"UTF-8\""),
    );
    r
}

fn safe_equal(a: &str, b: &str) -> bool {
    let ha = Sha256::digest(a.as_bytes());
    let hb = Sha256::digest(b.as_bytes());
    ha.iter()
        .zip(hb.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// HTTP Basic auth for the whole dashboard; fail-closed without a password (`auth.js`).
pub fn check_auth(cfg: &Config, headers: &HeaderMap) -> Option<Response> {
    if !cfg.auth.enabled {
        return None;
    }
    if cfg.auth.pass.is_empty() {
        return Some(text(
            503,
            "Dashboard is not configured: DASH_PASS is empty.\nSet DASH_USER and DASH_PASS in the environment (deploy/.env) and restart.\n",
        ));
    }
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut parts = header.split(' ');
    let scheme = parts.next().unwrap_or("");
    let encoded = parts.next().unwrap_or("");
    if encoded.is_empty() || !scheme.eq_ignore_ascii_case("basic") {
        return Some(unauthorized("Authentication required.\n"));
    }
    let decoded = String::from_utf8_lossy(&lenient_base64(encoded)).into_owned();
    let (got_user, got_pass) = match decoded.find(':') {
        Some(i) => (&decoded[..i], &decoded[i + 1..]),
        None => ("", ""),
    };
    let ok_user = safe_equal(got_user, &cfg.auth.user);
    let ok_pass = safe_equal(got_pass, &cfg.auth.pass);
    if ok_user && ok_pass {
        None
    } else {
        Some(unauthorized("Invalid credentials.\n"))
    }
}

fn parse_query(q: Option<&str>) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for pair in q.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let dec = |s: &str| {
            percent_encoding::percent_decode_str(&s.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        };
        m.entry(dec(k)).or_insert_with(|| dec(v));
    }
    m
}

/// `express.json()`: only `application/json`, only objects and arrays; everything else is `{}`.
fn parse_body(headers: &HeaderMap, bytes: &Bytes) -> Result<Map<String, Value>, Box<Response>> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let is_json = ct
        .split(';')
        .next()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"));
    if !is_json || bytes.is_empty() {
        return Ok(Map::new());
    }
    let s = String::from_utf8_lossy(bytes);
    let first = s.trim_start().chars().next();
    if !matches!(first, Some('{' | '[')) {
        return Err(Box::new(error_page(
            400,
            &format!(
                "SyntaxError: Unexpected token '{}', \"{}\" is not valid JSON",
                first.unwrap_or(' '),
                s.chars().take(20).collect::<String>()
            ),
        )));
    }
    match serde_json::from_str::<Value>(&s) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Ok(Map::new()),
        Err(e) => Err(Box::new(error_page(400, &format!("SyntaxError: {e}")))),
    }
}

fn id_key(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(_)) => Some(js_str(v.expect("some"))),
        _ => None,
    }
}

// ----------------------------------------------------------------------------------------------
// Routes

struct Req<'a> {
    method: &'a Method,
    segs: Vec<String>,
    query: HashMap<String, String>,
    body: Map<String, Value>,
}

impl Req<'_> {
    fn b(&self, k: &str) -> Option<&Value> {
        self.body.get(k)
    }
}

fn seg_is(s: &str, lit: &str) -> bool {
    s.eq_ignore_ascii_case(lit)
}

async fn route(st: &Arc<AppState>, r: &Req<'_>) -> Option<Response> {
    let s: Vec<&str> = r.segs.iter().map(String::as_str).collect();
    let m = r.method;
    let get = m == Method::GET || m == Method::HEAD;
    if s.is_empty() || !seg_is(s[0], "api") {
        return None;
    }
    match (s.len(), s.get(1).copied()) {
        (2, Some(x)) if seg_is(x, "health") && get => {
            return Some(st.with(|i| {
                jr(200, &json!({"ok": true, "connected": i.cache.connected, "updatedAt": num(i.cache.updated_at), "error": i.cache.error, "maps": i.arks.len()}))
            }));
        }
        (2, Some(x)) if seg_is(x, "state") && get => return Some(state_json(st)),
        (2, Some(x)) if seg_is(x, "rcon") && *m == Method::POST => {
            return Some(rcon_route(st, r).await);
        }
        (2, Some(x)) if seg_is(x, "config-catalog") && get => {
            let cat: Value = serde_json::from_str(CATALOG).unwrap_or(Value::Null);
            return Some(jr(200, &json!({"ok": true, "catalog": cat})));
        }
        (2, Some(x)) if seg_is(x, "audit") && get => {
            return Some(st.with(|i| jr(200, &json!({"ok": true, "audit": i.audit}))));
        }
        (2, Some(x)) if seg_is(x, "maps") && *m == Method::POST => {
            return Some(create_map_route(st, r).await);
        }
        (3, Some(x)) if seg_is(x, "maps") => {
            if seg_is(s[2], "available") && get {
                return Some(st.with(|i| {
                    let existing: Vec<String> = i.discovered.iter().map(|d| d.display.to_lowercase()).collect();
                    let available: Vec<Value> = OFFICIAL_MAPS
                        .iter()
                        .filter(|(d, _)| !existing.contains(&d.to_lowercase()))
                        .map(|(d, n)| json!({"display": d, "internal": n}))
                        .collect();
                    let maps: Vec<Map<String, Value>> = i.discovered.iter().map(DiscoveredMap::to_json).collect();
                    jr(200, &json!({"ok": true, "available": available, "ports": suggest_ports(&maps), "existing": i.discovered.iter().map(|d| d.display.clone()).collect::<Vec<_>>()}))
                }));
            }
            if *m == Method::DELETE {
                return Some(delete_map_route(st, &r.segs[2]).await);
            }
        }
        (3, Some(x)) if seg_is(x, "instance") && get => {
            return Some(st.with(|i| match i.cache.instances.iter().find(|(k, _)| *k == r.segs[2]) {
                None => jr(404, &json!({"error": "unknown instance"})),
                Some((id, c)) => {
                    let mut o = c.as_object().cloned().unwrap_or_default();
                    if let Some((c1, c2)) = i.cache.hist.get(id) {
                        o.insert("hist".into(), json!({"cpu": c1.iter().map(|x| num(*x)).collect::<Vec<_>>(), "ram": c2.iter().map(|x| num(*x)).collect::<Vec<_>>()}));
                    }
                    jr(200, &Value::Object(o))
                }
            }));
        }
        (4, Some(x)) if seg_is(x, "instance") => {
            return instance_route(st, r, &r.segs[2], s[3]).await;
        }
        (3, Some(x)) if seg_is(x, "console") && get => {
            return Some(console_route(st, r, &r.segs[2]).await);
        }
        (3, Some(x)) if seg_is(x, "config") && get => {
            let Some(a) = st.ark(&r.segs[2]) else {
                return Some(jr(
                    200,
                    &json!({"ok": false, "error": "not found", "ini": ""}),
                ));
            };
            let which = if r.query.get("file").is_some_and(|f| f == "game") {
                "game"
            } else {
                "gus"
            };
            return Some(done(a.read_config(which).await, json!({"ini": ""})));
        }
        (4, Some(x)) if seg_is(x, "config") && *m == Method::POST => {
            return config_post(st, r, &r.segs[2], s[3]).await;
        }
        (3, Some(x)) if seg_is(x, "backups") => return backups_route(st, r, &r.segs[2]).await,
        (4, Some(x)) if seg_is(x, "backups") && *m == Method::POST => {
            return backups_post(st, r, &r.segs[2], s[3]).await;
        }
        (3, Some(x)) if seg_is(x, "schedule") => return schedule_route(st, r, &r.segs[2]).await,
        (3, Some(x)) if seg_is(x, "update") && get => {
            let Some(a) = st.ark(&r.segs[2]) else {
                return Some(jr(200, &json!({"ok": false, "error": "not found"})));
            };
            return Some(done(a.update_check().await, json!({})));
        }
        (4, Some(x)) if seg_is(x, "update") && *m == Method::POST && seg_is(s[3], "apply") => {
            let Some(a) = st.ark(&r.segs[2]) else {
                return Some(jr(400, &json!({"ok": false, "error": "not found"})));
            };
            st.log_audit("admin", "Apply update + restart", &a.name, true);
            return Some(done(a.apply_update().await, json!({})));
        }
        _ => {}
    }
    None
}

fn state_json(st: &Arc<AppState>) -> Response {
    st.with(|i| {
        let instances: Vec<Value> = i
            .cache
            .instances
            .iter()
            .map(|(id, c)| {
                let g = |k: &str| c.get(k).cloned().unwrap_or(Value::Null);
                let or = |k: &str, d: Value| if truthy(c.get(k)) { g(k) } else { d };
                let hist = i.cache.hist.get(id).map_or(Value::Null, |(a, b)| json!({"cpu": a.iter().map(|x| num(*x)).collect::<Vec<_>>(), "ram": b.iter().map(|x| num(*x)).collect::<Vec<_>>()}));
                json!({
                    "id": g("id"), "name": g("name"), "code": g("code"), "map": g("map"), "emblem": g("emblem"), "gamemode": g("gamemode"),
                    "provisioned": true, "status": g("status"), "players": or("players", json!(0)), "max": g("max"),
                    "cpuPct": or("cpuPct", json!(0)), "ramUsed": or("ramUsed", json!(0)), "ramAlloc": g("ramAlloc"), "uptime": or("uptime", json!("\u{2014}")),
                    "coresAlloc": g("coresAlloc"), "ramLimitGB": g("ramLimitGB"), "ramLiveGB": g("ramLiveGB"),
                    "config": or("config", Value::Null), "ports": g("ports"), "hist": hist, "leakGuard": or("leakGuard", Value::Null),
                })
            })
            .collect();
        jr(
            200,
            &json!({
                "updatedAt": num(i.cache.updated_at), "connected": i.cache.connected, "error": i.cache.error,
                "host": i.cache.host.clone().map_or(Value::Null, Value::Object), "instances": instances, "feed": i.cache.feed,
            }),
        )
    })
}

async fn rcon_route(st: &Arc<AppState>, r: &Req<'_>) -> Response {
    let ark = id_key(r.b("id")).and_then(|id| st.ark(&id));
    let Some(a) = ark else {
        return jr(400, &json!({"ok": false, "error": "instance not found"}));
    };
    let Some(command) = r
        .b("command")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    else {
        return jr(400, &json!({"ok": false, "error": "command required"}));
    };
    let res = match a.rcon(command).await {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let lc = command.to_ascii_lowercase();
    let danger = ["doexit", "destroy", "kick", "ban"]
        .iter()
        .any(|k| lc.contains(k));
    st.log_audit("admin", command, &a.name, danger);
    jr(200, &res)
}

async fn instance_route(st: &Arc<AppState>, r: &Req<'_>, id: &str, what: &str) -> Option<Response> {
    let post = *r.method == Method::POST;
    let get = *r.method == Method::GET || *r.method == Method::HEAD;
    let ark = st.ark(id);
    if post && seg_is(what, "action") {
        let action = r.b("action");
        let act = action.and_then(Value::as_str).unwrap_or("");
        if ["start", "stop", "restart"].contains(&act) {
            let Some(a) = ark else {
                return Some(jr(400, &json!({"ok": false, "error": "not found"})));
            };
            st.log_audit("admin", act, &a.name, act != "start");
            return Some(match a.service_action(act).await {
                Ok(v) => jr(200, &v),
                Err(e) => jr(500, &json!({"ok": false, "error": e})),
            });
        }
        if ["vmreboot", "vmshutdown", "vmstart", "vmstop"].contains(&act) {
            let mapped = match act {
                "vmstart" => "start",
                "vmstop" => "stop",
                "vmreboot" => "reboot",
                _ => "shutdown",
            };
            return Some(match st.exec.vm_power(mapped).await {
                Err(e) => jr(500, &json!({"ok": false, "error": e})),
                Ok(()) => {
                    st.log_audit(
                        "admin",
                        &format!("VM {mapped}"),
                        &format!("VM {}", st.cfg.pve.vmid),
                        true,
                    );
                    jr(
                        200,
                        &json!({"ok": true, "output": format!("VM {mapped} requested")}),
                    )
                }
            });
        }
        if act == "save"
            && let Some(a) = ark
        {
            st.log_audit("admin", "SaveWorld", &a.name, false);
            return Some(done(a.rcon("SaveWorld").await, json!({})));
        }
        return Some(jr(400, &json!({"ok": false, "error": "unknown action"})));
    }
    if post && seg_is(what, "settings") {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a
            .update_settings(
                r.b("sessionName"),
                r.b("adminPassword"),
                r.b("serverPassword"),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            let changed = res
                .get("changed")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(js_str).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            st.log_audit(
                "admin",
                &format!("Update world settings ({changed})"),
                &a.name,
                true,
            );
        }
        return Some(jr(200, &res));
    }
    if seg_is(what, "resources") && get {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a.resources().await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        let (vc, vr) = st.vm_budget();
        let mut o = res.as_object().cloned().unwrap_or_default();
        o.insert("vmCores".into(), num(vc));
        o.insert("vmRam".into(), num(vr));
        return Some(jr(200, &Value::Object(o)));
    }
    if seg_is(what, "resources") && post {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let (vc, vr) = st.vm_budget();
        let res = match a.set_resources(r.b("cores"), r.b("ramGB"), vc, vr).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            st.log_audit(
                "admin",
                &format!(
                    "set compute {} vCPU / {} GB",
                    js_str(&res["cores"]),
                    js_str(&res["ramGB"])
                ),
                &a.name,
                true,
            );
            st.with(|i| i.cache.rsc.insert(id.to_string(), Rsc::Unset));
        }
        return Some(jr(200, &res));
    }
    None
}

async fn console_route(st: &Arc<AppState>, r: &Req<'_>, id: &str) -> Response {
    let Some(a) = st.ark(id) else {
        return jr(
            200,
            &json!({"ok": false, "error": "not found", "lines": []}),
        );
    };
    let n = match r.query.get("n").filter(|n| !n.is_empty()) {
        Some(n) => parse_int(n).map_or_else(|| "NaN".to_string(), |v| v.to_string()),
        None => "200".to_string(),
    };
    done(a.log_tail(&n).await, json!({"lines": []}))
}

async fn config_post(st: &Arc<AppState>, r: &Req<'_>, id: &str, what: &str) -> Option<Response> {
    let ark = st.ark(id);
    let file = r.b("file").and_then(Value::as_str);
    let which = if file == Some("game") { "game" } else { "gus" };
    if seg_is(what, "set") {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let key = js_str_opt(r.b("key"));
        let value = js_str_opt(r.b("value"));
        let res = match a.set_config_key(&key, &value, which).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        st.log_audit("admin", &format!("set {key}={value}"), &a.name, false);
        return Some(jr(200, &res));
    }
    if seg_is(what, "raw") {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let content = match r.b("content") {
            None | Some(Value::Null) => {
                return Some(jr(400, &json!({"ok": false, "error": "content required"})));
            }
            Some(c) => js_str(c),
        };
        let res = match a.write_config_raw(which, &content).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            st.log_audit(
                "admin",
                if file == Some("game") {
                    "Edit Game.ini"
                } else {
                    "Edit GameUserSettings.ini"
                },
                &a.name,
                false,
            );
        }
        return Some(jr(200, &res));
    }
    if seg_is(what, "platforms") {
        let list = if truthy(r.b("list")) {
            r.b("list").cloned().unwrap_or(json!([]))
        } else {
            json!([])
        };
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a.set_platforms(&list).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        let label = match res.get("platform") {
            Some(p) if truthy(Some(p)) => js_str(p),
            _ => match &list {
                Value::Array(a) => a
                    .iter()
                    .map(|x| {
                        if x.is_null() {
                            String::new()
                        } else {
                            js_str(x)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("+"),
                _ => String::new(),
            },
        };
        st.log_audit("admin", &format!("platforms {label}"), &a.name, false);
        return Some(jr(200, &res));
    }
    None
}

async fn backups_route(st: &Arc<AppState>, r: &Req<'_>, id: &str) -> Option<Response> {
    let ark = st.ark(id);
    let get = *r.method == Method::GET || *r.method == Method::HEAD;
    if get {
        let Some(a) = ark else {
            return Some(jr(200, &json!({"ok": true, "backups": []})));
        };
        return Some(done(a.backups_list().await, json!({"backups": []})));
    }
    if *r.method == Method::DELETE {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        if !truthy(r.b("name")) {
            return Some(jr(
                400,
                &json!({"ok": false, "error": "snapshot name required"}),
            ));
        }
        let name = js_str_opt(r.b("name"));
        let res = match a.delete_backup(&name).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            st.log_audit("admin", &format!("Delete snapshot {name}"), &a.name, true);
        }
        return Some(jr(200, &res));
    }
    None
}

async fn backups_post(st: &Arc<AppState>, r: &Req<'_>, id: &str, what: &str) -> Option<Response> {
    let ark = st.ark(id);
    if seg_is(what, "create") {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a.create_backup().await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        st.log_audit("admin", "Backup snapshot", &a.name, false);
        return Some(jr(200, &res));
    }
    if seg_is(what, "restore") {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        if !truthy(r.b("name")) {
            return Some(jr(
                400,
                &json!({"ok": false, "error": "snapshot name required"}),
            ));
        }
        let name = js_str_opt(r.b("name"));
        st.log_audit("admin", &format!("Restore snapshot {name}"), &a.name, true);
        return Some(done(a.restore_backup(&name).await, json!({})));
    }
    None
}

async fn schedule_route(st: &Arc<AppState>, r: &Req<'_>, id: &str) -> Option<Response> {
    let ark = st.ark(id);
    let get = *r.method == Method::GET || *r.method == Method::HEAD;
    if get {
        let Some(a) = ark else {
            return Some(jr(200, &json!({"ok": true, "jobs": []})));
        };
        return Some(done(a.schedule().await, json!({"jobs": []})));
    }
    if *r.method == Method::POST {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a
            .add_schedule(r.b("taskType"), r.b("cron"), r.b("message"), &r.body)
            .await
        {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            st.log_audit(
                "admin",
                &format!(
                    "Add schedule {} ({})",
                    js_str_opt(r.b("taskType")),
                    js_str_opt(r.b("cron"))
                ),
                &a.name,
                false,
            );
        }
        return Some(jr(200, &res));
    }
    if *r.method == Method::DELETE {
        let Some(a) = ark else {
            return Some(jr(400, &json!({"ok": false, "error": "not found"})));
        };
        let res = match a.delete_schedule(r.b("match")).await {
            Ok(v) => v,
            Err(e) => return Some(fail(&e)),
        };
        if truthy(res.get("ok")) {
            st.log_audit("admin", "Remove schedule", &a.name, true);
        }
        return Some(jr(200, &res));
    }
    None
}

async fn create_map_route(st: &Arc<AppState>, r: &Req<'_>) -> Response {
    if !truthy(r.b("display")) || !truthy(r.b("internal")) {
        return jr(400, &json!({"ok": false, "error": "map name required"}));
    }
    let display = js_str_opt(r.b("display"));
    let mut res = match ark::create_map(&st.exec, &st.cfg.ark, &r.body).await {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if truthy(res.get("ok")) {
        st.log_audit("admin", &format!("Created map {display}"), &display, false);
        // Open the internet-facing game port on the UDM (RCON and query stay LAN-only).
        if let Some(udm) = &st.udm
            && truthy(r.b("gamePort"))
        {
            let game_port = js_str_opt(r.b("gamePort"));
            let wan = parse_int(&game_port);
            let (ip, proto) = (&st.cfg.unifi.ark_vm_ip, &st.cfg.unifi.proto);
            let fwd = crate::unifi::Forward {
                name: &format!("ARK-{display}"),
                wan_port: wan,
                fwd_ip: ip,
                proto,
            };
            let forward = match udm.upsert_forward(&fwd).await {
                Ok(()) => {
                    st.log_audit(
                        "admin",
                        &format!("Opened WAN {game_port}/{proto} \u{2192} {ip}"),
                        &display,
                        false,
                    );
                    json!({"ok": true, "wanPort": wan.map_or(Value::Null, |w| json!(w))})
                }
                Err(e) => {
                    eprintln!("[unifi create] {e}");
                    json!({"ok": false, "error": e})
                }
            };
            if let Some(o) = res.as_object_mut() {
                o.insert("forward".into(), forward);
            }
        }
        st.sync_maps().await;
        let cores = js_number(r.b("cores"));
        let ram = js_number(r.b("ramGB"));
        if ((cores.is_finite() && cores > 0.0) || (ram.is_finite() && ram > 0.0))
            && let Some(a) = st.ark(&display.to_lowercase())
        {
            let (vc, vr) = st.vm_budget();
            let rr = a
                .set_resources(r.b("cores"), r.b("ramGB"), vc, vr)
                .await
                .unwrap_or_else(|e| json!({"ok": false, "error": e}));
            if truthy(rr.get("ok")) {
                st.log_audit(
                    "admin",
                    &format!(
                        "set compute {} vCPU / {} GB",
                        js_str(&rr["cores"]),
                        js_str(&rr["ramGB"])
                    ),
                    &display,
                    false,
                );
            }
            if let Some(o) = res.as_object_mut() {
                o.insert("resources".into(), rr);
            }
        }
    }
    jr(200, &res)
}

async fn delete_map_route(st: &Arc<AppState>, display: &str) -> Response {
    let mut res = match ark::delete_map(&st.exec, &st.cfg.ark, display).await {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if truthy(res.get("ok")) {
        st.log_audit("admin", &format!("Deleted map {display}"), display, true);
        // Remove the map's WAN forward so nothing stays exposed after deletion.
        if let Some(udm) = &st.udm {
            let forward = match udm.delete_forward_by_name(&format!("ARK-{display}")).await {
                Ok(removed) => {
                    if removed > 0 {
                        st.log_audit(
                            "admin",
                            &format!("Closed WAN forward ({removed})"),
                            display,
                            true,
                        );
                    }
                    json!({"ok": true, "removed": removed})
                }
                Err(e) => {
                    eprintln!("[unifi delete] {e}");
                    json!({"ok": false, "error": e})
                }
            };
            if let Some(o) = res.as_object_mut() {
                o.insert("forward".into(), forward);
            }
        }
        st.sync_maps().await;
    }
    jr(200, &res)
}

// ----------------------------------------------------------------------------------------------
// Static frontend

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=UTF-8",
        Some("js" | "mjs") => "application/javascript; charset=UTF-8",
        Some("css") => "text/css; charset=UTF-8",
        Some("json" | "map") => "application/json; charset=UTF-8",
        Some("txt" | "sh") => "text/plain; charset=UTF-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

fn serve_file(path: &Path, vendor: bool, head: bool) -> Option<Response> {
    let bytes = std::fs::read(path).ok()?;
    let cache = if vendor {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let len = bytes.len();
    let mut r = resp(
        200,
        mime_for(path),
        if head {
            Body::empty()
        } else {
            Body::from(bytes)
        },
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r.headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    r.headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    Some(r)
}

/// `express.static` for the frontend: the file if it exists inside the root, otherwise `None` (falls through).
fn static_file(root: &Path, raw_path: &str, query: Option<&str>, head: bool) -> Option<Response> {
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .ok()?
        .into_owned();
    if decoded.contains('\0') {
        return None;
    }
    let rel = decoded.trim_start_matches('/');
    let mut path = PathBuf::from(root);
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.starts_with('.') => return None, // dotfiles are ignored
            p => path.push(p),
        }
    }
    if path.is_dir() {
        if !decoded.ends_with('/') && !rel.is_empty() {
            // express.static redirects a folder to its slash form
            let to = format!(
                "{raw_path}/{}",
                query.map(|q| format!("?{q}")).unwrap_or_default()
            );
            let body = format!(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Redirecting</title>\n</head>\n<body>\n<pre>Redirecting to {}</pre>\n</body>\n</html>\n",
                html_escape(&to)
            );
            let mut r = resp(301, "text/html; charset=UTF-8", body);
            r.headers_mut()
                .insert(header::LOCATION, HeaderValue::from_str(&to).ok()?);
            r.headers_mut().insert(
                "content-security-policy",
                HeaderValue::from_static("default-src 'none'"),
            );
            r.headers_mut().insert(
                "x-content-type-options",
                HeaderValue::from_static("nosniff"),
            );
            return Some(r);
        }
        path.push("index.html");
    }
    let canon = path.canonicalize().ok()?;
    if !canon.starts_with(root.canonicalize().ok()?) {
        return None;
    }
    let vendor = decoded.contains("/vendor/");
    serve_file(&canon, vendor, head)
}

pub async fn dispatch(State(st): State<Arc<AppState>>, req: Request) -> Response {
    let mut resp = dispatch_inner(&st, req).await;
    crate::security::apply_headers(&mut resp);
    resp
}

async fn dispatch_inner(st: &Arc<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let peer = parts
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_default();
    let ip = crate::security::client_ip(&peer, &parts.headers, &st.cfg.trusted_proxies);
    st.throttle.expire(&ip, now_ms() as u64);
    if let Some(r) = check_auth(&st.cfg, &parts.headers) {
        // failed logins are answered after a growing delay; a missing header is not a login attempt
        if r.status() == StatusCode::UNAUTHORIZED
            && parts.headers.contains_key(header::AUTHORIZATION)
        {
            let wait = st.throttle.failed(&ip, now_ms() as u64);
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        return r;
    }
    if crate::security::cross_origin_refused(&parts.method, &parts.headers, &st.cfg.allowed_origins)
    {
        return text(403, "Cross-origin request geweigerd.\n");
    }
    let bytes = axum::body::to_bytes(body, 100 * 1024)
        .await
        .unwrap_or_default();
    let body_map = match parse_body(&parts.headers, &bytes) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let raw_path = parts.uri.path().to_string();
    let trimmed = if raw_path.len() > 1 {
        raw_path.trim_end_matches('/')
    } else {
        raw_path.as_str()
    };
    let mut segs: Vec<String> = Vec::new();
    for s in trimmed.split('/').skip(1) {
        match percent_encoding::percent_decode_str(s).decode_utf8() {
            Ok(d) => segs.push(d.into_owned()),
            Err(_) => return error_page(400, &format!("URIError: Failed to decode param '{s}'")),
        }
    }
    let r = Req {
        method: &parts.method,
        segs,
        query: parse_query(parts.uri.query()),
        body: body_map,
    };
    if let Some(resp) = route(st, &r).await {
        return resp;
    }
    let get = parts.method == Method::GET || parts.method == Method::HEAD;
    if get {
        let root = PathBuf::from(&st.cfg.frontend_dir);
        if let Some(resp) = static_file(
            &root,
            &raw_path,
            parts.uri.query(),
            parts.method == Method::HEAD,
        ) {
            return resp;
        }
        if let Some(resp) = serve_file(
            &root.join("index.html"),
            false,
            parts.method == Method::HEAD,
        ) {
            return resp;
        }
    }
    error_page(404, &format!("Cannot {} {}", parts.method, raw_path))
}

pub fn app(st: Arc<AppState>) -> axum::Router {
    axum::Router::new().fallback(dispatch).with_state(st)
}

/// Start the server: first poll, then listen, then poll on an interval.
pub async fn run(
    cfg: Config,
    exec: Arc<dyn Exec>,
    udm: Option<crate::unifi::Unifi>,
) -> Result<(), String> {
    let st = AppState::with_udm(cfg.clone(), exec, udm);
    st.sync_maps().await;
    st.refresh().await;
    let poll = st.clone();
    let every = std::time::Duration::from_millis(cfg.poll_interval_ms);
    tokio::spawn(async move {
        let mut t = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            t.tick().await;
            poll.refresh().await;
        }
    });
    let addr = format!("{}:{}", cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("binding {addr}: {e}"))?;
    println!(
        "ARK dashboard on {}:{} \u{2192} {}",
        cfg.bind,
        cfg.port,
        match cfg.mode {
            crate::config::Mode::Fake => "fake VM (in-memory)",
            crate::config::Mode::Proxmox => "proxmox",
            crate::config::Mode::Local => "local",
        }
    );
    if !cfg.auth.enabled {
        eprintln!(
            "WARNING: DASH_AUTH_DISABLED=1 \u{2014} this dashboard can start/stop worlds and read admin passwords. Do not expose it."
        );
    } else if cfg.auth.pass.is_empty() {
        eprintln!(
            "ERROR: DASH_PASS is empty \u{2014} every request will be refused until you set it in deploy/.env."
        );
    }
    axum::serve(
        listener,
        app(st).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .map_err(|e| e.to_string())
}
