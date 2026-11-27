//! Source RCON as `rcon.py` speaks it: packet encode/decode, the auth handshake and the command loop,
//! over a [`Transport`] so tests never open a socket.

use std::collections::VecDeque;

pub const SERVERDATA_AUTH: i32 = 3;
pub const SERVERDATA_AUTH_RESPONSE: i32 = 2;
pub const SERVERDATA_EXECCOMMAND: i32 = 2;
pub const SERVERDATA_RESPONSE_VALUE: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub ptype: i32,
    pub payload: String,
}

/// `build_packet`: `<len><id><type><payload>\0\0`, little endian.
pub fn build_packet(request_id: i32, packet_type: i32, payload: &str) -> Vec<u8> {
    let mut body = payload.as_bytes().to_vec();
    body.extend_from_slice(&[0, 0]);
    let length = (4 + 4 + body.len()) as i32;
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&request_id.to_le_bytes());
    out.extend_from_slice(&packet_type.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Where the bytes come from. `read_exact` mirrors `_recv_exact`: it returns what it got,
/// and `None` when nothing arrived (closed or timed out).
pub trait Transport {
    fn send(&mut self, data: &[u8]);
    fn recv_exact(&mut self, n: usize) -> Option<Vec<u8>>;
    fn set_timeout_secs(&mut self, secs: f64);
}

/// `read_packet`: `(None)` when the stream ends. The payload is decoded as UTF-8 with replacement characters.
pub fn read_packet(t: &mut dyn Transport) -> Option<Packet> {
    let raw_len = t.recv_exact(4)?;
    let length = i32::from_le_bytes(raw_len.get(..4)?.try_into().ok()?);
    let body = t.recv_exact(usize::try_from(length).ok()?)?;
    let id = i32::from_le_bytes(body.get(0..4)?.try_into().ok()?);
    let ptype = i32::from_le_bytes(body.get(4..8)?.try_into().ok()?);
    let end = body.len().saturating_sub(2).max(8);
    Some(Packet {
        id,
        ptype,
        payload: String::from_utf8_lossy(&body[8..end]).into_owned(),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum RconError {
    AuthFailed,
}

impl std::fmt::Display for RconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RconError::AuthFailed => {
                write!(f, "RCON authentication failed \u{2014} wrong password?")
            }
        }
    }
}

/// `rcon_connect` after the socket exists: send the password, accept an empty first response.
pub fn authenticate(t: &mut dyn Transport, password: &str) -> Result<(), RconError> {
    t.send(&build_packet(1, SERVERDATA_AUTH, password));
    let mut p = read_packet(t);
    if p.as_ref()
        .is_some_and(|p| p.ptype == SERVERDATA_RESPONSE_VALUE)
    {
        p = read_packet(t);
    }
    match p {
        Some(p) if p.id != -1 && p.ptype == SERVERDATA_AUTH_RESPONSE => Ok(()),
        _ => Err(RconError::AuthFailed),
    }
}

/// `rcon_command`: send, then collect the answer(s) with the matching request id. "keep alive" is skipped.
pub fn command(t: &mut dyn Transport, cmd: &str, req_id: i32, timeout_secs: f64) -> String {
    t.send(&build_packet(req_id, SERVERDATA_EXECCOMMAND, cmd));
    let mut responses = String::new();
    while let Some(p) = read_packet(t) {
        if p.payload.eq_ignore_ascii_case("keep alive") {
            continue;
        }
        if p.id == req_id && p.ptype == SERVERDATA_RESPONSE_VALUE {
            responses.push_str(&p.payload);
            t.set_timeout_secs(1.0);
            if let Some(p2) = read_packet(t)
                && p2.id == req_id
                && !p2.payload.is_empty()
            {
                responses.push_str(&p2.payload);
            }
            t.set_timeout_secs(timeout_secs);
            break;
        }
    }
    responses
}

/// `get_player_count` on a ListPlayers answer: lines starting `N.`; otherwise every non-empty line.
pub fn player_count(listplayers: &str) -> usize {
    if listplayers.is_empty() || listplayers.to_lowercase().contains("no players") {
        return 0;
    }
    let lines: Vec<&str> = listplayers
        .trim()
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let numbered = lines.iter().filter(|l| starts_with_number_dot(l)).count();
    if numbered > 0 { numbered } else { lines.len() }
}

fn starts_with_number_dot(l: &str) -> bool {
    let digits = l.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && l[digits..].starts_with('.')
}

/// The `host:port` argument of rcon.py (`^(.+):(\d+)$`).
pub fn parse_address(addr: &str) -> Option<(String, u16)> {
    let (host, port) = addr.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((host.to_string(), port.parse().ok()?))
}

/// An in-memory server for tests: queued reads, recorded writes. `recv_exact` returns what is left
/// (possibly short) like `_recv_exact` at end of stream, and `None` when empty.
#[derive(Default)]
pub struct FakeTransport {
    pub incoming: VecDeque<u8>,
    pub sent: Vec<Vec<u8>>,
    pub timeouts: Vec<f64>,
}

impl FakeTransport {
    pub fn queue(&mut self, bytes: &[u8]) {
        self.incoming.extend(bytes.iter().copied());
    }
}

impl Transport for FakeTransport {
    fn send(&mut self, data: &[u8]) {
        self.sent.push(data.to_vec());
    }
    fn recv_exact(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.incoming.is_empty() {
            return None;
        }
        let take = n.min(self.incoming.len());
        Some(self.incoming.drain(..take).collect())
    }
    fn set_timeout_secs(&mut self, secs: f64) {
        self.timeouts.push(secs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_layout() {
        let p = build_packet(7, 2, "hi");
        assert_eq!(p, [12, 0, 0, 0, 7, 0, 0, 0, 2, 0, 0, 0, b'h', b'i', 0, 0]);
    }

    #[test]
    fn packet_round_trip_including_unicode() {
        let mut t = FakeTransport::default();
        t.queue(&build_packet(5, 0, "h\u{e9}llo \u{1f996}"));
        assert_eq!(
            read_packet(&mut t),
            Some(Packet {
                id: 5,
                ptype: 0,
                payload: "h\u{e9}llo \u{1f996}".into()
            })
        );
        assert_eq!(read_packet(&mut t), None);
    }

    #[test]
    fn auth_accepts_an_empty_first_response() {
        let mut t = FakeTransport::default();
        t.queue(&build_packet(1, SERVERDATA_RESPONSE_VALUE, ""));
        t.queue(&build_packet(1, SERVERDATA_AUTH_RESPONSE, ""));
        assert_eq!(authenticate(&mut t, "pw"), Ok(()));
        assert_eq!(t.sent[0], build_packet(1, SERVERDATA_AUTH, "pw"));
    }

    #[test]
    fn auth_failure_is_minus_one() {
        let mut t = FakeTransport::default();
        t.queue(&build_packet(-1, SERVERDATA_AUTH_RESPONSE, ""));
        assert_eq!(authenticate(&mut t, "bad"), Err(RconError::AuthFailed));
        let mut t2 = FakeTransport::default();
        assert_eq!(authenticate(&mut t2, "x"), Err(RconError::AuthFailed));
    }

    #[test]
    fn command_skips_keep_alive_and_joins_a_second_packet() {
        let mut t = FakeTransport::default();
        t.queue(&build_packet(9, SERVERDATA_RESPONSE_VALUE, "Keep Alive"));
        t.queue(&build_packet(9, SERVERDATA_RESPONSE_VALUE, "part one "));
        t.queue(&build_packet(9, SERVERDATA_RESPONSE_VALUE, "part two"));
        assert_eq!(command(&mut t, "ListPlayers", 9, 10.0), "part one part two");
        assert_eq!(t.timeouts, [1.0, 10.0]);
    }

    #[test]
    fn players_are_counted_like_the_script() {
        assert_eq!(player_count(""), 0);
        assert_eq!(player_count("No Players Connected"), 0);
        assert_eq!(player_count("0. Alice, 1\n1. Bob, 2\n"), 2);
        assert_eq!(player_count("header\nsomething"), 2); // no numbered lines: every line counts
        assert_eq!(player_count("Header\n1. A, x\nfooter"), 1);
    }

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("localhost:27020"),
            Some(("localhost".into(), 27020))
        );
        assert_eq!(parse_address("a:b:27020"), Some(("a:b".into(), 27020)));
        assert_eq!(parse_address("nohost"), None);
        assert_eq!(parse_address(":27020"), None);
        assert_eq!(parse_address("h:"), None);
    }
}
