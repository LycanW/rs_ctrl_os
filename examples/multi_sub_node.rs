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

/// Dynamic section of the TOML config used for the multi-sub node.
///
/// ```toml
/// [static_config]
/// my_id = "multi_sub"
/// host = "127.0.0.1"
/// port = 5561
/// is_master = false
///
/// [static_config.subscribers]
/// multi_sub = "multi_pub"
///
/// [dynamic]
/// # 订阅频率 Hz；>0 固定频率，0 表示不订阅（示例中要求 >0）
/// subscribe_hz = 500
/// ```
#[derive(Clone, Deserialize)]
struct DynamicCfg {
    // 本示例不再从 dynamic 控制频率，仅保留占位以兼容 ConfigManager 泛型。
}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "multi_sub_config.toml".to_string());

    let manager: ConfigManager<DynamicCfg> = ConfigManager::new(Path::new(&config_path))?;
    let static_cfg = manager.static_cfg().clone();

    let time_sync = Arc::new(TimeSynchronizer::new());

    // Participate in discovery to learn where `multi_pub` lives.
    let registry = start_discovery(
        &static_cfg.my_id,
        &static_cfg.host,
        static_cfg.port,
        static_cfg.is_master,
        Some(time_sync.clone()),
    )?;

    let mut bus = PubSubManager::new(&static_cfg, registry)?;
    bus.set_publish_hz(static_cfg.publish_hz);
    bus.set_subscribe_hz(static_cfg.subscribe_hz);

    loop {
        let _dyn_cfg = manager.get_dynamic_clone();

        // Drive pending_subs to actually connect to multi_pub once discovered.
        bus.tick()?;

        // One SUB socket (local name "multi_sub"); distinguish streams by sub-topic.
        if let Some((topic, bytes)) = bus.try_recv_raw("multi_sub")? {
            // 原始二进制
            // println!(
            //     "[multi_sub][raw]  sub_topic='{}', len={}, bytes={:02X?}",
            //     topic,
            //     bytes.len(),
            //     bytes
            // );

            // 尝试按 String 反序列化并打印解码后的内容
            if let Ok(text) = bincode::deserialize::<String>(&bytes) {
                println!(
                    "[multi_sub][dec]  sub_topic='{}', text=\"{}\"",
                    topic, text
                );
            } else {
                println!(
                    "[multi_sub][dec]  sub_topic='{}', <failed to deserialize as String>",
                    topic
                );
            }
        }

        // 简单 sleep，真正的订阅频率由 PubSubManager::set_subscribe_hz 控制
        thread::sleep(Duration::from_millis(1));
    }
}

