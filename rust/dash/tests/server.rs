//! End-to-end through the router with the in-memory VM: auth, routing quirks, the poller and the leak guard.

use std::collections::HashMap;
use std::sync::Arc;

use ark_dashboard::config::Config;
use ark_dashboard::exec::{Exec, ShellOpts};
use ark_dashboard::fake::FakeVm;
use ark_dashboard::server::{AppState, app};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt;

fn cfg(extra: &[(&str, &str)]) -> Config {
    let mut m: HashMap<String, String> = [
        ("ARK_EXEC_MODE", "fake"),
        ("DASH_PASS", "pw"),
        ("ARK_FRONTEND_DIR", "/nonexistent"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
    .collect();
    for (k, v) in extra {
        m.insert((*k).to_string(), (*v).to_string());
    }
    Config::from_env(&|k| m.get(k).cloned()).expect("config")
}

async fn booted(extra: &[(&str, &str)]) -> Arc<AppState> {
    let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
    let st = AppState::new(cfg(extra), exec);
    st.sync_maps().await;
    st.refresh().await;
    st
}

fn basic(user: &str, pass: &str) -> String {
    // tiny base64 encoder for the test
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let data = format!("{user}:{pass}");
    let mut out = String::new();
    for chunk in data.as_bytes().chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    format!("Basic {out}")
}

async fn call(
    st: &Arc<AppState>,
    method: &str,
    uri: &str,
    auth: Option<&str>,
    body: Option<&str>,
) -> (StatusCode, String) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(a) = auth {
        b = b.header(header::AUTHORIZATION, a);
    }
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    let resp = app(st.clone())
        .oneshot(
            b.body(Body::from(body.unwrap_or("").to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn auth_is_checked_before_anything_else() {
    let st = booted(&[]).await;
    assert_eq!(
        call(&st, "GET", "/api/health", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &st,
            "GET",
            "/api/health",
            Some(&basic("admin", "nope")),
            None
        )
        .await
        .1,
        "Invalid credentials.\n"
    );
    assert_eq!(
        call(&st, "GET", "/api/health", Some("Bearer x"), None)
            .await
            .1,
        "Authentication required.\n"
    );
    let (s, b) = call(&st, "GET", "/api/health", Some(&basic("admin", "pw")), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(&b).expect("json")["maps"], 2);
}

#[tokio::test]
async fn no_password_means_everything_is_refused() {
    let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
    let mut c = cfg(&[]);
    c.auth.pass = String::new();
    let st = AppState::new(c, exec);
    assert_eq!(
        call(&st, "GET", "/", None, None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn routes_ignore_case_and_trailing_slash_and_unknown_methods_are_404() {
    let st = booted(&[]).await;
    let a = basic("admin", "pw");
    assert_eq!(
        call(&st, "GET", "/API/Health/", Some(&a), None).await.0,
        StatusCode::OK
    );
    let (s, b) = call(&st, "PUT", "/api/state", Some(&a), Some("{}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(b.contains("<pre>Cannot PUT /api/state</pre>"));
}

#[tokio::test]
async fn malformed_json_is_a_400_page() {
    let st = booted(&[]).await;
    let (s, _) = call(
        &st,
        "POST",
        "/api/rcon",
        Some(&basic("admin", "pw")),
        Some("{not json"),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn actions_write_the_audit_log_newest_first() {
    let st = booted(&[]).await;
    let a = basic("admin", "pw");
    call(
        &st,
        "POST",
        "/api/rcon",
        Some(&a),
        Some(r#"{"id":"theisland","command":"KickPlayer 1"}"#),
    )
    .await;
    call(
        &st,
        "POST",
        "/api/instance/ragnarok/action",
        Some(&a),
        Some(r#"{"action":"start"}"#),
    )
    .await;
    let (_, b) = call(&st, "GET", "/api/audit", Some(&a), None).await;
    let v: Value = serde_json::from_str(&b).expect("json");
    assert_eq!(v["audit"][0]["action"], "start");
    assert_eq!(v["audit"][1]["danger"], true); // "kick" is a dangerous command
}

#[tokio::test]
async fn host_power_is_refused_in_local_mode() {
    let st = booted(&[]).await;
    let (s, b) = call(
        &st,
        "POST",
        "/api/instance/theisland/action",
        Some(&basic("admin", "pw")),
        Some(r#"{"action":"vmreboot"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(b.contains("Refusing to reboot the machine"));
}

#[tokio::test]
async fn leak_guard_restarts_an_empty_world_over_the_ceiling_and_never_one_with_players() {
    // ceiling 5 GB, minimum uptime 1 minute: the fake world uses 6 GB and has been up 90 minutes
    let exec: Arc<dyn Exec> = Arc::new(FakeVm::new(false));
    let st = AppState::new(
        cfg(&[
            ("ARK_LEAK_GUARD_GB", "5"),
            ("ARK_LEAK_GUARD_MIN_UPTIME_H", "0.01"),
        ]),
        exec.clone(),
    );
    // Ragnarok has nobody online: start it so it reports 6 GB
    exec.guest_shell(
        "systemctl start ark-ragnarok 2>&1; echo \"exit=$?\"",
        ShellOpts::default(),
    )
    .await
    .expect("exec");
    st.sync_maps().await;
    st.refresh().await;
    let (_, b) = call(&st, "GET", "/api/audit", Some(&basic("admin", "pw")), None).await;
    let v: Value = serde_json::from_str(&b).expect("json");
    let entries = v["audit"].as_array().expect("audit");
    assert_eq!(
        entries.len(),
        1,
        "only the empty world is restarted: {entries:?}"
    );
    assert_eq!(entries[0]["who"], "leak-guard");
    assert_eq!(entries[0]["target"], "Ragnarok");
}

#[tokio::test]
async fn static_files_stay_inside_the_frontend_folder() {
    let dir = std::env::temp_dir().join(format!("ark-dash-static-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("vendor")).expect("dir");
    std::fs::write(dir.join("index.html"), "<h1>home</h1>").expect("index");
    std::fs::write(dir.join("vendor/lib.js"), "x=1").expect("lib");
    std::fs::write(dir.join(".secret"), "no").expect("dot");
    let st = booted(&[("ARK_FRONTEND_DIR", dir.to_str().expect("utf8"))]).await;
    let a = basic("admin", "pw");
    assert_eq!(
        call(&st, "GET", "/vendor/lib.js", Some(&a), None).await.1,
        "x=1"
    );
    assert_eq!(
        call(&st, "GET", "/..%2f..%2fetc/passwd", Some(&a), None)
            .await
            .1,
        "<h1>home</h1>"
    );
    assert_eq!(
        call(&st, "GET", "/.secret", Some(&a), None).await.1,
        "<h1>home</h1>"
    );
    assert_eq!(
        call(&st, "GET", "/any/spa/route", Some(&a), None).await.1,
        "<h1>home</h1>"
    );
}

async fn call_full(
    st: &Arc<AppState>,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut b = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = app(st.clone())
        .oneshot(
            b.body(Body::from(body.unwrap_or("").to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let (status, hdrs) = (resp.status(), resp.headers().clone());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, hdrs, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let st = booted(&[]).await;
    let a = basic("admin", "pw");
    for (m, u, auth) in [
        ("GET", "/api/health", Some(a.as_str())),
        ("GET", "/api/health", None),
        ("GET", "/nope", Some(a.as_str())),
    ] {
        let hs: Vec<(&str, &str)> = auth.map(|x| ("authorization", x)).into_iter().collect();
        let (_, h, _) = call_full(&st, m, u, &hs, None).await;
        assert_eq!(h["x-content-type-options"], "nosniff", "{u}");
        assert_eq!(h["x-frame-options"], "DENY");
        assert_eq!(h["referrer-policy"], "no-referrer");
        assert_eq!(h["cross-origin-opener-policy"], "same-origin");
    }
}

#[tokio::test]
async fn a_foreign_origin_cannot_change_anything_but_may_read() {
    let st = booted(&[("ALLOWED_ORIGINS", "https://ark.example")]).await;
    let a = basic("admin", "pw");
    let post = |origin: &'static str| {
        let st = st.clone();
        let a = a.clone();
        async move {
            call_full(
                &st,
                "POST",
                "/api/rcon",
                &[
                    ("authorization", a.as_str()),
                    ("content-type", "application/json"),
                    ("host", "dash.lan:8787"),
                    ("origin", origin),
                ],
                Some(r#"{"id":"theisland","command":"ListPlayers"}"#),
            )
            .await
        }
    };
    let (s, _, b) = post("https://evil.example").await;
    assert_eq!((s, b.as_str()), (StatusCode::FORBIDDEN, "Cross-origin request geweigerd.\n"));
    assert_eq!(post("http://dash.lan:8787").await.0, StatusCode::OK);
    assert_eq!(post("https://ark.example").await.0, StatusCode::OK);
    let (s, _, _) = call_full(
        &st,
        "GET",
        "/api/health",
        &[("authorization", a.as_str()), ("origin", "https://evil.example")],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // the refusal is not an action: nothing was logged for the blocked request
    let (_, b) = call(&st, "GET", "/api/audit", Some(&a), None).await;
    let v: Value = serde_json::from_str(&b).expect("json");
    assert_eq!(v["audit"].as_array().expect("audit").len(), 2);
}

#[tokio::test]
async fn failed_logins_wait_longer_each_time_but_a_good_login_does_not() {
    let st = booted(&[]).await;
    let bad = basic("admin", "nope");
    let t = std::time::Instant::now();
    for _ in 0..2 {
        assert_eq!(call(&st, "GET", "/api/health", Some(&bad), None).await.0, StatusCode::UNAUTHORIZED);
    }
    assert!(t.elapsed() >= std::time::Duration::from_millis(250 + 500), "{:?}", t.elapsed());
    let t = std::time::Instant::now();
    assert_eq!(call(&st, "GET", "/api/health", Some(&basic("admin", "pw")), None).await.0, StatusCode::OK);
    assert_eq!(call(&st, "GET", "/api/health", None, None).await.0, StatusCode::UNAUTHORIZED);
    assert!(t.elapsed() < std::time::Duration::from_millis(200), "no header is not a login attempt");
}

#[tokio::test]
async fn an_unreachable_host_gives_unknown_not_stopped_and_500s_with_the_reason() {
    let fake = Arc::new(FakeVm::new(false));
    let exec: Arc<dyn Exec> = fake.clone();
    let st = AppState::new(cfg(&[]), exec);
    st.sync_maps().await;
    st.refresh().await;
    let a = basic("admin", "pw");
    let state = |b: &str| serde_json::from_str::<Value>(b).expect("json");
    let island = |v: &Value| v["instances"].as_array().expect("instances").iter().find(|i| i["id"] == "theisland").expect("island")["status"].clone();
    let (_, b) = call(&st, "GET", "/api/state", Some(&a), None).await;
    assert_eq!(island(&state(&b)), "running");

    fake.set_down(Some("Proxmox POST /agent/exec \u{2192} HTTP 500 QEMU guest agent is not running"));
    st.sync_maps().await;
    st.refresh().await;
    let (_, b) = call(&st, "GET", "/api/state", Some(&a), None).await;
    let v = state(&b);
    assert_eq!(v["connected"], true, "the poller itself survives, like the original");
    assert_eq!(v["host"], Value::Null);
    assert_eq!(v["instances"].as_array().expect("instances").len(), 2, "maps are kept, not dropped");
    assert_eq!(island(&v), "unknown");

    let (s, b) = call(&st, "GET", "/api/backups/theisland", Some(&a), None).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(state(&b), serde_json::json!({"ok": false, "error": "Proxmox POST /agent/exec \u{2192} HTTP 500 QEMU guest agent is not running", "backups": []}));
    let (s, b) = call(&st, "GET", "/api/console/theisland", Some(&a), None).await;
    assert_eq!((s, state(&b)["lines"].clone()), (StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!([])));
    let (s, _) = call(&st, "POST", "/api/rcon", Some(&a), Some(r#"{"id":"theisland","command":"SaveWorld"}"#)).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    let (s, _) = call(&st, "POST", "/api/instance/theisland/action", Some(&a), Some(r#"{"action":"restart"}"#)).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    let (s, b) = call(&st, "POST", "/api/maps", Some(&a), Some(r#"{"display":"Aberration","internal":"Aberration_WP","adminPassword":"x","gamePort":7781,"queryPort":27017,"rconPort":27022,"maxPlayers":70}"#)).await;
    assert_eq!((s, state(&b)["ok"].clone()), (StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!(false)));
    let (_, b) = call(&st, "GET", "/api/update/theisland", Some(&a), None).await;
    assert_eq!(state(&b)["ok"], false);

    // and it recovers on its own
    fake.set_down(None);
    st.sync_maps().await;
    st.refresh().await;
    let (_, b) = call(&st, "GET", "/api/state", Some(&a), None).await;
    assert_eq!(island(&state(&b)), "running");
}
