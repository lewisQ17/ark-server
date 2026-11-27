//! The Proxmox and UniFi clients against in-process HTTPS stubs with a self-signed certificate:
//! certificate pinning, ticket login and renewal, guest-exec polling and serialization, error texts.
//! Nothing here touches a network other than 127.0.0.1.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ark_dashboard::config::{Pve, UnifiCfg};
use ark_dashboard::exec::{Exec, ShellOpts};
use ark_dashboard::pinned::sha256_hex;
use ark_dashboard::proxmox::Proxmox;
use ark_dashboard::unifi::{Forward, Unifi};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Handler = dyn Fn(&Seen, usize) -> (u16, String) + Send + Sync;

struct Stub {
    port: u16,
    fingerprint: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    fn log(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }
    fn count(&self, method: &str, path_prefix: &str) -> usize {
        self.log()
            .iter()
            .filter(|s| s.method == method && s.path.starts_with(path_prefix))
            .count()
    }
}

fn decode_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let dec = |s: &str| {
                let s = s.replace('+', " ");
                String::from_utf8_lossy(
                    &percent_encoding::percent_decode_str(&s).collect::<Vec<u8>>(),
                )
                .into_owned()
            };
            (dec(k), dec(v))
        })
        .collect()
}

async fn start(handler: Arc<Handler>) -> Stub {
    let key = rcgen::KeyPair::generate().expect("key");
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .expect("params")
        .self_signed(&key)
        .expect("cert");
    let fingerprint = sha256_hex(cert.der());
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let cfg = tokio_rustls::rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .expect("server cfg");
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let n = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let handler = handler.clone();
            let seen = seen2.clone();
            let n = n.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(sock).await else {
                    return; // a client that refuses the certificate ends up here
                };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let (head_end, content_len) = loop {
                    let Ok(k) = tls.read(&mut tmp).await else {
                        return;
                    };
                    if k == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..k]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (p + 4, cl);
                    }
                };
                while buf.len() < head_end + content_len {
                    let Ok(k) = tls.read(&mut tmp).await else {
                        return;
                    };
                    if k == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..k]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let headers = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                    .collect();
                let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                let s = Seen {
                    method,
                    path,
                    headers,
                    body,
                };
                seen.lock().expect("seen").push(s.clone());
                let idx = n.fetch_add(1, Ordering::SeqCst);
                let (status, out) = handler(&s, idx);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}",
                    out.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    Stub {
        port,
        fingerprint,
        seen,
    }
}

fn pve_cfg(stub: &Stub, fp: &str) -> Pve {
    Pve {
        host: "127.0.0.1".into(),
        port: stub.port,
        node: "pve".into(),
        vmid: 1050,
        user: "root@pam".into(),
        password: "dummy pw&=+".into(),
        tls_fingerprint: fp.into(),
        tls_insecure: false,
    }
}

fn ticket_reply() -> (u16, String) {
    (
        200,
        json!({"data": {"ticket": "PVE:root@pam:TICKET", "CSRFPreventionToken": "CSRF1"}})
            .to_string(),
    )
}

#[tokio::test]
async fn pinned_login_status_and_csrf_only_on_writes() {
    let h: Arc<Handler> = Arc::new(|s, _| match (s.method.as_str(), s.path.as_str()) {
        ("POST", "/api2/json/access/ticket") => ticket_reply(),
        ("GET", p) if p.ends_with("/status/current") => (
            200,
            json!({"data": {"name": "ARK", "status": "running", "cpus": 4}}).to_string(),
        ),
        ("GET", p) if p.contains("/rrddata?timeframe=hour") => {
            (200, json!({"data": [{"cpu": 0.5, "time": 1}]}).to_string())
        }
        ("POST", p) if p.ends_with("/status/reboot") => {
            (200, json!({"data": "UPID:x"}).to_string())
        }
        _ => (404, "{}".into()),
    });
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");

    assert_eq!(pve.vm_status().await.expect("status")["name"], "ARK");
    assert_eq!(pve.rrddata().await.expect("rrd").len(), 1);
    pve.vm_power("reboot").await.expect("power");

    let log = stub.log();
    let login = &log[0];
    assert_eq!(login.path, "/api2/json/access/ticket");
    assert_eq!(login.body, "username=root%40pam&password=dummy+pw%26%3D%2B");
    assert_eq!(
        stub.count("POST", "/api2/json/access/ticket"),
        1,
        "one login for three calls"
    );
    let get = log
        .iter()
        .find(|s| s.path.ends_with("/status/current"))
        .expect("get");
    assert_eq!(get.path, "/api2/json/nodes/pve/qemu/1050/status/current");
    assert_eq!(
        get.headers.get("cookie").map(String::as_str),
        Some("PVEAuthCookie=PVE:root@pam:TICKET")
    );
    assert!(!get.headers.contains_key("csrfpreventiontoken"));
    let post = log
        .iter()
        .find(|s| s.path.ends_with("/status/reboot"))
        .expect("post");
    assert_eq!(
        post.headers.get("csrfpreventiontoken").map(String::as_str),
        Some("CSRF1")
    );
}

