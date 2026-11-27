//! Small helpers that mirror JavaScript semantics the original backend relies on
//! (string coercion, `parseFloat`, `Math.round`, `toFixed`, regex corner cases).

use serde_json::{Map, Number, Value};
use std::sync::OnceLock;

use regex::Regex;

/// JavaScript `\s` (WhiteSpace + LineTerminator).
pub fn js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript line terminators (what `.` refuses to match).
pub fn js_lt(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub fn js_trim(s: &str) -> &str {
    s.trim_matches(js_ws)
}

/// `'...'` quoting for POSIX shells, as `shq()` in ark.js.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Proxmox guest-exec cannot carry non-ASCII: transliterate typographic characters, drop the rest.
pub fn ascii_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{2012}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2018}' | '\u{2019}' | '\u{201b}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            '\t' | '\n' | '\r' | ' '..='~' => out.push(c),
            _ => {}
        }
    }
    out
}

/// ASCII-case-insensitive version of a literal, as a regex fragment (JS `/i` without `u`).
pub fn ci(lit: &str) -> String {
    lit.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else if c == ' ' {
                " ".to_string()
            } else {
                regex::escape(&c.to_string())
            }
        })
        .collect()
}

fn secrets_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        let pat = format!(
            "(({s})?({a})?{p}|{r}{p}|{s}{a}{p})=([^\\s?\"&]+)",
            s = ci("Server"),
            a = ci("Admin"),
            p = ci("Password"),
            r = ci("RCON")
        );
        Regex::new(&pat).expect("secrets regex")
    })
}

