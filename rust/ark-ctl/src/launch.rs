//! What `start_map` would run, without running it: the launch plan, the running-process pattern,
//! the enabled mod ids and the password rewrite of GameUserSettings.ini.

use crate::conf::read_conf_value;

pub const ARK_APPID: &str = "2430930";

#[derive(Debug, Clone)]
pub struct Paths {
    pub script_dir: String,
    pub server_dir: String,
    pub steamcmd_dir: String,
    pub proton_dir: String,
    pub maps_dir: String,
}

impl Paths {
    /// The default layout under the manager's own directory (`SCRIPT_DIR`).
    pub fn under(script_dir: &str, proton_version: &str) -> Self {
        Self {
            script_dir: script_dir.into(),
            server_dir: format!("{script_dir}/server-files"),
            steamcmd_dir: format!("{script_dir}/steamcmd"),
            proton_dir: format!("{script_dir}/{proton_version}"),
            maps_dir: format!("{script_dir}/maps"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub env: Vec<(String, String)>,
    /// `nice <level> proton run ...`
    pub argv: Vec<String>,
    pub log_file: String,
    pub game_port: String,
}

/// Whitespace word splitting of an unquoted shell variable.
fn words(s: &str) -> Vec<String> {
    s.split([' ', '\t', '\n'])
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// `get_enabled_mod_ids`: `id|name|enabled` lines (a last line without newline is not read, like `while read`).
pub fn enabled_mod_ids(mods_conf: Option<&str>, map_conf: &str) -> String {
    let Some(text) = mods_conf else {
        return read_conf_value(map_conf, "ModIDs", "");
    };
    let mut ids: Vec<&str> = Vec::new();
    let complete = match text.rfind('\n') {
        Some(i) => &text[..i],
        None => "",
    };
    if !text.is_empty() && text.contains('\n') {
        for line in complete.split('\n') {
            let mut f = line.splitn(3, '|');
            let id = f.next().unwrap_or("");
            let _name = f.next().unwrap_or("");
            let enabled = f.next().unwrap_or("");
            if id.starts_with('#') || id.is_empty() {
                continue;
            }
            if enabled == "true" {
                ids.push(id);
            }
        }
    }
    ids.join(",")
}

/// Build the launch plan from `map.conf`, `optimization.conf` and `mods.conf` texts.
pub fn plan(
    map: &str,
    map_conf: &str,
    opt_conf: &str,
    mods_conf: Option<&str>,
    p: &Paths,
) -> LaunchPlan {
    let r = |k: &str, d: &str| read_conf_value(map_conf, k, d);
    let map_name = r("MapName", "");
    let save_dir = r("SaveDir", map);
    let game_port = r("GamePort", "7777");
    let query_port = r("QueryPort", "27015");
    let rcon_port = r("RCONPort", "27020");
    let max_players = r("MaxPlayers", "70");
    let custom_params = r(
        "CustomStartParams",
        "-NoBattlEye -crossplay -NoHangDetection",
    );
    let cluster_id = r("ClusterID", "");
    let mod_ids = enabled_mod_ids(mods_conf, map_conf);

    let mut cluster_args: Vec<String> = Vec::new();
    if !cluster_id.is_empty() {
        let cluster_dir = format!("{}/clusters/{cluster_id}", p.script_dir);
        // the manager keeps the double quotes inside the words (an unquoted expansion)
        cluster_args = words(&format!(
            "-ClusterDirOverride=\"{cluster_dir}\" -ClusterId=\"{cluster_id}\""
        ));
    }
    let nullrhi = if read_conf_value(opt_conf, "UseNullRHI", "true") == "true" {
        vec!["-nullrhi".to_string()]
    } else {
        vec![]
    };
    let nice_lvl = read_conf_value(opt_conf, "ServerNiceLevel", "-5");

    let mut argv: Vec<String> = vec!["nice".into()];
    argv.extend(words(&nice_lvl));
    argv.push(format!("{}/proton", p.proton_dir));
    argv.push("run".into());
    argv.push(format!(
        "{}/ShooterGame/Binaries/Win64/ArkAscendedServer.exe",
        p.server_dir
    ));
    argv.push(format!(
        "{map_name}?listen?SessionName={map}?AltSaveDirectoryName={save_dir}"
    ));
    argv.extend(words(&custom_params));
    argv.extend(nullrhi);
    argv.push(format!("-WinLiveMaxPlayers={max_players}"));
    argv.push(format!("-Port={game_port}"));
    argv.push(format!("-QueryPort={query_port}"));
    argv.push(format!("-RCONPort={rcon_port}"));
    argv.push("-game".into());
    argv.extend(cluster_args);
    argv.extend(["-server", "-log"].map(String::from));
    argv.push(format!("-mods={mod_ids}"));

    LaunchPlan {
        env: vec![
            (
                "STEAM_COMPAT_DATA_PATH".into(),
                format!("{}/steamapps/compatdata/{ARK_APPID}", p.server_dir),
            ),
            (
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".into(),
                p.steamcmd_dir.clone(),
            ),
            ("SteamAppId".into(), ARK_APPID.into()),
            ("SteamGameId".into(), ARK_APPID.into()),
        ],
        argv,
        log_file: format!("{}/{map}/server.log", p.maps_dir),
        game_port,
    }
}

/// `pgrep -f` pattern of `is_map_running`: the save dir must end the argument.
pub fn running_pattern(save_dir: &str) -> String {
    format!("ArkAscendedServer\\.exe.*AltSaveDirectoryName={save_dir}( |$)")
}

/// Does a process command line belong to this save dir (what the pattern above means)?
pub fn matches_cmdline(save_dir: &str, cmdline: &str) -> bool {
    let Some(start) = cmdline.find("ArkAscendedServer.exe") else {
        return false;
    };
    let rest = &cmdline[start + "ArkAscendedServer.exe".len()..];
    let needle = format!("AltSaveDirectoryName={save_dir}");
    let mut from = 0;
    while let Some(i) = rest[from..].find(&needle) {
        let end = from + i + needle.len();
        if rest[end..].is_empty() || rest[end..].starts_with(' ') {
            return true;
        }
        from += i + 1;
    }
    false
}

/// The awk program of `start_map`: drop the four credential keys, then put them under `[ServerSettings]`
/// (creating the section when the file has none).
pub fn rewrite_gus(text: &str, server_pw: &str, admin_pw: &str, rcon_port: &str) -> String {
    let emit = |out: &mut Vec<String>| {
        out.push(format!("ServerPassword={server_pw}"));
        out.push(format!("ServerAdminPassword={admin_pw}"));
        out.push("RCONEnabled=True".into());
        out.push(format!("RCONPort={rcon_port}"));
    };
    let mut records: Vec<&str> = text.split('\n').collect();
    if records.last() == Some(&"") {
        records.pop();
    }
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for l in records {
        if [
            "ServerPassword=",
            "ServerAdminPassword=",
            "RCONEnabled=",
            "RCONPort=",
        ]
        .iter()
        .any(|k| l.starts_with(k))
        {
            continue;
        }
        out.push(l.to_string());
        if !done
            && l.strip_prefix("[ServerSettings]").is_some_and(|r| {
                r.chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\r' | '\u{b}' | '\u{c}'))
            })
        {
            emit(&mut out);
            done = true;
        }
    }
    if !done {
        out.push("[ServerSettings]".into());
        emit(&mut out);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths::under("/opt/ark", "GE-Proton10-4")
    }

    #[test]
    fn defaults_give_the_documented_command_line() {
        let p = plan("Island", "MapName=TheIsland_WP\n", "", None, &paths());
        assert_eq!(p.argv[0..2], ["nice", "-5"]);
        assert_eq!(p.argv[2], "/opt/ark/GE-Proton10-4/proton");
        assert_eq!(
            p.argv[5],
            "TheIsland_WP?listen?SessionName=Island?AltSaveDirectoryName=Island"
        );
        assert!(p.argv.contains(&"-nullrhi".to_string()));
        assert!(p.argv.contains(&"-Port=7777".to_string()));
        assert_eq!(p.argv.last().map(String::as_str), Some("-mods="));
        assert_eq!(p.log_file, "/opt/ark/maps/Island/server.log");
    }

    #[test]
    fn options_change_the_plan() {
        let conf = "MapName=Ragnarok_WP\nSaveDir=Rag\nGamePort=7779\nMaxPlayers=10\nCustomStartParams=-a  -b\nClusterID=c1\n";
        let p = plan(
            "Rag",
            conf,
            "UseNullRHI=false\nServerNiceLevel=0\n",
            Some("# c\n11|a|true\n22|b|false\n33|c|true\n"),
            &paths(),
        );
        assert!(!p.argv.contains(&"-nullrhi".to_string()));
        assert_eq!(p.argv[1], "0");
        assert!(p.argv.contains(&"-a".to_string()) && p.argv.contains(&"-b".to_string()));
        assert!(
            p.argv
                .contains(&"-ClusterDirOverride=\"/opt/ark/clusters/c1\"".to_string())
        );
        assert_eq!(p.argv.last().map(String::as_str), Some("-mods=11,33"));
        assert!(p.argv.contains(&"-WinLiveMaxPlayers=10".to_string()));
    }

    #[test]
    fn mods_fall_back_to_the_conf_and_skip_an_unterminated_line() {
        assert_eq!(enabled_mod_ids(None, "ModIDs=1,2\n"), "1,2");
        assert_eq!(enabled_mod_ids(Some("1|a|true\n2|b|true"), ""), "1");
        assert_eq!(enabled_mod_ids(Some("1|a|true|x\n"), ""), "");
        assert_eq!(enabled_mod_ids(Some(""), "ModIDs=9\n"), "");
    }

    #[test]
    fn running_pattern_is_anchored() {
        assert_eq!(
            running_pattern("Rag"),
            "ArkAscendedServer\\.exe.*AltSaveDirectoryName=Rag( |$)"
        );
        assert!(matches_cmdline(
            "Rag",
            "x ArkAscendedServer.exe Map?AltSaveDirectoryName=Rag -log"
        ));
        assert!(matches_cmdline(
            "Rag",
            "ArkAscendedServer.exe ?AltSaveDirectoryName=Rag"
        ));
        assert!(!matches_cmdline(
            "Rag",
            "ArkAscendedServer.exe ?AltSaveDirectoryName=Rag2 -log"
        ));
        assert!(!matches_cmdline("Rag", "other ?AltSaveDirectoryName=Rag"));
    }

    #[test]
    fn gus_rewrite_moves_credentials_under_the_section() {
        let out = rewrite_gus(
            "[Other]\na=1\n[ServerSettings]\nServerPassword=old\nx=2\nRCONPort=1\n",
            "sp",
            "ap",
            "27020",
        );
        assert_eq!(
            out,
            "[Other]\na=1\n[ServerSettings]\nServerPassword=sp\nServerAdminPassword=ap\nRCONEnabled=True\nRCONPort=27020\nx=2\n"
        );
        assert_eq!(
            rewrite_gus("", "s", "a", "1"),
            "[ServerSettings]\nServerPassword=s\nServerAdminPassword=a\nRCONEnabled=True\nRCONPort=1\n"
        );
        assert_eq!(
            rewrite_gus("a=1", "s", "a", "1"),
            "a=1\n[ServerSettings]\nServerPassword=s\nServerAdminPassword=a\nRCONEnabled=True\nRCONPort=1\n"
        );
    }
}
