//! Pure parts of `ark-manager.sh` and `rcon.py`.
//!
//! * [`rcon`]   Source-RCON packets and the client protocol, over a [`rcon::Transport`] (fake in tests)
//! * [`conf`]   `read_conf_value` / `write_conf_value` on `KEY=VALUE` text
//! * [`launch`] launch plan of a map, running-process pattern, mod ids, password rewrite of GameUserSettings.ini
//! * [`unit`]   systemd unit text of a map
//! * [`plan`]   `add-map` option parsing and the files it would write
//! * [`system`] everything that would execute something, behind a trait with an in-memory fake

pub mod conf;
pub mod launch;
pub mod plan;
pub mod rcon;
pub mod system;
pub mod unit;

/// Uptime string of `get_map_uptime`: `3d 4h`, `5h 12m` or `7m`.
pub fn format_uptime(elapsed_s: u64) -> String {
    let d = elapsed_s / 86400;
    let h = (elapsed_s % 86400) / 3600;
    let m = (elapsed_s % 3600) / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Pad or cut `t` to `w` characters (`pad()` of the manager, for multi-byte text too).
pub fn pad(w: usize, t: &str) -> String {
    let cut: String = t.chars().take(w).collect();
    let n = w.saturating_sub(cut.chars().count());
    format!("{cut}{}", " ".repeat(n))
}

/// The internal map names the manager knows (`MAP_NAMES`).
pub const MAP_NAMES: [(&str, &str); 12] = [
    ("TheIsland", "TheIsland_WP"),
    ("ScorchedEarth", "ScorchedEarth_WP"),
    ("Aberration", "Aberration_WP"),
    ("Extinction", "Extinction_WP"),
    ("TheCenter", "TheCenter_WP"),
    ("Ragnarok", "Ragnarok_WP"),
    ("Valguero", "Valguero_WP"),
    ("CrystalIsles", "CrystalIsles_WP"),
    ("LostIsland", "LostIsland_WP"),
    ("Fjordur", "Fjordur_WP"),
    ("GenesisPart1", "Genesis_WP"),
    ("GenesisPart2", "Gen2_WP"),
];

pub fn internal_name(display: &str) -> Option<&'static str> {
    MAP_NAMES
        .iter()
        .find(|(d, _)| *d == display)
        .map(|(_, i)| *i)
}

/// `^[A-Za-z0-9_]+$`
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_matches_the_bash_branches() {
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3600), "1h 0m");
        assert_eq!(format_uptime(90061), "1d 1h");
    }

    #[test]
    fn pad_counts_characters() {
        assert_eq!(pad(5, "ab"), "ab   ");
        assert_eq!(pad(3, "caf\u{e9}s"), "caf");
        assert_eq!(pad(4, "\u{2714}"), "\u{2714}   ");
    }

    #[test]
    fn names() {
        assert_eq!(internal_name("GenesisPart1"), Some("Genesis_WP"));
        assert_eq!(internal_name("nope"), None);
        assert!(valid_name("My_Map1") && !valid_name("My Map") && !valid_name(""));
    }
}
