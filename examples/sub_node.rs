use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use bincode;

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
/// # 订阅频率 Hz；>0 固定频率，0 表示不订阅（示例中要求 >0）
/// subscribe_hz = 1000
/// ```
#[derive(Clone, Deserialize)]
struct DynamicCfg {
    // 本示例不再从 dynamic 控制频率，仅保留占位以兼容 ConfigManager 泛型。
}

fn fmt_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
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
    bus.set_publish_hz(static_cfg.publish_hz);
    bus.set_subscribe_hz(static_cfg.subscribe_hz);

    loop {
        let _dyn_cfg = manager.get_dynamic_clone();

        // Drive discovery connections
        bus.tick()?;

        // Drain all pending messages from "local_sub" and print them.
        while let Some((topic, raw)) = bus.try_recv_raw("local_sub")? {
            // rs_ctrl_os publish_topic() uses bincode; can_bridge currently publishes a String(JSON) under sub_topic="data".
            if let Ok(s) = bincode::deserialize::<String>(&raw) {
                println!("Sub received sub_topic={topic} string={s}");
            } else {
                let as_utf8 = std::str::from_utf8(&raw).ok();
                println!(
                    "Sub received sub_topic={topic} len={} utf8={} hex={}",
                    raw.len(),
                    as_utf8.unwrap_or("<non-utf8>"),
                    fmt_hex(&raw)
                );
            }
        }

        // 简单 sleep，真正的订阅频率由 PubSubManager::set_subscribe_hz 控制
        thread::sleep(Duration::from_millis(1));
    }
}