/// Hide the secrets ARK writes into its own log and config.
pub fn redact_secrets(text: &str) -> String {
    secrets_re()
        .replace_all(text, |c: &regex::Captures| {
            let whole = c.get(0).map_or("", |m| m.as_str());
            let key_len = whole.find('=').unwrap_or(0);
            format!(
                "{}=\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}",
                &whole[..key_len]
            )
        })
        .into_owned()
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `^\s*([A-Za-z0-9_]+)\s*=\s*(.*?)\s*$` on one line (the value is trimmed).
pub fn match_kv_trimmed(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = split_kv(line)?;
    let v = js_trim(rest);
    if v.chars().any(js_lt) {
        return None;
    }
    Some((key, v))
}

/// `^(\s*)([A-Za-z0-9_]+)(\s*=\s*)(.*)$`: (lead, key, mid, value). The value may not hold a line terminator.
pub fn match_kv_greedy(line: &str) -> Option<(&str, &str, &str, &str)> {
    let t = line.trim_start_matches(js_ws);
    let lead = &line[..line.len() - t.len()];
    let kend = t.find(|c: char| !is_key_char(c)).unwrap_or(t.len());
    if kend == 0 {
        return None;
    }
    let key = &t[..kend];
    let after = &t[kend..];
    let a2 = after.trim_start_matches(js_ws);
    let a2 = a2.strip_prefix('=')?;
    let a3 = a2.trim_start_matches(js_ws);
    let mid_len = after.len() - a3.len();
    let mid = &after[..mid_len];
    if a3.chars().any(js_lt) {
        return None;
    }
    Some((lead, key, mid, a3))
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start_matches(js_ws);
    let kend = t.find(|c: char| !is_key_char(c)).unwrap_or(t.len());
    if kend == 0 {
        return None;
    }
    let after = t[kend..].trim_start_matches(js_ws);
    let after = after.strip_prefix('=')?;
    Some((&t[..kend], after))
}

/// `parseConf` of ark.js: KEY=VALUE lines into an ordered object, last value wins.
pub fn parse_conf(text: &str) -> Map<String, Value> {
    let mut o = Map::new();
    for l in text.split('\n') {
        if let Some((k, v)) = match_kv_trimmed(l) {
            o.insert(k.to_string(), Value::String(v.to_string()));
        }
    }
    o
}

pub fn conf_get<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// A JSON number the way `JSON.stringify` prints a JS number (integral values without `.0`).
pub fn num(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        // -0 prints as 0 in JS
        return Value::Number(Number::from(x as i64));
    }
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

/// `String(number)`.
pub fn num_str(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if x == 0.0 {
        return "0".into();
    }
    let a = x.abs();
    if (1e-6..1e21).contains(&a) {
        if x.fract() == 0.0 && a < 1e21 {
            return format!("{x:.0}");
        }
        return format!("{x}");
    }
    // exponent form: 1.5e-7, 1e+21
    let s = format!("{x:e}");
    match s.split_once('e') {
        Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
        _ => s,
    }
}

/// `String(value)` for the JSON values a request body can carry.
pub fn js_str(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map_or_else(
            || n.to_string(),
            |f| {
                if let Some(i) = n.as_i64() {
                    i.to_string()
                } else {
                    num_str(f)
                }
            },
        ),
        Value::String(s) => s.clone(),
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
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `String(x)` for an optional field (undefined gives "undefined").
pub fn js_str_opt(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".to_string(), js_str)
}

/// JS truthiness of a JSON value (`undefined` is `None`).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `Number(value)`.
pub fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js_string_to_number(s),
        Some(Value::Array(a)) => match a.len() {
            0 => 0.0,
            1 => js_number(Some(&a[0])),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

fn js_string_to_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    for (p, r) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(d) = t.strip_prefix(p) {
            return u64::from_str_radix(d, r).map_or(f64::NAN, |n| n as f64);
        }
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if t.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
    {
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// `parseFloat(string)`: longest valid decimal prefix.
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(js_ws);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if t[i..].starts_with("Infinity") {
        return if b.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - ds;
    if i < b.len() && b[i] == b'.' {
        let save = i;
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - fs;
        if i - fs == 0 && digits == 0 {
            i = save;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            i = j;
        }
    }
    t[..i].parse::<f64>().unwrap_or(f64::NAN)
}

/// `parseInt(string, 10)`.
pub fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim_start_matches(js_ws);
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let n: i64 = rest[..end].parse().ok()?;
    Some(if neg { -n } else { n })
}

/// `Math.round`: halves go up.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    (x + 0.5).floor()
}

/// `x.toFixed(digits)` for small digit counts; exact ties go up like V8.
pub fn to_fixed(x: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    let scaled = x * p;
    let fl = scaled.floor();
    let diff = scaled - fl;
    let r = if diff >= 0.5 { fl + 1.0 } else { fl };
    r / p
}

/// The uptime string of the dashboard: `3d 4h`, `5h 12m`, `7m`, or an em dash for nothing.
pub fn uptime_str(s: f64) -> String {
    if s == 0.0 || s.is_nan() {
        return "\u{2014}".into();
    }
    let d = (s / 86400.0).floor();
    let h = ((s % 86400.0) / 3600.0).floor();
    let m = ((s % 3600.0) / 60.0).floor();
    if d != 0.0 {
        return format!("{}d {}h", d as i64, h as i64);
    }
    if h != 0.0 {
        return format!("{}h {}m", h as i64, m as i64);
    }
    format!("{}m", m as i64)
}

const EMBLEMS: [&str; 7] = [
    "#FF7A2E", "#37D67A", "#FFB23E", "#8C7BF7", "#37D3C3", "#5AA9FF", "#FF5B5B",
];

/// A stable colour per map name (31-hash over the first UTF-16 unit of every code point).
pub fn emblem_for(name: &str) -> &'static str {
    let mut h: u32 = 0;
    for c in name.chars() {
        let mut buf = [0u16; 2];
        let unit = c.encode_utf16(&mut buf)[0];
        h = (h.wrapping_mul(31).wrapping_add(u32::from(unit))) & 0xffff;
    }
    EMBLEMS[(h as usize) % EMBLEMS.len()]
}

struct FeedRes {
    joined: Regex,
    left: Regex,
    save: Regex,
    adv: Regex,
    done: Regex,
}

fn feed_res() -> &'static FeedRes {
    static R: OnceLock<FeedRes> = OnceLock::new();
    R.get_or_init(|| FeedRes {
        joined: Regex::new(&format!(
            "{}\"?([^\"]+?)\"? {}",
            ci("Player "),
            ci("joined")
        ))
        .expect("re"),
        left: Regex::new(&format!("{}\"?([^\"]+?)\"? {}", ci("Player "), ci("left"))).expect("re"),
        save: Regex::new(&format!("{}|{}", ci("World Save"), ci("Saving world"))).expect("re"),
        adv: Regex::new(&ci("advertising for join")).expect("re"),
        done: Regex::new(&ci("completed startup")).expect("re"),
    })
}