#[tokio::test]
async fn wrong_pin_never_sends_the_password() {
    let h: Arc<Handler> = Arc::new(|_, _| ticket_reply());
    let stub = start(h).await;
    let wrong = "00".repeat(32);
    let pve = Proxmox::new(&pve_cfg(&stub, &wrong)).expect("client");
    let e = pve.vm_status().await.expect_err("must refuse");
    assert!(e.starts_with("fetch failed"), "{e}");
    assert!(e.contains("TLS fingerprint mismatch"), "{e}");
    assert!(stub.log().is_empty(), "no request may reach the peer");
    // colons and upper case are fine in the configured pin
    let pretty = stub
        .fingerprint
        .as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).expect("hex").to_uppercase())
        .collect::<Vec<_>>()
        .join(":");
    let pve = Proxmox::new(&pve_cfg(&stub, &pretty)).expect("client");
    assert!(
        pve.vm_status().await.is_ok(),
        "colons and upper case in the pin are accepted"
    );
    assert_eq!(stub.count("POST", "/api2/json/access/ticket"), 1);
}

#[tokio::test]
async fn failed_login_and_error_bodies_are_reported_like_the_original() {
    let h: Arc<Handler> = Arc::new(|s, _| match s.path.as_str() {
        "/api2/json/access/ticket" => (401, "{}".into()),
        _ => (500, "{}".into()),
    });
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");
    assert_eq!(
        pve.vm_status().await.expect_err("auth"),
        "Proxmox auth failed: HTTP 401"
    );

    let h: Arc<Handler> = Arc::new(|s, _| match s.path.as_str() {
        "/api2/json/access/ticket" => ticket_reply(),
        _ => (500, format!("boom {}", "x".repeat(300))),
    });
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");
    let e = pve.vm_status().await.expect_err("500");
    assert!(
        e.starts_with("Proxmox GET /nodes/pve/qemu/1050/status/current \u{2192} HTTP 500 boom xxx"),
        "{e}"
    );
    assert_eq!(
        e.chars().count(),
        "Proxmox GET /nodes/pve/qemu/1050/status/current \u{2192} HTTP 500 "
            .chars()
            .count()
            + 200
    );
}

#[tokio::test]
async fn an_expired_ticket_is_renewed_once() {
    let logins = Arc::new(AtomicUsize::new(0));
    let l2 = logins.clone();
    let h: Arc<Handler> = Arc::new(move |s, _| {
        if s.path == "/api2/json/access/ticket" {
            let n = l2.fetch_add(1, Ordering::SeqCst);
            return (
                200,
                json!({"data": {"ticket": format!("T{n}"), "CSRFPreventionToken": "C"}})
                    .to_string(),
            );
        }
        if s.headers.get("cookie").map(String::as_str) == Some("PVEAuthCookie=T0") {
            return (401, "{}".into());
        }
        (200, json!({"data": {"status": "running"}}).to_string())
    });
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");
    assert_eq!(pve.vm_status().await.expect("renewed")["status"], "running");
    assert_eq!(logins.load(Ordering::SeqCst), 2);
}

