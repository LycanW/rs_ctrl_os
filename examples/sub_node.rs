use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bincode;
use serde::Deserialize;

use rs_ctrl_os::{
    init_logging, load_config_typed, start_discovery, PubSubManager, Result, TimeSynchronizer,
};

/// 本示例不需要热重载，使用 load_config_typed 一次性加载。
#[derive(Clone, Deserialize)]
struct DynamicCfg {}

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

    let (static_cfg, _dynamic) = load_config_typed::<DynamicCfg>(&config_path)?;

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
        // try_recv_raw 内部自动 tick()，无需手动调用
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

        // 简单 sleep，订阅限频由 static_config.subscribe_hz 控制（new 时已注入）
        thread::sleep(Duration::from_millis(1));
    }
}
