//! Hardening layer (`security.js`): fixed response headers, the Origin check on mutating requests,
//! trusted-proxy client IPs and the delay on repeated failed logins.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use axum::http::{HeaderMap, HeaderValue, Method};
use axum::response::Response;

/// The four fixed headers every response carries (`securityHeaders`).
pub fn apply_headers(r: &mut Response) {
    let h = r.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
}

/// Comma separated list, trimmed, empty entries dropped (`parseTrusted`).
pub fn parse_list(list: &str) -> Vec<String> {
    list.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_mutating(m: &Method) -> bool {
    matches!(
        *m,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// `new URL(origin).host`: lower-cased host plus the port unless it is the scheme's default.
/// Empty when the origin is not a URL (including the literal `null`).
fn origin_host(origin: &str) -> String {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return String::new();
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return String::new();
    }
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit('@').next().unwrap_or("");
    if auth.is_empty() {
        return String::new();
    }
    let lower = auth.to_ascii_lowercase();
    let default = match scheme.as_str() {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        _ => None,
    };
    match lower.rsplit_once(':') {
        Some((h, p)) if !lower.ends_with(']') && Some(p) == default => h.to_string(),
        _ => lower,
    }
}

/// `sameOrigin`: `None` lets the request through, `Some(())` means answer 403.
pub fn cross_origin_refused(method: &Method, headers: &HeaderMap, allowed: &[String]) -> bool {
    if !is_mutating(method) {
        return false;
    }
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if origin.is_empty() || allowed.iter().any(|a| a == origin) {
        return false;
    }
    let host = origin_host(origin);
    let own = ["host", "x-forwarded-host"].iter().filter_map(|n| {
        headers
            .get(*n)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
    });
    let mut own = own;
    !(!host.is_empty() && own.any(|h| h == host))
}

fn normalize(ip: &str) -> &str {
    ip.strip_prefix("::ffff:").unwrap_or(ip)
}

fn ipv4_int(a: &str) -> Option<u32> {
    a.parse::<std::net::Ipv4Addr>().ok().map(u32::from)
}

/// `isTrusted`: exact address, or an IPv4 CIDR.
pub fn is_trusted(peer: &str, trusted: &[String]) -> bool {
    let p = normalize(peer);
    trusted.iter().any(|t| {
        if let Some((base, bits)) = t.split_once('/') {
            let (Some(b), Some(pi), Ok(bits)) = (ipv4_int(base), ipv4_int(p), bits.parse::<u32>())
            else {
                return false;
            };
            if bits > 32 {
                return false;
            }
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            return (b & mask) == (pi & mask);
        }
        normalize(t) == p
    })
}

/// `clientIp`: forwarded headers only count when the direct peer is a trusted proxy.
pub fn client_ip(peer: &str, headers: &HeaderMap, trusted: &[String]) -> String {
    let peer = normalize(peer).to_string();
    if !trusted.is_empty() && is_trusted(&peer, trusted) {
        let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
        if let Some(cf) = get("cf-connecting-ip") {
            let cf = cf.trim();
            if cf.parse::<IpAddr>().is_ok() {
                return cf.to_string();
            }
        }
        let xff = get("x-forwarded-for")
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or("")
            .trim();
        if xff.parse::<IpAddr>().is_ok() {
            return xff.to_string();
        }
    }
    peer
}

/// Growing delay on failed logins per client IP; a good login is never delayed (`authThrottle`).
pub struct Throttle {
    fails: Mutex<HashMap<String, (u64, u64)>>,
    window_ms: u64,
    step_ms: u64,
    max_delay_ms: u64,
}

impl Throttle {
    pub fn new(window_ms: u64, step_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            fails: Mutex::new(HashMap::new()),
            window_ms,
            step_ms,
            max_delay_ms,
        }
    }

    pub fn standard() -> Self {
        Self::new(60_000, 250, 3_000)
    }

    /// Forget stale records for `ip` before a request is judged.
    pub fn expire(&self, ip: &str, now: u64) {
        let mut f = self.fails.lock().expect("throttle lock");
        if f.get(ip)
            .is_some_and(|r| now.saturating_sub(r.1) > self.window_ms)
        {
            f.remove(ip);
        }
    }

    /// Count one failed login and return how long to hold the answer back.
    pub fn failed(&self, ip: &str, now: u64) -> u64 {
        let mut f = self.fails.lock().expect("throttle lock");
        let r = f.entry(ip.to_string()).or_insert((0, now));
        r.0 += 1;
        r.1 = now;
        let n = r.0;
        if f.len() > 5000 {
            f.clear();
        }
        self.max_delay_ms.min(n * self.step_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).expect("name"),
                HeaderValue::from_str(v).expect("value"),
            );
        }
        h
    }

    #[test]
    fn origin_check_matches_the_node_tests() {
        let allowed = vec!["https://ok.example".to_string()];
        let post = Method::POST;
        let h = |o: &str| hm(&[("host", "127.0.0.1:8787"), ("origin", o)]);
        assert!(!cross_origin_refused(
            &post,
            &hm(&[("host", "x")]),
            &allowed
        ));
        assert!(!cross_origin_refused(
            &post,
            &h("http://127.0.0.1:8787"),
            &allowed
        ));
        assert!(!cross_origin_refused(
            &post,
            &h("https://ok.example"),
            &allowed
        ));
        assert!(cross_origin_refused(
            &post,
            &h("https://evil.example"),
            &allowed
        ));
        assert!(cross_origin_refused(&post, &h("null"), &allowed));
        assert!(!cross_origin_refused(
            &Method::GET,
            &h("https://evil.example"),
            &allowed
        ));
        let fwd = hm(&[
            ("host", "10.0.0.1:8787"),
            ("x-forwarded-host", "ark.example, b"),
            ("origin", "https://ark.example"),
        ]);
        assert!(!cross_origin_refused(&Method::DELETE, &fwd, &[]));
    }

    #[test]
    fn origin_host_drops_default_ports_like_url() {
        assert_eq!(origin_host("https://Ark.Example:443"), "ark.example");
        assert_eq!(origin_host("http://a:80"), "a");
        assert_eq!(origin_host("http://a:8080"), "a:8080");
        assert_eq!(origin_host("nonsense"), "");
    }

    #[test]
    fn client_ip_trusts_forwarded_headers_only_from_trusted_peers() {
        let x = hm(&[("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(client_ip("10.0.0.5", &x, &[]), "10.0.0.5");
        assert_eq!(client_ip("10.0.0.5", &x, &["10.0.0.5".into()]), "1.2.3.4");
        let cf = hm(&[("cf-connecting-ip", "5.6.7.8")]);
        assert_eq!(
            client_ip("::ffff:10.0.0.5", &cf, &["10.0.0.0/24".into()]),
            "5.6.7.8"
        );
        assert_eq!(client_ip("9.9.9.9", &x, &["10.0.0.5".into()]), "9.9.9.9");
        let junk = hm(&[("x-forwarded-for", "rommel")]);
        assert_eq!(
            client_ip("10.0.0.5", &junk, &["10.0.0.5".into()]),
            "10.0.0.5"
        );
        assert!(is_trusted("10.0.0.7", &["10.0.0.0/24".into()]));
        assert!(!is_trusted("10.0.1.7", &["10.0.0.0/24".into()]));
    }

    #[test]
    fn throttle_grows_then_caps_and_expires() {
        let t = Throttle::new(1000, 40, 100);
        assert_eq!(t.failed("a", 0), 40);
        assert_eq!(t.failed("a", 10), 80);
        assert_eq!(t.failed("a", 20), 100);
        assert_eq!(t.failed("b", 20), 40);
        t.expire("a", 5000);
        assert_eq!(t.failed("a", 5000), 40);
    }
}