/// A guest-agent stub: `exec` returns a pid, `exec-status` says "running" a few times, then the result.
fn agent_handler(
    polls_before_done: usize,
    inflight: Arc<AtomicUsize>,
    max_inflight: Arc<AtomicUsize>,
) -> Arc<Handler> {
    let polls: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_pid = Arc::new(AtomicUsize::new(100));
    Arc::new(move |s, _| {
        if s.path == "/api2/json/access/ticket" {
            return ticket_reply();
        }
        if s.method == "POST" && s.path.ends_with("/agent/exec") {
            let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
            max_inflight.fetch_max(now, Ordering::SeqCst);
            let pid = next_pid.fetch_add(1, Ordering::SeqCst);
            return (200, json!({"data": {"pid": pid}}).to_string());
        }
        if let Some((_, pid)) = s.path.split_once("exec-status?pid=") {
            let mut m = polls.lock().expect("polls");
            let n = m.entry(pid.to_string()).or_insert(0);
            *n += 1;
            if *n <= polls_before_done {
                return (200, json!({"data": {"exited": 0}}).to_string());
            }
            inflight.fetch_sub(1, Ordering::SeqCst);
            return (
                200,
                json!({"data": {"exited": 1, "exitcode": 3, "out-data": format!("out{pid}\n"), "err-data": "e"}}).to_string(),
            );
        }
        (404, "{}".into())
    })
}

#[tokio::test]
async fn guest_shell_builds_the_runuser_command_and_polls() {
    let h = agent_handler(2, Arc::default(), Arc::default());
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");

    let t = Instant::now();
    let r = pve
        .guest_shell(
            "echo hi; cat",
            ShellOpts::user("arkadmin")
                .input("secret\nSaveWorld")
                .timeout(10_000),
        )
        .await
        .expect("exec");
    assert_eq!(
        (r.exitcode, r.out.as_str(), r.err.as_str()),
        (3, "out100\n", "e")
    );
    assert!(
        t.elapsed() >= Duration::from_millis(600),
        "two polls of 350 ms"
    );

    let exec = stub
        .log()
        .into_iter()
        .find(|s| s.path.ends_with("/agent/exec"))
        .expect("exec call");
    assert_eq!(
        exec.headers.get("csrfpreventiontoken").map(String::as_str),
        Some("CSRF1")
    );
    let form = decode_form(&exec.body);
    let commands: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "command")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(
        commands,
        [
            "/usr/sbin/runuser",
            "-u",
            "arkadmin",
            "--",
            "/bin/bash",
            "-lc",
            "echo hi; cat"
        ]
    );
    assert!(form.contains(&("input-data".into(), "secret\nSaveWorld".into())));

    // without a user: plain bash, and guest_exec ignores the user like the original
    pve.guest_shell("id", ShellOpts::default())
        .await
        .expect("root");
    pve.guest_exec(
        &["/bin/sh", "-c", "cat > /x"],
        ShellOpts::user("arkadmin").input("data"),
    )
    .await
    .expect("exec");
    let log = stub.log();
    let execs: Vec<Vec<(String, String)>> = log
        .iter()
        .filter(|s| s.path.ends_with("/agent/exec"))
        .map(|s| decode_form(&s.body))
        .collect();
    let cmds = |f: &Vec<(String, String)>| {
        f.iter()
            .filter(|(k, _)| k == "command")
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(cmds(&execs[1]), ["/bin/bash", "-lc", "id"]);
    assert_eq!(cmds(&execs[2]), ["/bin/sh", "-c", "cat > /x"]);
}

#[tokio::test]
async fn guest_exec_times_out_and_the_queue_keeps_working() {
    let h = agent_handler(usize::MAX, Arc::default(), Arc::default());
    let stub = start(h).await;
    let pve = Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client");
    let e = pve
        .guest_shell("sleep 100", ShellOpts::default().timeout(500))
        .await
        .expect_err("timeout");
    assert_eq!(e, "guest-exec timed out");
    // a second call is not blocked by the failed one
    let e = pve
        .guest_shell("x", ShellOpts::default().timeout(300))
        .await
        .expect_err("timeout again");
    assert_eq!(e, "guest-exec timed out");
}

#[tokio::test]
async fn guest_calls_are_serialized() {
    let inflight = Arc::new(AtomicUsize::new(0));
    let max = Arc::new(AtomicUsize::new(0));
    let h = agent_handler(1, inflight, max.clone());
    let stub = start(h).await;
    let pve = Arc::new(Proxmox::new(&pve_cfg(&stub, &stub.fingerprint)).expect("client"));
    let mut tasks = Vec::new();
    for i in 0..5 {
        let p = pve.clone();
        tasks.push(tokio::spawn(async move {
            p.guest_shell(&format!("echo {i}"), ShellOpts::default())
                .await
        }));
    }
    for t in tasks {
        t.await.expect("join").expect("exec");
    }
    assert_eq!(
        max.load(Ordering::SeqCst),
        1,
        "the guest agent handles one exec at a time"
    );
    assert_eq!(
        stub.count("POST", "/api2/json/nodes/pve/qemu/1050/agent/exec"),
        5
    );
}

