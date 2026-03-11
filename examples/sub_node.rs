use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use rs_ctrl_os::{
    config::ConfigManager,
    init_logging,
    start_discovery,
    PubSubManager,
    TimeSynchronizer,
    Result,
};

/// Dynamic section of the TOML config used for the subscriber node.
///
/// ```toml
/// [static_config]
/// my_id = "sub_node"
/// host = "127.0.0.1"
/// port = 5556
/// is_master = false
///
/// [static_config.subscribers]
/// local_sub = "pub_node"
///
/// [dynamic]
/// poll_interval_ms = 200
/// ```
#[derive(Clone, Deserialize)]
struct DynamicCfg {
    poll_interval_ms: u64,
}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sub_config.toml".to_string());

    let manager: ConfigManager<DynamicCfg> = ConfigManager::new(Path::new(&config_path))?;
    let static_cfg = manager.static_cfg().clone();

    let time_sync = Arc::new(TimeSynchronizer::new());

    // Subscriber participates in discovery so it can learn where `pub_node` is.
    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    // Subscriber only: one SUB socket named "local_sub"
    let mut bus = PubSubManager::new(&static_cfg, registry)?;

    loop {
        let dyn_cfg = manager.get_dynamic_clone();

        // Drive discovery connections
        bus.tick()?;

        if let Some(msg) = bus.try_recv_specific::<String>("local_sub", "demo")? {
            println!("Sub received: {msg}");
        }

        thread::sleep(Duration::from_millis(dyn_cfg.poll_interval_ms));
    }
}

