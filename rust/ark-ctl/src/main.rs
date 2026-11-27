//! `ark-ctl`: the side-effect-free parts of `ark-manager.sh` and `rcon.py` as a CLI.
//! Commands that would start, stop or reconfigure anything only print what they would do.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use ark_ctl::launch::{self, Paths};
use ark_ctl::plan::{self, FakeRandom, Outcome, World};
use ark_ctl::{conf, format_uptime, rcon, unit};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ark-ctl",
    version,
    about = "Pure parts of the ARK server manager (no process is started, no service touched)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the value of KEY in a KEY=VALUE file (last match; DEFAULT when missing or empty)
    ConfGet {
        file: PathBuf,
        key: String,
        default: Option<String>,
    },
    /// Set KEY=VALUE in a file (every matching line is replaced, otherwise it is appended)
    ConfSet {
        file: PathBuf,
        key: String,
        value: String,
    },
    /// Source-RCON packets
    Packet {
        #[command(subcommand)]
        what: PacketCmd,
    },
    /// Count the players in a ListPlayers answer read from stdin
    PlayerCount,
    /// Split host:port the way rcon.py does
    ParseAddress { addr: String },
    /// `3d 4h` style uptime from elapsed seconds
    Uptime { seconds: u64 },
    /// Print the systemd unit of a map
    Unit {
        map: String,
        #[arg(long, default_value = "arkadmin")]
        user: String,
        #[arg(long, default_value = "/home/arkadmin/ark-manager")]
        script_dir: String,
        #[arg(long, default_value = "/home/arkadmin")]
        home: String,
    },
    /// Print the command line `start` would run for a map (nothing is started)
    LaunchPlan {
        map: String,
        /// The manager's directory (holds maps/, server-files/, config/)
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "GE-Proton10-4")]
        proton: String,
    },
    /// Show what `add-map` would create (nothing is written, nothing is run)
    AddMap {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "arkadmin")]
        user: String,
        #[arg(long, default_value = "/home/arkadmin")]
        home: String,
        /// <DisplayName> [--internal N] [--game-port N] [--query-port N] [--rcon-port N] [--max-players N] [--admin-pw PW] [--server-pw PW]
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PacketCmd {
    /// Hex of a packet: ark-ctl packet encode <id> <type> <payload>
    Encode {
        id: i32,
        ptype: i32,
        payload: String,
    },
    /// Decode one packet from hex
    Decode { hex: String },
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err("odd number of hex digits".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::ConfGet { file, key, default } => {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            println!(
                "{}",
                conf::read_conf_value(&text, &key, default.as_deref().unwrap_or(""))
            );
        }
        Cmd::ConfSet { file, key, value } => {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let out = conf::write_conf_value(&text, &key, &value).map_err(|_| {
                format!("Value for '{key}' contains a newline \u{2014} refusing to write it.")
            })?;
            std::fs::write(&file, out).map_err(|e| e.to_string())?;
        }
        Cmd::Packet {
            what: PacketCmd::Encode { id, ptype, payload },
        } => println!("{}", to_hex(&rcon::build_packet(id, ptype, &payload))),
        Cmd::Packet {
            what: PacketCmd::Decode { hex },
        } => {
            let mut t = rcon::FakeTransport::default();
            t.queue(&from_hex(&hex)?);
            let p = rcon::read_packet(&mut t).ok_or("incomplete packet")?;
            println!(
                "{}",
                serde_json::json!({"id": p.id, "type": p.ptype, "payload": p.payload})
            );
        }
        Cmd::PlayerCount => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| e.to_string())?;
            println!("{}", rcon::player_count(&s));
        }
        Cmd::ParseAddress { addr } => match rcon::parse_address(&addr) {
            Some((h, p)) => println!("{h} {p}"),
            None => return Err("Error: address must be host:port".into()),
        },
        Cmd::Uptime { seconds } => println!("{}", format_uptime(seconds)),
        Cmd::Unit {
            map,
            user,
            script_dir,
            home,
        } => {
            if !ark_ctl::valid_name(&map) {
                return Err(format!("Invalid map name '{map}'"));
            }
            print!("{}", unit::render(&map, &user, &script_dir, &home));
        }
        Cmd::LaunchPlan { map, dir, proton } => {
            let d = dir.to_string_lossy().trim_end_matches('/').to_string();
            let p = Paths::under(&d, &proton);
            let map_conf = std::fs::read_to_string(format!("{}/{map}/map.conf", p.maps_dir))
                .map_err(|_| format!("Map '{map}' not found."))?;
            let opt = std::fs::read_to_string(format!("{d}/config/optimization.conf"))
                .unwrap_or_default();
            let mods = std::fs::read_to_string(format!("{}/{map}/mods.conf", p.maps_dir)).ok();
            let lp = launch::plan(&map, &map_conf, &opt, mods.as_deref(), &p);
            for (k, v) in &lp.env {
                println!("export {k}={v}");
            }
            println!("{}", lp.argv.join(" "));
            println!("# log: {}", lp.log_file);
            eprintln!("(plan only: nothing was started)");
        }
        Cmd::AddMap {
            dir,
            user,
            home,
            args,
        } => {
            let d = dir.to_string_lossy().trim_end_matches('/').to_string();
            let a = plan::parse_args(&args)?;
            let maps_dir = format!("{d}/maps");
            let confs: Vec<String> = std::fs::read_dir(&maps_dir)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter(|e| e.path().is_dir())
                        .filter_map(|e| std::fs::read_to_string(e.path().join("map.conf")).ok())
                        .collect()
                })
                .unwrap_or_default();
            let defaults = std::fs::read_to_string(format!("{d}/config/server-defaults.conf"))
                .unwrap_or_default();
            let world = World {
                dir_exists: std::path::Path::new(&format!("{maps_dir}/{}", a.display)).is_dir(),
                existing_map_confs: confs.iter().map(String::as_str).collect(),
                defaults_conf: &defaults,
                service_user: &user,
                script_dir: &d,
                home_dir: &home,
            };
            // a dry run must never invent a password that looks real
            match plan::plan(&a, &world, &mut FakeRandom("DRYRUN"))? {
                Outcome::AlreadyExists => {
                    println!("Map '{}' already exists \u{2014} nothing to do.", a.display)
                }
                Outcome::Create(p) => {
                    println!(
                        "# {maps_dir}/{}/map.conf\n{}",
                        a.display,
                        if p.generated_admin_pw {
                            p.map_conf
                                .replace("DRYRUNDRYRUNDRYRUNDRYR", "<generated at run time>")
                        } else {
                            p.map_conf.clone()
                        }
                    );
                    println!("# {maps_dir}/{}/mods.conf\n{}", a.display, p.mods_conf);
                    println!("# {d}/config/maps.conf (append)\n{}\n", p.maps_conf_line);
                    println!("# /etc/systemd/system/{}\n{}", p.unit_name, p.unit_text);
                    println!("# firewall");
                    for c in plan::firewall_commands(p.game_port, p.query_port, p.rcon_port) {
                        println!("{c}");
                    }
                    eprintln!("(plan only: nothing was written or run)");
                }
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
