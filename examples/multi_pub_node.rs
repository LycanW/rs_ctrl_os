use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use rs_ctrl_os::{
    config::ConfigManager, init_logging, start_discovery, PubSubManager, Result, TimeSynchronizer,
};

/// Dynamic section of the TOML config used for the multi-pub node.
///
/// ```toml
/// [static_config]
/// my_id = "multi_pub"
/// host = "127.0.0.1"
/// port = 5560
/// is_master = true
///
/// [static_config.publishers]
/// control = "self"
/// status  = "self"
/// alerts  = "self"
///
/// [dynamic]
/// message_prefix = "multi"
/// interval_ms = 500
/// ```
#[derive(Clone, Deserialize)]
struct DynamicCfg {
    control_sub_topic: String,
    status_sub_topic: String,
    alerts_sub_topic: String,
    interval_ms: u64,
}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "multi_pub_config.toml".to_string());

    let manager: ConfigManager<DynamicCfg> = ConfigManager::new(Path::new(&config_path))?;
    let static_cfg = manager.static_cfg().clone();

    let time_sync = Arc::new(TimeSynchronizer::new());

    // This node also participates in discovery (acts as master here).
    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    // One process, single PUB socket (topic key "control"), multiple sub-topics
    let mut bus = PubSubManager::new(&static_cfg, registry)?;

    let mut counter: u64 = 0;

    loop {
        let dyn_cfg = manager.get_dynamic_clone();
        let ts_ms = time_sync.now_corrected_ms();
        counter = counter.wrapping_add(1);

        // control sub-topic
        let control_msg = format!(
            "[control] {} #{counter} at {} ms",
            dyn_cfg.control_sub_topic, ts_ms
        );
        bus.publish_topic("control", "demo_control", &control_msg)?;

        // status sub-topic
        let status_msg = format!(
            "[status] {} #{counter} at {} ms",
            dyn_cfg.status_sub_topic, ts_ms
        );
        bus.publish_topic("control", "demo_status", &status_msg)?;

        // alerts sub-topic
        let alerts_msg = format!(
            "[alerts] {} #{counter} at {} ms",
            dyn_cfg.alerts_sub_topic, ts_ms
        );
        bus.publish_topic("control", "demo_alerts", &alerts_msg)?;

        thread::sleep(Duration::from_millis(dyn_cfg.interval_ms));
    }
}