fn feed_item(who: &str, action: &str, map: &str, ty: &str) -> Value {
    let mut m = Map::new();
    m.insert("who".into(), who.into());
    m.insert("action".into(), action.into());
    m.insert("map".into(), map.into());
    m.insert("type".into(), ty.into());
    Value::Object(m)
}

/// Turn the tail of a server log into at most 8 feed entries, newest first.
pub fn parse_feed(lines: &[String], map_name: &str) -> Vec<Value> {
    let re = feed_res();
    let start = lines.len().saturating_sub(160);
    let mut feed = Vec::new();
    for line in lines[start..].iter().rev() {
        if let Some(c) = re.joined.captures(line) {
            feed.push(feed_item(&c[1], "joined the server", map_name, "join"));
        } else if let Some(c) = re.left.captures(line) {
            feed.push(feed_item(&c[1], "left the server", map_name, "leave"));
        } else if re.save.is_match(line) {
            feed.push(feed_item("Server", "saved the world", map_name, "sys"));
        } else if re.adv.is_match(line) {
            feed.push(feed_item(
                "Server",
                "is online \u{2014} advertising for join",
                map_name,
                "sys",
            ));
        } else if re.done.is_match(line) {
            feed.push(feed_item("Server", "completed startup", map_name, "sys"));
        }
        if feed.len() >= 8 {
            break;
        }
    }
    feed
}

/// Next free ports given the existing maps (`suggestPorts`).
pub fn suggest_ports(maps: &[Map<String, Value>]) -> Value {
    let (mut g, mut q, mut r) = (7777i64, 27015i64, 27020i64);
    let port = |m: &Map<String, Value>, k: &str| -> i64 {
        match m.get(k) {
            Some(Value::String(s)) => parse_int(s).unwrap_or(0),
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            _ => 0,
        }
    };
    for m in maps {
        g = g.max(port(m, "gamePort"));
        q = q.max(port(m, "queryPort"));
        r = r.max(port(m, "rconPort"));
    }
    let mut o = Map::new();
    if maps.is_empty() {
        o.insert("gamePort".into(), 7777.into());
        o.insert("queryPort".into(), 27015.into());
        o.insert("rconPort".into(), 27020.into());
    } else {
        o.insert("gamePort".into(), (g + 2).into());
        o.insert("queryPort".into(), (q + 1).into());
        o.insert("rconPort".into(), (r + 1).into());
    }
    Value::Object(o)
}

/// Milliseconds since the epoch.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `systemd` timestamp such as `Thu 2026-11-26 10:30:00 UTC` (weekday stripped by the caller) to epoch ms.
pub fn parse_systemd_ts(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = match t.split_once(' ') {
        Some((w, rest)) if w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => rest,
        _ => t,
    };
    let mut it = t.split_whitespace();
    let date = it.next()?;
    let time = it.next()?;
    let mut dp = date.split('-');
    let (y, mo, d) = (
        dp.next()?.parse::<i64>().ok()?,
        dp.next()?.parse::<i64>().ok()?,
        dp.next()?.parse::<i64>().ok()?,
    );
    let mut tp = time.split(':');
    let (h, mi, se) = (
        tp.next()?.parse::<i64>().ok()?,
        tp.next()?.parse::<i64>().ok()?,
        tp.next()?.parse::<i64>().ok()?,
    );
    Some(((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se) * 1000) as f64)
}

