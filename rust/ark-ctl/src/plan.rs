//! `add-map` without side effects: parse the options, decide ports and passwords, and return the
//! files the manager would write and the commands it would run. Nothing is executed here.

use crate::conf::read_conf_value;
use crate::{internal_name, unit, valid_name};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AddMapArgs {
    pub display: String,
    pub internal: String,
    pub game_port: String,
    pub query_port: String,
    pub rcon_port: String,
    pub max_players: String,
    pub admin_pw: String,
    pub server_pw: String,
}

/// Parse `add-map <Display> [--internal N] [--game-port N] ...`. A missing option value is an error
/// (the shell script would loop forever on it).
pub fn parse_args(args: &[String]) -> Result<AddMapArgs, String> {
    let mut it = args.iter();
    let display = it.next().cloned().unwrap_or_default();
    if display.is_empty() {
        return Err(
            "Usage: ark-manager.sh add-map <DisplayName> [--internal NAME] [--game-port N] ..."
                .into(),
        );
    }
    let mut a = AddMapArgs {
        display,
        ..AddMapArgs::default()
    };
    while let Some(opt) = it.next() {
        let slot = match opt.as_str() {
            "--internal" => &mut a.internal,
            "--game-port" => &mut a.game_port,
            "--query-port" => &mut a.query_port,
            "--rcon-port" => &mut a.rcon_port,
            "--max-players" => &mut a.max_players,
            "--admin-pw" => &mut a.admin_pw,
            "--server-pw" => &mut a.server_pw,
            other => return Err(format!("Unknown option for add-map: {other}")),
        };
        *slot = it
            .next()
            .cloned()
            .ok_or_else(|| format!("Option {opt} needs a value"))?;
    }
    Ok(a)
}

/// Where passwords come from when none is given (the manager reads /dev/urandom).
pub trait Random {
    fn alnum(&mut self, len: usize) -> String;
}