fn unifi_cfg(stub: &Stub) -> UnifiCfg {
    UnifiCfg {
        enabled: true,
        host: format!("127.0.0.1:{}", stub.port),
        api_key: "dummy-key".into(),
        site: "default".into(),
        ark_vm_ip: "10.9.9.9".into(),
        proto: "udp".into(),
        tls_fingerprint: stub.fingerprint.clone(),
        tls_insecure: false,
    }
}

#[tokio::test]
async fn unifi_upsert_replaces_duplicates_and_delete_counts_them() {
    let forwards = Arc::new(Mutex::new(vec![
        json!({"_id": "a1", "name": "ARK-Foo"}),
        json!({"_id": "b2", "name": "Other"}),
        json!({"_id": "c3", "name": "ARK-Foo"}),
    ]));
    let f2 = forwards.clone();
    let h: Arc<Handler> = Arc::new(move |s, _| {
        let base = "/proxy/network/api/s/default/rest/portforward";
        let mut f = f2.lock().expect("fw");
        match (s.method.as_str(), s.path.as_str()) {
            ("GET", p) if p == base => (
                200,
                json!({"meta": {"rc": "ok"}, "data": f.clone()}).to_string(),
            ),
            ("DELETE", p) if p.starts_with(&format!("{base}/")) => {
                let id = &p[base.len() + 1..];
                f.retain(|x| x["_id"] != id);
                (200, json!({"meta": {"rc": "ok"}, "data": []}).to_string())
            }
            ("POST", p) if p == base => {
                let b: Value = serde_json::from_str(&s.body).expect("json body");
                let mut stored = b.clone();
                stored["_id"] = json!("n1");
                f.push(stored);
                (200, json!({"meta": {"rc": "ok"}, "data": [b]}).to_string())
            }
            _ => (404, "{}".into()),
        }
    });
    let stub = start(h).await;
    let udm = Unifi::new(&unifi_cfg(&stub)).expect("client");

    udm.upsert_forward(&Forward {
        name: "ARK-Foo",
        wan_port: Some(7779),
        fwd_ip: "10.9.9.9",
        proto: "udp",
    })
    .await
    .expect("upsert");
    let now = forwards.lock().expect("fw").clone();
    assert_eq!(
        now.len(),
        2,
        "both duplicates gone, the other forward untouched, one new"
    );
    let mut new = now
        .iter()
        .find(|x| x["name"] == "ARK-Foo")
        .expect("new")
        .clone();
    new.as_object_mut().expect("object").remove("_id");
    assert_eq!(
        &new,
        &json!({"name": "ARK-Foo", "enabled": true, "pfwd_interface": "wan", "src": "any", "log": false,
                "proto": "udp", "dst_port": "7779", "fwd": "10.9.9.9", "fwd_port": "7779"})
    );
    assert_eq!(
        stub.log()[0].headers.get("x-api-key").map(String::as_str),
        Some("dummy-key")
    );
    assert_eq!(
        udm.delete_forward_by_name("ARK-Foo").await.expect("delete"),
        1
    );
    assert_eq!(
        udm.delete_forward_by_name("ARK-Foo").await.expect("delete"),
        0
    );
}

#[tokio::test]
async fn unifi_errors_carry_the_controller_message() {
    let h: Arc<Handler> = Arc::new(|s, _| {
        if s.method == "GET" {
            (
                200,
                json!({"meta": {"rc": "error", "msg": "api.err.NoPermission"}, "data": []})
                    .to_string(),
            )
        } else {
            (500, "not json at all".into())
        }
    });
    let stub = start(h).await;
    let udm = Unifi::new(&unifi_cfg(&stub)).expect("client");
    assert_eq!(
        udm.list_forwards().await.expect_err("rc"),
        "UniFi GET /rest/portforward \u{2192} HTTP 200 api.err.NoPermission"
    );
    let e = udm
        .delete_forward_by_name("x")
        .await
        .expect_err("list fails first");
    assert!(e.contains("api.err.NoPermission"), "{e}");

    let mut cfg = unifi_cfg(&stub);
    cfg.tls_fingerprint = "ab".repeat(32);
    let udm = Unifi::new(&cfg).expect("client");
    let e = udm.list_forwards().await.expect_err("pin");
    assert!(e.contains("TLS fingerprint mismatch"), "{e}");
}
