use std::sync::Arc;

use ark_dashboard::config::{Config, Mode};
use ark_dashboard::exec::{Exec, LocalExec};
use ark_dashboard::fake::FakeVm;
use ark_dashboard::proxmox::Proxmox;
use ark_dashboard::server;
use ark_dashboard::unifi::Unifi;

#[tokio::main]
async fn main() {
    let cfg = match Config::from_env(&|k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(2);
        }
    };
    let exec: Arc<dyn Exec> = match cfg.mode {
        Mode::Fake => Arc::new(FakeVm::new(cfg.allow_host_power)),
        Mode::Local => Arc::new(LocalExec::new(&cfg.run_as_user, cfg.allow_host_power)),
        Mode::Proxmox => match Proxmox::new(&cfg.pve) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(2);
            }
        },
    };
    let udm = if cfg.unifi.enabled {
        match Unifi::new(&cfg.unifi) {
            Ok(u) => Some(u),
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(2);
            }
        }
    } else {
        None
    };
    if let Err(e) = server::run(cfg, exec, udm).await {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}
