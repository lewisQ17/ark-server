use std::sync::Arc;

use ark_dashboard::config::{Config, Mode};
use ark_dashboard::exec::{Exec, LocalExec};
use ark_dashboard::fake::FakeVm;
use ark_dashboard::server;

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
        _ => Arc::new(LocalExec::new(&cfg.run_as_user, cfg.allow_host_power)),
    };
    if let Err(e) = server::run(cfg, exec).await {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}
