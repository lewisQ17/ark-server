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
    .await;
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
