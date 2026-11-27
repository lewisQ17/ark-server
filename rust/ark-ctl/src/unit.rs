//! The systemd unit of a map (`create_systemd_service`, and the same text the dashboard writes).

pub fn service_name(map: &str) -> String {
    format!("ark-{}.service", map.to_lowercase())
}

/// `user` runs the server; `proton_log_dir` is that user's home directory.
pub fn render(map: &str, user: &str, script_dir: &str, proton_log_dir: &str) -> String {
    format!(
        "[Unit]\nDescription=ARK ASA Server - {map}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=forking\nUser={user}\nWorkingDirectory={script_dir}\nEnvironment=PROTON_LOG=1\nEnvironment=PROTON_LOG_DIR={proton_log_dir}\nExecStart={script_dir}/ark-manager.sh start {map}\nExecStop={script_dir}/ark-manager.sh stop {map}\nRestart=on-failure\nRestartSec=30\nTimeoutStartSec=300\nTimeoutStopSec=180\nMemoryAccounting=true\nCPUAccounting=true\n\n[Install]\nWantedBy=multi-user.target\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_text() {
        let u = render("Island", "arkadmin", "/opt/ark", "/home/arkadmin");
        assert!(u.starts_with("[Unit]\nDescription=ARK ASA Server - Island\n"));
        assert!(u.contains("ExecStart=/opt/ark/ark-manager.sh start Island\n"));
        assert!(u.contains("User=arkadmin\n") && u.ends_with("WantedBy=multi-user.target\n"));
        assert_eq!(service_name("TheIsland"), "ark-theisland.service");
    }
}
