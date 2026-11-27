//! Rust against the original: rcon.py vectors (tests/vectors/rcon.json, from scripts/gen-vectors.py)
//! and the manager's own `read_conf_value` / `write_conf_value` run in bash on temp files.

use std::path::PathBuf;
use std::process::Command;

use ark_ctl::conf::{read_conf_value, write_conf_value};
use ark_ctl::rcon::{FakeTransport, Transport, build_packet, command, player_count, read_packet};
use serde_json::Value;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vectors() -> Value {
    let text =
        std::fs::read_to_string(manifest().join("tests/vectors/rcon.json")).expect("vectors file");
    serde_json::from_str(&text).expect("vectors json")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn packets_equal_rcon_py() {
    for v in vectors()["packets"].as_array().expect("packets") {
        let got = build_packet(
            v["id"].as_i64().expect("id") as i32,
            v["type"].as_i64().expect("type") as i32,
            v["payload"].as_str().expect("payload"),
        );
        assert_eq!(hex(&got), v["hex"].as_str().expect("hex"), "packet {v}");
    }
}

#[test]
fn decoding_equals_rcon_py() {
    for v in vectors()["decode"].as_array().expect("decode") {
        let mut t = FakeTransport::default();
        t.queue(&unhex(v["hex"].as_str().expect("hex")));
        let p = read_packet(&mut t).expect("packet");
        assert_eq!(i64::from(p.id), v["id"].as_i64().expect("id"));
        assert_eq!(i64::from(p.ptype), v["type"].as_i64().expect("type"));
        assert_eq!(p.payload, v["payload"].as_str().expect("payload"));
    }
}

/// A transport that answers a command packet with the vector's replies, using the request id it was sent.
struct Replier {
    replies: Vec<(String, String)>,
    buf: Vec<u8>,
}

impl Transport for Replier {
    fn send(&mut self, data: &[u8]) {
        let rid = i32::from_le_bytes(data[4..8].try_into().expect("id"));
        for (kind, payload) in &self.replies {
            let pid = if kind == "other" { rid + 1 } else { rid };
            self.buf.extend(build_packet(pid, 0, payload));
        }
    }
    fn recv_exact(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.buf.is_empty() {
            return None;
        }
        let take = n.min(self.buf.len());
        Some(self.buf.drain(..take).collect())
    }
    fn set_timeout_secs(&mut self, _secs: f64) {}
}

fn replier(v: &Value, key: &str) -> Replier {
    let replies = v[key].as_array().map_or_else(Vec::new, |a| {
        a.iter()
            .map(|r| {
                (
                    r[0].as_str().expect("kind").to_string(),
                    r[1].as_str().expect("payload").to_string(),
                )
            })
            .collect()
    });
    Replier {
        replies,
        buf: Vec::new(),
    }
}

#[test]
fn command_collection_equals_rcon_py() {
    for v in vectors()["commands"].as_array().expect("commands") {
        let mut t = replier(v, "replies");
        assert_eq!(
            command(&mut t, "Cmd", 42, 10.0),
            v["output"].as_str().expect("output"),
            "case {}",
            v["name"]
        );
    }
}

#[test]
fn player_counts_equal_rcon_py() {
    for v in vectors()["players"].as_array().expect("players") {
        assert_eq!(
            player_count(v["text"].as_str().expect("text")) as i64,
            v["count"].as_i64().expect("count"),
            "case {}",
            v["name"]
        );
    }
}

// ---- the manager's own functions, run in bash ----

fn bash_available() -> bool {
    Command::new("bash")
        .arg("-c")
        .arg("true")
        .status()
        .is_ok_and(|s| s.success())
}

/// The two functions, cut out of ark-manager.sh (sourcing the whole script would run its main section).
fn manager_functions() -> String {
    let script =
        std::fs::read_to_string(manifest().join("../../ark-manager.sh")).expect("ark-manager.sh");
    let mut out = String::from(
        "log_err() { :; }\nsed() { if [ \"$1\" = -i ]; then shift; command sed -i '' \"$@\"; else command sed \"$@\"; fi; }\n",
    );
    for name in ["read_conf_value", "write_conf_value"] {
        let start = script
            .find(&format!("\n{name}() {{"))
            .expect("function start")
            + 1;
        let end = start + script[start..].find("\n}\n").expect("function end") + 3;
        out.push_str(&script[start..end]);
    }
    out
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ark-ctl-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).expect("tmp dir");
    d
}

const READ_CASES: &[(&str, &str, &str)] = &[
    ("A=1\nB=2\nA=3\n", "A", "d"),
    ("A=1\nB=x=y\n", "B", "d"),
    ("A=\nB=2\n", "A", "dflt"),
    ("B=2\n", "A", "dflt"),
    ("AB=1\nB=2\n", "B", ""),
    ("K=with space \n", "K", "d"),
    ("K=caf\u{e9} \u{1f996}\n", "K", "d"),
    ("", "A", "d"),
    ("#K=1\nK=2\n", "K", "d"),
];

#[test]
fn read_conf_value_equals_the_bash_function() {
    if !bash_available() {
        return;
    }
    let dir = scratch("read");
    let funcs = manager_functions();
    for (i, (text, key, default)) in READ_CASES.iter().enumerate() {
        let f = dir.join(format!("c{i}.conf"));
        std::fs::write(&f, text).expect("write");
        let script = format!("{funcs}\nread_conf_value \"$1\" \"$2\" \"$3\"");
        let out = Command::new("bash")
            .arg("-c")
            .arg(script)
            .arg("x")
            .arg(&f)
            .arg(key)
            .arg(default)
            .output()
            .expect("bash");
        let want = String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string();
        assert_eq!(
            read_conf_value(text, key, default),
            want,
            "case {i}: {text:?} {key}"
        );
    }
}

const WRITE_CASES: &[(&str, &str, &str)] = &[
    ("A=1\nB=2\n", "A", "9"),
    ("A=1\nB=2\nA=3\n", "A", "9"),
    ("A=1\n", "B", "2"),
    ("", "B", "2"),
    ("P=old\n", "P", "a|b&c\\d"),
    ("P=old\n", "P", "x/y;z $HOME `id`"),
    ("P=old\nQ=1\n", "Q", "caf\u{e9} \u{1f996}"),
    ("K=1\n", "K", ""),
    ("A.B=1\nAxB=2\n", "AxB", "7"),
];

#[test]
fn write_conf_value_equals_the_bash_function() {
    if !bash_available() {
        return;
    }
    let dir = scratch("write");
    let funcs = manager_functions();
    for (i, (text, key, value)) in WRITE_CASES.iter().enumerate() {
        let f = dir.join(format!("w{i}.conf"));
        std::fs::write(&f, text).expect("write");
        let script = format!("{funcs}\nwrite_conf_value \"$1\" \"$2\" \"$3\"");
        let st = Command::new("bash")
            .arg("-c")
            .arg(script)
            .arg("x")
            .arg(&f)
            .arg(key)
            .arg(value)
            .status()
            .expect("bash");
        assert!(st.success(), "bash failed on case {i}");
        let want = std::fs::read_to_string(&f).expect("read back");
        assert_eq!(
            write_conf_value(text, key, value).expect("write"),
            want,
            "case {i}: {text:?} {key}={value}"
        );
    }
}
