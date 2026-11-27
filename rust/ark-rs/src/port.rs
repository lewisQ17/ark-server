//! The Rust port of `suggestPorts` from `dashboard/backend/ark.js`.
//!
//! `discoverMaps`, `createMap` and `deleteMap` take the executor object (`pmx`), which cannot cross
//! the bridge: they stay in JavaScript and are ported inside `rust/dash` instead.

#![forbid(unsafe_code)]

use serde_json::{Map, Value};

/// `parseInt(String(v), 10) || 0` of a field that may be missing, a string, a number or anything else.
fn port_of(m: &Value, key: &str) -> Result<f64, String> {
    let field = match m {
        Value::Null => {
            return Err(format!(
                "TypeError: Cannot read properties of null (reading '{key}')"
            ));
        }
        Value::Object(o) => o.get(key),
        _ => None, // strings, numbers, booleans and arrays have no such property
    };
    let text = match field {
        None => return Ok(0.0),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => crate::js::number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| match x {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                Value::Number(n) => crate::js::number_to_string(n.as_f64().unwrap_or(f64::NAN)),
                Value::Bool(b) => b.to_string(),
                _ => "[object Object]".to_string(),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    };
    Ok(parse_int(&text))
}

/// `parseInt(text, 10) || 0`: leading blanks, a sign, then digits; anything else is 0.
fn parse_int(text: &str) -> f64 {
    let t = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let (neg, rest) = match t.chars().next() {
        Some('-') => (true, &t[1..]),
        Some('+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return 0.0;
    }
    let n: f64 = digits.parse().unwrap_or(0.0);
    if neg { -n } else { n }
}

/// Port of `suggestPorts` (`dashboard/backend/ark.js:844`).
pub fn suggest_ports(maps: Value) -> Result<Value, String> {
    // A string is iterable too: every character is an "entry" without ports.
    let chars: Vec<Value>;
    let items = match &maps {
        Value::Array(a) => a.as_slice(),
        Value::String(s) => {
            chars = s.chars().map(|c| Value::String(c.to_string())).collect();
            chars.as_slice()
        }
        _ => return Err("TypeError: maps is not iterable".into()),
    };
    let (mut g, mut q, mut r) = (7777.0f64, 27015.0f64, 27020.0f64);
    for m in items {
        g = g.max(port_of(m, "gamePort")?);
        q = q.max(port_of(m, "queryPort")?);
        r = r.max(port_of(m, "rconPort")?);
    }
    let (g, q, r) = if items.is_empty() {
        (7777.0, 27015.0, 27020.0)
    } else {
        (g + 2.0, q + 1.0, r + 1.0)
    };
    let mut out = Map::new();
    out.insert("gamePort".into(), crate::js::number(g));
    out.insert("queryPort".into(), crate::js::number(q));
    out.insert("rconPort".into(), crate::js::number(r));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_list_gives_the_defaults() {
        assert_eq!(
            suggest_ports(json!([])).unwrap(),
            json!({"gamePort": 7777, "queryPort": 27015, "rconPort": 27020})
        );
    }

    #[test]
    fn highest_port_wins_with_the_offsets() {
        let maps = json!([{"gamePort": "7779", "queryPort": 27016, "rconPort": "27021"}, {"gamePort": 7781, "queryPort": "x"}]);
        assert_eq!(
            suggest_ports(maps).unwrap(),
            json!({"gamePort": 7783, "queryPort": 27017, "rconPort": 27022})
        );
    }

    #[test]
    fn lenient_integers_like_parse_int() {
        assert_eq!(parse_int(" 12abc"), 12.0);
        assert_eq!(parse_int("-3"), -3.0);
        assert_eq!(parse_int("abc"), 0.0);
        assert_eq!(parse_int("0x10"), 0.0);
        assert_eq!(parse_int("7.9"), 7.0);
    }

    #[test]
    fn low_ports_do_not_pull_the_defaults_down() {
        assert_eq!(
            suggest_ports(json!([{"gamePort": 1}])).unwrap()["gamePort"],
            7779
        );
    }

    #[test]
    fn a_null_entry_is_an_error_like_the_original() {
        assert!(
            suggest_ports(json!([null]))
                .unwrap_err()
                .starts_with("TypeError")
        );
        assert!(suggest_ports(json!(5)).is_err());
    }
}