pub fn fmt_systemd_ts(epoch_s: i64) -> String {
    let days = epoch_s.div_euclid(86400);
    let rem = epoch_s.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "Thu {y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Lenient base64 decode like Node's `Buffer.from(s, 'base64')` (accepts url-safe, ignores junk, stops at `=`).
pub fn lenient_base64(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

/// UTF-16-unit aware `slice(0, n)`.
pub fn slice_units(s: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > n {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_safe_maps_typography() {
        assert_eq!(
            ascii_safe("Caf\u{e9} \u{201c}x\u{201d} \u{2014} \u{1f996}\u{2026}"),
            "Caf \"x\" - ..."
        );
    }

    #[test]
    fn redact_hides_secrets() {
        assert_eq!(
            redact_secrets("ServerAdminPassword=hunter2 RCONPassword=abc&x serverpassword=z"),
            "ServerAdminPassword=\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022} RCONPassword=\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}&x serverpassword=\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"
        );
        assert_eq!(
            redact_secrets("ADMINPASSWORD=a"),
            "ADMINPASSWORD=\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"
        );
        assert_eq!(redact_secrets("Password=\n"), "Password=\n");
    }

    #[test]
    fn parse_conf_last_wins_and_trims() {
        let m = parse_conf("A=1\n  B = two words  \r\nA=3\nbad line\nC=x\ry\n");
        assert_eq!(conf_get(&m, "A"), Some("3"));
        assert_eq!(conf_get(&m, "B"), Some("two words"));
        assert_eq!(conf_get(&m, "C"), None);
        assert_eq!(m.keys().collect::<Vec<_>>(), ["A", "B"]);
    }

    #[test]
    fn greedy_kv_rejects_cr_in_value() {
        assert!(match_kv_greedy("K=v\r").is_none());
        assert_eq!(
            match_kv_greedy("  K = v w"),
            Some(("  ", "K", " = ", "v w"))
        );
    }

    #[test]
    fn number_strings() {
        assert_eq!(num_str(2.0), "2");
        assert_eq!(num_str(1.5), "1.5");
        assert_eq!(num_str(1e21), "1e+21");
        assert_eq!(num_str(1.5e-7), "1.5e-7");
        assert_eq!(num_str(0.000001), "0.000001");
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(to_fixed(0.25, 1), 0.3);
    }

    #[test]
    fn parse_float_prefix() {
        assert_eq!(parse_float(" 2.5abc"), 2.5);
        assert_eq!(parse_float(".5"), 0.5);
        assert!(parse_float("abc").is_nan());
        assert!(parse_float(".").is_nan());
        assert_eq!(parse_float("1e3x"), 1000.0);
        assert_eq!(parse_float("-Infinityx"), f64::NEG_INFINITY);
    }

    #[test]
    fn js_number_coercion() {
        assert_eq!(js_number(Some(&Value::String(" 3 ".into()))), 3.0);
        assert_eq!(js_number(Some(&Value::String(String::new()))), 0.0);
        assert!(js_number(None).is_nan());
        assert!(js_number(Some(&Value::String("nan".into()))).is_nan());
        assert_eq!(js_number(Some(&Value::Null)), 0.0);
    }

    #[test]
    fn uptime_formats() {
        assert_eq!(uptime_str(0.0), "\u{2014}");
        assert_eq!(uptime_str(5400.0), "1h 30m");
        assert_eq!(uptime_str(90000.0), "1d 1h");
        assert_eq!(uptime_str(120.0), "2m");
    }

    #[test]
    fn emblem_is_stable() {
        assert_eq!(emblem_for("TheIsland"), emblem_for("TheIsland"));
        assert!(EMBLEMS.contains(&emblem_for("\u{1f996}")));
    }

    #[test]
    fn feed_parses_newest_first() {
        let lines: Vec<String> = [
            "Player \"Alice\" joined the server",
            "World Save complete",
            "Player Bob left the server",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let f = parse_feed(&lines, "M");
        assert_eq!(f[0]["who"], "Bob");
        assert_eq!(f[0]["type"], "leave");
        assert_eq!(f[1]["action"], "saved the world");
        assert_eq!(f[2]["who"], "Alice");
    }

    #[test]
    fn ports_follow_the_highest() {
        assert_eq!(
            suggest_ports(&[]).to_string(),
            r#"{"gamePort":7777,"queryPort":27015,"rconPort":27020}"#
        );
        let mut m = Map::new();
        m.insert("gamePort".into(), "7779".into());
        assert_eq!(suggest_ports(&[m])["gamePort"], 7781);
    }

    #[test]
    fn dates_round_trip() {
        let s = fmt_systemd_ts(1_795_000_000);
        assert_eq!(parse_systemd_ts(&s), Some(1_795_000_000_000.0));
    }

    #[test]
    fn base64_lenient() {
        assert_eq!(lenient_base64("YWRtaW46eA"), b"admin:x");
        assert_eq!(lenient_base64("YWRtaW46eA=="), b"admin:x");
    }
}
