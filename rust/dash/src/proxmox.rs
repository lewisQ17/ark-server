//! Thin Proxmox VE API client (`proxmox.js`), implementing [`Exec`].
//!
//! Two things the dashboard relies on:
//! 1. `status/current` and `rrddata`: host-level VM metrics. Read-only, never touches the guest.
//! 2. guest-exec: run a command inside the VM through the already running qemu-guest-agent, so nothing is
//!    installed on the game VM. The agent handles one exec at a time reliably, so calls are serialized.
//!
//! Auth is a root@pam ticket (cached 110 minutes, renewed on a 401); the peer certificate is pinned
//! ([`crate::pinned`]).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::Pve;
use crate::exec::{Exec, ExecOut, ShellOpts};
use crate::pinned;
use crate::util::slice_units;

/// `application/x-www-form-urlencoded` like `URLSearchParams`: space is `+`, `*-._` and alphanumerics stay.
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    fn enc(s: &str, out: &mut String) {
        for b in s.bytes() {
            match b {
                b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'*' | b'-' | b'.' | b'_' => {
                    out.push(b as char);
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        enc(k, &mut out);
        out.push('=');
        enc(v, &mut out);
    }
    out
}

async fn json_body(res: reqwest::Response) -> Result<Value, String> {
    let txt = res.text().await.map_err(|e| pinned::transport_error(&e))?;
    serde_json::from_str(&txt).map_err(|e| e.to_string())
}

struct Ticket {
    ticket: Option<String>,
    csrf: Option<String>,
    at: Instant,
}

pub struct Proxmox {
    base: String,
    node: String,
    vmid: i64,
    user: String,
    password: String,
    client: reqwest::Client,
    auth: Mutex<Ticket>,
    queue: Mutex<()>,
}

const TICKET_TTL: Duration = Duration::from_secs(110 * 60);
const EXEC_POLL: Duration = Duration::from_millis(350);

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

impl Proxmox {
    pub fn new(cfg: &Pve) -> Result<Self, String> {
        let client = pinned::build_client("proxmox", &cfg.tls_fingerprint, cfg.tls_insecure)?;
        Ok(Self {
            base: format!("https://{}:{}/api2/json", cfg.host, cfg.port),
            node: cfg.node.clone(),
            vmid: cfg.vmid,
            user: cfg.user.clone(),
            password: cfg.password.clone(),
            client,
            auth: Mutex::new(Ticket {
                ticket: None,
                csrf: None,
                at: Instant::now(),
            }),
            queue: Mutex::new(()),
        })
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: &[(&str, String)],
        body: Option<String>,
    ) -> Result<reqwest::Response, String> {
        let mut rb = self
            .client
            .request(method, format!("{}{path}", self.base));
        for (k, v) in headers {
            rb = rb.header(*k, v);
        }
        if let Some(b) = body {
            rb = rb
                .header(
                    "content-type",
                    "application/x-www-form-urlencoded;charset=UTF-8",
                )
                .body(b);
        }
        rb.send().await.map_err(|e| pinned::transport_error(&e))
    }

    async fn authenticate(&self, t: &mut Ticket) -> Result<(), String> {
        let body = form_encode(&[("username", &self.user), ("password", &self.password)]);
        let res = self
            .send(reqwest::Method::POST, "/access/ticket", &[], Some(body))
            .await?;
        if !res.status().is_success() {
            return Err(format!("Proxmox auth failed: HTTP {}", res.status().as_u16()));
        }
        let j: Value = json_body(res).await?;
        let d = j.get("data").ok_or("Cannot read properties of undefined (reading 'ticket')")?;
        t.ticket = d.get("ticket").and_then(Value::as_str).map(String::from);
        t.csrf = d
            .get("CSRFPreventionToken")
            .and_then(Value::as_str)
            .map(String::from);
        t.at = Instant::now();
        Ok(())
    }

    /// A valid ticket and its CSRF token, logging in when there is none or it is older than 110 minutes.
    async fn ticket(&self, force: bool) -> Result<(String, String), String> {
        let mut t = self.auth.lock().await;
        if force || t.ticket.is_none() || t.at.elapsed() > TICKET_TTL {
            self.authenticate(&mut t).await?;
        }
        Ok((
            t.ticket.clone().unwrap_or_default(),
            t.csrf.clone().unwrap_or_default(),
        ))
    }

    /// Authenticated request; retries once on 401 with a fresh ticket. Returns the `data` member.
    pub async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        form: Option<String>,
    ) -> Result<Value, String> {
        let (mut ticket, mut csrf) = self.ticket(false).await?;
        let mut retried = false;
        loop {
            let mut headers = vec![("cookie", format!("PVEAuthCookie={ticket}"))];
            // Proxmox wants the CSRF token on every mutating request, with or without a body.
            if method != reqwest::Method::GET {
                headers.push(("csrfpreventiontoken", csrf.clone()));
            }
            let res = self
                .send(method.clone(), path, &headers, form.clone())
                .await?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED && !retried {
                retried = true;
                (ticket, csrf) = self.ticket(true).await?;
                continue;
            }
            let status = res.status();
            if !status.is_success() {
                let txt = res.text().await.unwrap_or_default();
                return Err(format!(
                    "Proxmox {method} {path} \u{2192} HTTP {} {}",
                    status.as_u16(),
                    slice_units(&txt, 200)
                ));
            }
            let j: Value = json_body(res).await?;
            return Ok(j.get("data").cloned().unwrap_or(Value::Null));
        }
    }

    fn vm_path(&self, tail: &str) -> String {
        format!("/nodes/{}/qemu/{}/{tail}", self.node, self.vmid)
    }

    async fn exec_inner(
        &self,
        args: &[String],
        input: Option<&str>,
        timeout_ms: u64,
    ) -> Result<ExecOut, String> {
        let mut pairs: Vec<(&str, &str)> = args.iter().map(|a| ("command", a.as_str())).collect();
        if let Some(i) = input {
            pairs.push(("input-data", i));
        }
        let started = self
            .api(
                reqwest::Method::POST,
                &self.vm_path("agent/exec"),
                Some(form_encode(&pairs)),
            )
            .await?;
        let pid = started
            .get("pid")
            .map(|p| match p {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .ok_or("Cannot read properties of undefined (reading 'pid')")?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let st = self
                .api(
                    reqwest::Method::GET,
                    &self.vm_path(&format!("agent/exec-status?pid={pid}")),
                    None,
                )
                .await?;
            if truthy(st.get("exited")) {
                let s = |k: &str| {
                    st.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                return Ok(ExecOut {
                    exitcode: st
                        .get("exitcode")
                        .and_then(Value::as_i64)
                        .map_or(-1, |c| c as i32),
                    out: s("out-data"),
                    err: s("err-data"),
                });
            }
            if Instant::now() > deadline {
                return Err("guest-exec timed out".into());
            }
            tokio::time::sleep(EXEC_POLL).await;
        }
    }
}

#[async_trait]
impl Exec for Proxmox {
    async fn guest_shell(&self, script: &str, opts: ShellOpts) -> Result<ExecOut, String> {
        let cmd: Vec<String> = match &opts.as_user {
            Some(u) => ["/usr/sbin/runuser", "-u", u, "--", "/bin/bash", "-lc", script]
                .map(String::from)
                .to_vec(),
            None => ["/bin/bash", "-lc", script].map(String::from).to_vec(),
        };
        let _q = self.queue.lock().await;
        self.exec_inner(&cmd, opts.input.as_deref(), opts.timeout_ms)
            .await
    }

    /// Like the original, `as_user` is not applied here: only `guest_shell` wraps the command in `runuser`.
    async fn guest_exec(&self, args: &[&str], opts: ShellOpts) -> Result<ExecOut, String> {
        let cmd: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let _q = self.queue.lock().await;
        self.exec_inner(&cmd, opts.input.as_deref(), opts.timeout_ms)
            .await
    }

    async fn vm_status(&self) -> Result<Value, String> {
        self.api(reqwest::Method::GET, &self.vm_path("status/current"), None)
            .await
    }

    async fn rrddata(&self) -> Result<Vec<Value>, String> {
        let d = self
            .api(
                reqwest::Method::GET,
                &self.vm_path("rrddata?timeframe=hour"),
                None,
            )
            .await?;
        Ok(d.as_array().cloned().unwrap_or_default())
    }

    /// action: start | stop | shutdown | reboot | suspend | resume
    async fn vm_power(&self, action: &str) -> Result<(), String> {
        self.api(
            reqwest::Method::POST,
            &self.vm_path(&format!("status/{action}")),
            None,
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_matches_urlsearchparams() {
        assert_eq!(
            form_encode(&[("command", "/bin/bash"), ("command", "-lc"), ("input-data", "a b\nc&d=é*~")]),
            "command=%2Fbin%2Fbash&command=-lc&input-data=a+b%0Ac%26d%3D%C3%A9*%7E"
        );
    }
}
