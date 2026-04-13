use crate::error::{Result, RsCtrlError};
use crate::time_sync::TimeSynchronizer;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

const MULTICAST_ADDR: &str = "224.0.0.100";
const DISCOVERY_PORT: u16 = 9999;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub timestamp: u64,
    pub clock_time_ms: u64,
    pub is_master: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceRegistry {
    nodes: Arc<RwLock<HashMap<String, (String, u16, u64)>>>,
}

impl ServiceRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&self, hb: &Heartbeat) {
        if let Ok(mut map) = self.nodes.write() {
            map.insert(hb.node_id.clone(), (hb.host.clone(), hb.port, hb.timestamp));
            debug!("📡 Discovered: {} @ {}:{}", hb.node_id, hb.host, hb.port);
        } else {
            warn!("ServiceRegistry register poisoned, skipping update");
        }
    }

    pub fn get_address(&self, node_id: &str) -> Option<(String, u16)> {
        self.nodes
            .read()
            .ok()
            .and_then(|map| map.get(node_id).map(|(h, p, _)| (h.clone(), *p)))
    }

    pub fn cleanup(&self, timeout_secs: u64) {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let timeout_ms = timeout_secs.saturating_mul(1000);
        if let Ok(mut map) = self.nodes.write() {
            map.retain(|_, (_, _, ts)| now_ms.saturating_sub(*ts) < timeout_ms);
        }
    }
}

pub fn start_discovery(
    my_id: &str,
    my_host: &str,
    my_port: u16,
    is_master: bool,
    time_sync: Option<Arc<TimeSynchronizer>>,
) -> Result<ServiceRegistry> {
    let registry = ServiceRegistry::new();
    let registry_clone = registry.clone();
    let my_id = my_id.to_string();
    let my_host = my_host.to_string();
    let time_sync_clone = time_sync.clone();

    let my_id_for_sender = my_id.clone();
    let my_id_for_receiver = my_id.clone();

    // Use socket2 to allow multiple processes to bind the same discovery port
    let addr: SocketAddr = format!("0.0.0.0:{}", DISCOVERY_PORT).parse().unwrap();
    let socket2 = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| RsCtrlError::Discovery(format!("Create discovery socket failed: {e}")))?;
    socket2
        .set_reuse_address(true)
        .map_err(|e| RsCtrlError::Discovery(format!("set_reuse_address failed: {e}")))?;
    socket2
        .bind(&addr.into())
        .map_err(|e| RsCtrlError::Discovery(format!("Bind discovery socket failed: {e}")))?;
    let socket: UdpSocket = socket2.into();
    let multicast_ip = Ipv4Addr::new(224, 0, 0, 100);
    socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_nonblocking(true)?;

    let send_socket = socket.try_clone()?;
    let broadcast_addr: SocketAddr = format!("{}:{}", MULTICAST_ADDR, DISCOVERY_PORT)
        .parse()
        .map_err(|e| RsCtrlError::Discovery(format!("Invalid discovery address: {e}")))?;
    
    thread::spawn(move || {
        let interval = Duration::from_secs(1);
        loop {
            let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
            let hb = Heartbeat {
                node_id: my_id_for_sender.clone(),
                host: my_host.clone(),
                port: my_port,
                timestamp: now_ms,
                clock_time_ms: now_ms,
                is_master,
            };
            if let Ok(json) = serde_json::to_string(&hb) {
                // Discovery heartbeat itself uses JSON for simplicity as it's low freq (1Hz)
                // If you strictly want no JSON anywhere, we can swap this to bincode too, 
                // but for 1Hz discovery, JSON overhead is negligible.
                let _ = send_socket.send_to(json.as_bytes(), broadcast_addr);
            }
            thread::sleep(interval);
        }
    });

    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    if let Ok(hb_str) = std::str::from_utf8(&buf[..len]) {
                        if let Ok(hb) = serde_json::from_str::<Heartbeat>(hb_str) {
                            if hb.node_id != my_id_for_receiver {
                                registry_clone.register(&hb);
                                if hb.is_master {
                                    if let Some(ref sync) = time_sync_clone {
                                        sync.update_from_master(&hb.node_id, hb.clock_time_ms);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    warn!("UDP recv error: {}", e);
                    thread::sleep(Duration::from_secs(1));
                }
            }
            registry_clone.cleanup(10);
        }
    });

    info!("📡 Discovery started (ID: {}, Master: {})", my_id, is_master);
    Ok(registry)
}