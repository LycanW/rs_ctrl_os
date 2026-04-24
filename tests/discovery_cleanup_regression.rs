use rs_ctrl_os::discovery::{Heartbeat, ServiceRegistry};
use std::thread;
use std::time::Duration;

#[test]
fn register_stores_receiver_time_not_sender_timestamp() {
    let registry = ServiceRegistry::new();
    let hb = Heartbeat {
        node_id: "node".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5555,
        timestamp: 0, // sender timestamp is ignored
        clock_time_ms: 0,
        is_master: false,
    };
    registry.register(&hb);
    // register stores receiver-local time, so node should be found immediately
    assert!(registry.get_address("node").is_some());
}

#[test]
fn cleanup_removes_expired_nodes() {
    let registry = ServiceRegistry::new();
    let hb = Heartbeat {
        node_id: "stale_node".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5555,
        timestamp: 0,
        clock_time_ms: 0,
        is_master: false,
    };

    registry.register(&hb);
    assert!(registry.get_address("stale_node").is_some());

    // Wait long enough for the entry to expire under a zero-second timeout.
    thread::sleep(Duration::from_millis(10));
    registry.cleanup(0);

    assert!(
        registry.get_address("stale_node").is_none(),
        "stale node should be removed after its entry ages past the cleanup timeout"
    );
}
