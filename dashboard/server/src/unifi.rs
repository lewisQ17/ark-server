//! Minimal UniFi/UDM controller client for WAN port-forwards (`unifi.js`).
//!
//! Lets the dashboard open the internet-facing game port when a map is created and remove it when the map is
//! deleted. RCON and query ports are never forwarded. Auth is the `X-API-KEY` of a UniFi OS integration; the
//! controller certificate is pinned ([`crate::pinned`]).

use serde_json::{Value, json};

use crate::config::UnifiCfg;
use crate::pinned;
use crate::util::slice_units;

pub struct Unifi {
    base: String,
    api_key: String,
    client: reqwest::Client,
}

/// A WAN to LAN forward, identified by name.
pub struct Forward<'a> {
    pub name: &'a str,
    /// `None` is what `parseInt` gives for text that is not a number (`NaN`).
    pub wan_port: Option<i64>,
    pub fwd_ip: &'a str,
    pub proto: &'a str,
}

impl Unifi {
    pub fn new(cfg: &UnifiCfg) -> Result<Self, String> {
        let client = pinned::build_client("unifi", &cfg.tls_fingerprint, cfg.tls_insecure)?;
        Ok(Self {
            base: format!("https://{}/proxy/network/api/s/{}", cfg.host, cfg.site),
            api_key: cfg.api_key.clone(),
            client,
        })
    }

    async fn req(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Vec<Value>, String> {
        let mut rb = self
            .client
            .request(method.clone(), format!("{}{path}", self.base))
            .header("x-api-key", &self.api_key);
        if let Some(b) = body {
            rb = rb
                .header("content-type", "application/json")
                .body(b.to_string());
        }
        let res = rb.send().await.map_err(|e| pinned::transport_error(&e))?;
        let status = res.status();
        let txt = res.text().await.unwrap_or_default();
        let j: Value = if txt.is_empty() {
            json!({})
        } else {
            serde_json::from_str(&txt).unwrap_or_else(|_| json!({}))
        };
        let meta = j.get("meta");
        let rc = meta.and_then(|m| m.get("rc")).and_then(Value::as_str);
        let rc_bad = rc.is_some_and(|r| !r.is_empty() && r != "ok");
        if !status.is_success() || rc_bad {
            let msg = meta
                .and_then(|m| m.get("msg"))
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map_or_else(|| slice_units(&txt, 120), String::from);
            return Err(format!(
                "UniFi {method} {path} \u{2192} HTTP {} {msg}",
                status.as_u16()
            ));
        }
        Ok(j.get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn list_forwards(&self) -> Result<Vec<Value>, String> {
        self.req(reqwest::Method::GET, "/rest/portforward", None)
            .await
    }

    async fn delete_named(&self, name: &str) -> Result<usize, String> {
        let dupes: Vec<Value> = self
            .list_forwards()
            .await?
            .into_iter()
            .filter(|f| f.get("name").and_then(Value::as_str) == Some(name))
            .collect();
        for f in &dupes {
            let id = f.get("_id").and_then(Value::as_str).unwrap_or("undefined");
            self.req(
                reqwest::Method::DELETE,
                &format!("/rest/portforward/{id}"),
                None,
            )
            .await?;
        }
        Ok(dupes.len())
    }

    /// Create or replace the forward with this name (idempotent: duplicates are removed first).
    pub async fn upsert_forward(&self, f: &Forward<'_>) -> Result<(), String> {
        self.delete_named(f.name).await?;
        let port = f
            .wan_port
            .map_or_else(|| "NaN".to_string(), |p| p.to_string());
        let body = json!({
            "name": f.name, "enabled": true, "pfwd_interface": "wan", "src": "any", "log": false,
            "proto": f.proto, "dst_port": port, "fwd": f.fwd_ip, "fwd_port": port,
        });
        self.req(reqwest::Method::POST, "/rest/portforward", Some(&body))
            .await
            .map(|_| ())
    }

    /// Remove every forward with this name; returns how many there were.
    pub async fn delete_forward_by_name(&self, name: &str) -> Result<usize, String> {
        self.delete_named(name).await
    }
}
