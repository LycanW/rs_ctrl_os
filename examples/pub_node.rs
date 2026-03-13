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

#[derive(Clone, Deserialize)]
struct DynamicCfg {
    message_prefix: String,
    interval_ms: u64,
}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pub_config.toml".to_string());

    let manager: ConfigManager<DynamicCfg> = ConfigManager::new(Path::new(&config_path))?;
    let static_cfg = manager.static_cfg().clone();

    // Time synchronizer for this node (acts as master in this simple setup)
    let time_sync = Arc::new(TimeSynchronizer::new());

    // Start discovery so other nodes can find us; we also get a live registry back.
    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    // Publisher only: one PUB socket for topic "control"
    // 频率由 static_config 的 publish_hz/subscribe_hz 提供，new() 时已注入
    let mut bus = PubSubManager::new(&static_cfg, registry)?;

    loop {
        let dyn_cfg = manager.get_dynamic_clone();
        let ts_ms = time_sync.now_corrected_ms();

        let payload = format!(
            "{} from {} at {} ms",
            dyn_cfg.message_prefix, static_cfg.my_id, ts_ms
        );

        // Single pub: topic key "control", sub-topic "demo"
        bus.publish_topic("control", "demo", &payload)?;

        thread::sleep(Duration::from_millis(dyn_cfg.interval_ms));
    }
}

