//! Everything the manager does that touches the machine (systemctl, ufw, pgrep, files, proton).
//! Nothing here executes anything: [`System`] is the seam, [`FakeSystem`] is the in-memory double
//! used by tests and by the CLI's `--dry-run` output.

use std::collections::BTreeMap;

use crate::launch::{LaunchPlan, matches_cmdline};

pub trait System {
    fn read_file(&self, path: &str) -> Option<String>;
    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String>;
    fn list_dir(&self, path: &str) -> Vec<String>;
    /// Command lines of running processes (what `pgrep -f` searches).
    fn process_cmdlines(&self) -> Vec<String>;
    /// Run a privileged command (`systemctl ...`, `ufw ...`). Returns the exit code.
    fn run(&mut self, argv: &[String]) -> i32;
    /// Start the game server process.
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<(), String>;
}

/// Is a map with this save dir running?
pub fn is_map_running(sys: &dyn System, save_dir: &str) -> bool {
    sys.process_cmdlines()
        .iter()
        .any(|c| matches_cmdline(save_dir, c))
}

#[derive(Default)]
pub struct FakeSystem {
    pub files: BTreeMap<String, String>,
    pub processes: Vec<String>,
    /// Every privileged command, in order.
    pub commands: Vec<Vec<String>>,
    pub spawned: Vec<LaunchPlan>,
    pub fail_commands: bool,
}

impl System for FakeSystem {
    fn read_file(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        self.files.insert(path.to_string(), content.to_string());
        Ok(())
    }
    fn list_dir(&self, path: &str) -> Vec<String> {
        let prefix = format!("{}/", path.trim_end_matches('/'));
        let mut names: Vec<String> = self
            .files
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter_map(|r| r.split('/').next())
            .map(String::from)
            .collect();
        names.sort();
        names.dedup();
        names
    }
    fn process_cmdlines(&self) -> Vec<String> {
        self.processes.clone()
    }
    fn run(&mut self, argv: &[String]) -> i32 {
        self.commands.push(argv.to_vec());
        i32::from(self.fail_commands)
    }
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<(), String> {
        self.processes.push(plan.argv.join(" "));
        self.spawned.push(plan.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::{Paths, plan};

    #[test]
    fn spawning_makes_the_map_look_running_and_only_that_map() {
        let mut sys = FakeSystem::default();
        let paths = Paths::under("/opt/ark", "GE-Proton10-4");
        let lp = plan("Rag", "MapName=Ragnarok_WP\n", "", None, &paths);
        assert!(!is_map_running(&sys, "Rag"));
        sys.spawn(&lp).unwrap();
        assert!(is_map_running(&sys, "Rag"));
        assert!(!is_map_running(&sys, "Ra"));
        assert_eq!(sys.spawned.len(), 1);
    }

    #[test]
    fn commands_are_recorded_not_run() {
        let mut sys = FakeSystem::default();
        assert_eq!(sys.run(&["systemctl".into(), "daemon-reload".into()]), 0);
        assert_eq!(
            sys.commands,
            vec![vec!["systemctl".to_string(), "daemon-reload".to_string()]]
        );
    }

    #[test]
    fn dir_listing_is_by_prefix() {
        let mut sys = FakeSystem::default();
        sys.write_file("/m/A/map.conf", "x").unwrap();
        sys.write_file("/m/B/map.conf", "x").unwrap();
        sys.write_file("/m/B/Game.ini", "x").unwrap();
        assert_eq!(sys.list_dir("/m"), ["A", "B"]);
    }
}