/// Deterministic fake for tests.
pub struct FakeRandom(pub &'static str);

impl Random for FakeRandom {
    fn alnum(&mut self, len: usize) -> String {
        self.0.chars().cycle().take(len).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The directory exists already: nothing to do (exit 0).
    AlreadyExists,
    Create(Box<Plan>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub map_conf: String,
    pub mods_conf: String,
    pub maps_conf_line: String,
    pub unit_name: String,
    pub unit_text: String,
    pub game_port: u64,
    pub query_port: u64,
    pub rcon_port: u64,
    pub generated_admin_pw: bool,
}

/// What the manager sees of the machine: existing maps' `map.conf` texts and the defaults file.
pub struct World<'a> {
    pub dir_exists: bool,
    pub existing_map_confs: Vec<&'a str>,
    pub defaults_conf: &'a str,
    pub service_user: &'a str,
    pub script_dir: &'a str,
    pub home_dir: &'a str,
}

/// Bash arithmetic on a port string: an integer, or 0 for anything else.
fn arith(s: &str) -> u64 {
    s.trim().parse::<u64>().unwrap_or(0)
}

pub fn plan(a: &AddMapArgs, w: &World<'_>, rng: &mut dyn Random) -> Result<Outcome, String> {
    if !valid_name(&a.display) {
        return Err(format!(
            "Invalid map name '{}' \u{2014} only letters, digits and _ are allowed, no spaces.",
            a.display
        ));
    }
    if w.dir_exists {
        return Ok(Outcome::AlreadyExists);
    }
    let mut internal = a.internal.clone();
    if internal.is_empty() {
        internal = internal_name(&a.display)
            .ok_or_else(|| {
                format!(
                    "'{}' is not a known map. Pass --internal <InternalName> (e.g. MyMap_WP).",
                    a.display
                )
            })?
            .to_string();
    }
    if !valid_name(&internal) {
        return Err(format!(
            "Invalid internal map name '{internal}' \u{2014} only letters, digits and _ are allowed."
        ));
    }
    let pick = |given: &str, key: &str, d: &str| {
        if given.is_empty() {
            read_conf_value(w.defaults_conf, key, d)
        } else {
            given.to_string()
        }
    };
    let mut game = pick(&a.game_port, "DefaultGamePort", "7777");
    let mut query = pick(&a.query_port, "DefaultQueryPort", "27015");
    let mut rcon = pick(&a.rcon_port, "DefaultRCONPort", "27020");
    let max_players = pick(&a.max_players, "DefaultMaxPlayers", "70");
    if !w.existing_map_confs.is_empty() {
        // the manager overrides even explicit ports once other maps exist
        let (mut mg, mut mq, mut mr) = (0, 0, 0);
        for c in &w.existing_map_confs {
            mg = mg.max(arith(&read_conf_value(c, "GamePort", "0")));
            mq = mq.max(arith(&read_conf_value(c, "QueryPort", "0")));
            mr = mr.max(arith(&read_conf_value(c, "RCONPort", "0")));
        }
        game = (mg + 2).to_string();
        query = (mq + 1).to_string();
        rcon = (mr + 1).to_string();
    }
    let mut admin = a.admin_pw.clone();
    if admin.is_empty() {
        admin = read_conf_value(w.defaults_conf, "DefaultAdminPassword", "");
    }
    let mut generated = false;
    if admin.is_empty() {
        admin = rng.alnum(20);
        generated = true;
    }
    let map_conf = format!(
        "MapName={internal}\nDisplayName={d}\nSaveDir={d}\nGamePort={game}\nQueryPort={query}\nRCONPort={rcon}\nMaxPlayers={max_players}\nAdminPassword={admin}\nServerPassword={sp}\nCustomStartParams={cp}\nClusterID={cl}\nBatchEnabled=true\n",
        d = a.display,
        sp = a.server_pw,
        cp = read_conf_value(
            w.defaults_conf,
            "DefaultStartParams",
            "-NoBattlEye -crossplay -NoHangDetection"
        ),
        cl = read_conf_value(w.defaults_conf, "ClusterID", ""),
    );
    Ok(Outcome::Create(Box::new(Plan {
        map_conf,
        mods_conf: format!(
            "# Mod configuration for {0}\n# Format: id|name|enabled\n",
            a.display
        ),
        maps_conf_line: format!("{}|{internal}|enabled", a.display),
        unit_name: unit::service_name(&a.display),
        unit_text: unit::render(&a.display, w.service_user, w.script_dir, w.home_dir),
        game_port: arith(&game),
        query_port: arith(&query),
        rcon_port: arith(&rcon),
        generated_admin_pw: generated,
    })))
}

/// `ufw` commands of `open_firewall_ports` (without the LAN rule, which needs the machine's routes).
pub fn firewall_commands(game: u64, query: u64, rcon: u64) -> Vec<String> {
    vec![
        format!("ufw allow {game}/udp"),
        format!("ufw allow {}/udp", game + 1),
        format!("ufw allow {query}/udp"),
        format!("ufw delete allow {rcon}/tcp"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    fn world<'a>(existing: Vec<&'a str>, defaults: &'a str) -> World<'a> {
        World {
            dir_exists: false,
            existing_map_confs: existing,
            defaults_conf: defaults,
            service_user: "arkadmin",
            script_dir: "/opt/ark",
            home_dir: "/home/arkadmin",
        }
    }

    fn created(o: Outcome) -> Plan {
        match o {
            Outcome::Create(p) => *p,
            Outcome::AlreadyExists => panic!("expected a plan"),
        }
    }

    #[test]
    fn options_parse_and_unknown_ones_fail() {
        let a = parse_args(&s(&["Fjordur", "--game-port", "7790", "--admin-pw", "x"])).unwrap();
        assert_eq!(
            (
                a.display.as_str(),
                a.game_port.as_str(),
                a.admin_pw.as_str()
            ),
            ("Fjordur", "7790", "x")
        );
        assert!(
            parse_args(&s(&["X", "--bogus", "1"]))
                .unwrap_err()
                .contains("Unknown option")
        );
        assert!(
            parse_args(&s(&["X", "--game-port"]))
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn a_known_map_gets_its_internal_name_and_default_ports() {
        let a = parse_args(&s(&["Ragnarok", "--admin-pw", "pw"])).unwrap();
        let p = created(plan(&a, &world(vec![], ""), &mut FakeRandom("ab")).unwrap());
        assert!(p.map_conf.starts_with("MapName=Ragnarok_WP\nDisplayName=Ragnarok\nSaveDir=Ragnarok\nGamePort=7777\nQueryPort=27015\nRCONPort=27020\nMaxPlayers=70\nAdminPassword=pw\n"));
        assert!(p.map_conf.ends_with("ClusterID=\nBatchEnabled=true\n"));
        assert_eq!(p.maps_conf_line, "Ragnarok|Ragnarok_WP|enabled");
        assert_eq!(p.unit_name, "ark-ragnarok.service");
    }

    #[test]
    fn ports_follow_existing_maps_even_when_explicit() {
        let a = parse_args(&s(&["Ragnarok", "--game-port", "9000", "--admin-pw", "pw"])).unwrap();
        let existing = [
            "GamePort=7777\nQueryPort=27015\nRCONPort=27020\n",
            "GamePort=7779\nQueryPort=27016\nRCONPort=junk\n",
        ];
        let p = created(plan(&a, &world(existing.to_vec(), ""), &mut FakeRandom("ab")).unwrap());
        assert_eq!(
            (p.game_port, p.query_port, p.rcon_port),
            (7781, 27017, 27021)
        );
    }

    #[test]
    fn passwords_come_from_args_then_defaults_then_the_generator() {
        let a = parse_args(&s(&["Ragnarok"])).unwrap();
        let p = created(
            plan(
                &a,
                &world(vec![], "DefaultAdminPassword=fromdefaults\n"),
                &mut FakeRandom("ab"),
            )
            .unwrap(),
        );
        assert!(p.map_conf.contains("AdminPassword=fromdefaults\n") && !p.generated_admin_pw);
        let g = created(plan(&a, &world(vec![], ""), &mut FakeRandom("ab")).unwrap());
        assert!(
            g.map_conf.contains("AdminPassword=abababababababababab\n") && g.generated_admin_pw
        );
    }

    #[test]
    fn bad_names_and_unknown_maps_are_refused() {
        let w = world(vec![], "");
        assert!(
            plan(
                &parse_args(&s(&["Bad Name"])).unwrap(),
                &w,
                &mut FakeRandom("a")
            )
            .unwrap_err()
            .contains("Invalid map name")
        );
        assert!(
            plan(
                &parse_args(&s(&["MyMap"])).unwrap(),
                &w,
                &mut FakeRandom("a")
            )
            .unwrap_err()
            .contains("not a known map")
        );
        assert!(
            plan(
                &parse_args(&s(&["MyMap", "--internal", "My Map"])).unwrap(),
                &w,
                &mut FakeRandom("a")
            )
            .unwrap_err()
            .contains("Invalid internal")
        );
        let mut exists = world(vec![], "");
        exists.dir_exists = true;
        assert_eq!(
            plan(
                &parse_args(&s(&["MyMap"])).unwrap(),
                &exists,
                &mut FakeRandom("a")
            )
            .unwrap(),
            Outcome::AlreadyExists
        );
    }

    #[test]
    fn firewall_never_opens_rcon_to_the_world() {
        let c = firewall_commands(7777, 27015, 27020);
        assert!(c.contains(&"ufw allow 7778/udp".to_string()));
        assert!(c.contains(&"ufw delete allow 27020/tcp".to_string()));
        assert!(!c.iter().any(|x| x.starts_with("ufw allow 27020")));
    }
}
