use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use rs_ctrl_os::{
    init_logging, load_config_typed, start_discovery, PubSubManager, Result, TimeSynchronizer,
};

/// 本示例不需要热重载，使用 load_config_typed 一次性加载。
#[derive(Clone, Deserialize)]
struct DynamicCfg {}

fn main() -> Result<()> {
    init_logging();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "multi_sub_config.toml".to_string());

    let (static_cfg, _dynamic) = load_config_typed::<DynamicCfg>(&config_path)?;

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

    // 只关心各远端节点下“一个” sub_topic：
    // - from_multi_pub: 只订阅 demo_status
    // - from_pub:       只订阅 demo
    bus.set_sub_topics("from_multi_pub", &["demo_status"])?;
    bus.set_sub_topics("from_pub", &["demo"])?;

    loop {
        // try_recv_raw 内部自动 tick()，无需手动调用
        // 多端口（多远端节点）+ 多子话题：
        // - "from_multi_pub" 订阅 multi_pub 节点
        // - "from_pub"       订阅 pub_node 节点
        for local_name in &["from_multi_pub", "from_pub"] {
            if let Some((sender, topic, bytes)) = bus.try_recv_raw(local_name)? {
                if let Ok(text) = bincode::deserialize::<String>(&bytes) {
                    println!(
                        "[multi_sub][dec]  from={} local='{}' sub_topic='{}' text=\"{}\"",
                        sender, local_name, topic, text
                    );
                } else {
                    println!(
                        "[multi_sub][dec]  from={} local='{}' sub_topic='{}' <failed to deserialize as String>",
                        sender, local_name, topic
                    );
                }
            }
        }

        // 简单 sleep，订阅限频由 static_config.subscribe_hz 控制（new 时已注入）
        thread::sleep(Duration::from_millis(1));
    }
}
