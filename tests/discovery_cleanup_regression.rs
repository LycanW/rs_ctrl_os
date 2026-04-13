use rs_ctrl_os::discovery::{Heartbeat, ServiceRegistry};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cleanup_removes_expired_nodes() {
    let registry = ServiceRegistry::new();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_millis() as u64;

    // Timestamp is 20 seconds old, so cleanup(timeout=10s) should remove it.
    let stale = Heartbeat {
        node_id: "stale_node".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5555,
        timestamp: now_ms.saturating_sub(20_000),
        clock_time_ms: now_ms,
        is_master: false,
    };

    registry.register(&stale);
    registry.cleanup(10);

    assert!(
        registry.get_address("stale_node").is_none(),
        "stale node should be removed after cleanup timeout"
    );
}
