use crate::config::StaticBase;
use crate::discovery::ServiceRegistry;
use crate::error::{Result, RsCtrlError};
use bincode;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use zmq::{Context, Socket};

static ZMQ_CONTEXT: Lazy<Context> = Lazy::new(|| Context::new());

/// 解析 "host:port" 或 "[::1]:port"，不进行 DNS 解析。
fn parse_host_port(s: &str) -> Option<(String, u16)> {
    let idx = s.rfind(':')?;
    if idx == 0 {
        return None;
    }
    let (host, port_part) = s.split_at(idx);
    let port = port_part[1..].parse::<u16>().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

struct SubSocket {
    socket: Socket,
    /// 若为空集，则不过滤 sub_topic；非空时，仅保留在集合内的 sub_topic。
    topics: HashSet<String>,
}

/// Pub/Sub 管理器
///
/// - 发布频率控制：通过 `publish_hz` 限制 `publish_topic` 的最大发送速率。
/// - 订阅频率控制：通过 `subscribe_hz` 限制 `try_recv_*` 的轮询频率。
///   频率配置建议从各节点的 `[dynamic]` 中传入（如 `publish_hz` / `subscribe_hz`）。
pub struct PubSubManager {
    shared_pub: Option<Socket>,
    subs: HashMap<String, SubSocket>,
    registry: ServiceRegistry,
    /// node_id -> (host, port)，discovery 失败时的 fallback
    static_nodes: HashMap<String, (String, u16)>,
    pending_subs: HashMap<String, String>,
    my_id: String,

    // 频率控制（节点级别）
    publish_hz: i64,
    subscribe_hz: i64,
    last_publish: HashMap<String, Instant>, // 按 topic_key 跟踪
    last_sub_poll: HashMap<String, Instant>, // 按 local_name 跟踪
}

impl PubSubManager {
    pub fn new(static_cfg: &StaticBase, registry: ServiceRegistry) -> Result<Self> {
        let mut subs = HashMap::new();
        let mut pending_subs = HashMap::new();

        let shared_pub = if static_cfg.publishers.is_empty() {
            None
        } else {
            let socket = ZMQ_CONTEXT.socket(zmq::PUB)?;
            let endpoint = format!("tcp://{}:{}", static_cfg.host, static_cfg.port);
            socket.set_sndhwm(1000)?;
            socket.bind(&endpoint)?;
            info!("📢 [PUB] bound to {} (topics: {:?})", endpoint, static_cfg.publishers);
            Some(socket)
        };

        let static_nodes: HashMap<String, (String, u16)> = static_cfg
            .static_nodes
            .iter()
            .filter_map(|(k, v)| parse_host_port(v).map(|hp| (k.clone(), hp)))
            .collect();

        for (local_name, target_node_id) in &static_cfg.subscribers {
            let addr = registry.get_address(target_node_id).or_else(|| {
                static_nodes
                    .get(target_node_id)
                    .map(|(h, p)| (h.clone(), *p))
            });
            if let Some((host, port)) = addr {
                Self::connect_sub(&mut subs, local_name, target_node_id, &host, port)?;
            } else {
                warn!("⏳ [SUB] '{}' waiting for '{}'", local_name, target_node_id);
                pending_subs.insert(local_name.clone(), target_node_id.clone());
            }
        }

        Ok(Self {
            shared_pub,
            subs,
            registry,
            static_nodes,
            pending_subs,
            my_id: static_cfg.my_id.clone(),
            publish_hz: static_cfg.publish_hz,
            subscribe_hz: static_cfg.subscribe_hz,
            last_publish: HashMap::new(),
            last_sub_poll: HashMap::new(),
        })
    }

    fn connect_sub(
        subs: &mut HashMap<String, SubSocket>,
        local_name: &str,
        target_id: &str,
        host: &str,
        port: u16,
    ) -> Result<()> {
        let socket = ZMQ_CONTEXT.socket(zmq::SUB)?;
        let endpoint = format!("tcp://{}:{}", host, port);
        socket.connect(&endpoint)?;
        socket.set_subscribe(b"")?; // Subscribe all, filter by app logic
        socket.set_rcvtimeo(100)?;
        socket.set_reconnect_ivl(100)?;
        socket.set_reconnect_ivl_max(5000)?;
        socket.set_rcvhwm(1000)?;

        info!(
            "🔗 [SUB] '{}' connected to {} (Target: {})",
            local_name, endpoint, target_id
        );
        subs.insert(
            local_name.to_string(),
            SubSocket {
                socket,
                topics: HashSet::new(),
            },
        );
        Ok(())
    }

    /// 设置节点级发布频率（Hz）。
    ///
    /// - `hz > 0`：对所有 `publish_topic` 生效，按最小时间间隔限频；
    /// - `hz = 0`：动态频率（有多少发多快，仍受 ZMQ HWM 影响）。
    /// - `hz < 0`：不发布（publish_topic 直接返回 Ok(())）。
    pub fn set_publish_hz(&mut self, hz: i64) {
        self.publish_hz = hz;
    }

    /// 设置节点级订阅/处理频率（Hz）。
    ///
    /// - `hz > 0`：对所有 `try_recv_*` 生效，按最小时间间隔限频；
    /// - `hz = 0`：动态频率（按调用频率尝试收取）。
    /// - `hz < 0`：不订阅/不消费（直接返回 Ok(None)）。
    pub fn set_subscribe_hz(&mut self, hz: i64) {
        self.subscribe_hz = hz;
    }

    /// 为指定本地订阅名配置需要保留的 sub_topic 列表。
    ///
    /// - `topics` 为空：不过滤任何 sub_topic（保留所有）。
    /// - 非空：仅当收到的 sub_topic 在此列表中时才返回；其他 sub_topic 会被静默丢弃。
    pub fn set_sub_topics<S: AsRef<str>>(&mut self, local_name: &str, topics: &[S]) -> Result<()> {
        let entry = self
            .subs
            .get_mut(local_name)
            .ok_or_else(|| RsCtrlError::Comms(format!("SUB '{}' not found", local_name)))?;
        entry.topics.clear();
        for t in topics {
            entry.topics.insert(t.as_ref().to_string());
        }
        Ok(())
    }

    pub fn tick(&mut self) -> Result<()> {
        let mut to_connect = Vec::new();
        for (local_name, target_id) in &self.pending_subs.clone() {
            let addr = self.registry.get_address(target_id).or_else(|| {
                self.static_nodes
                    .get(target_id)
                    .map(|(h, p)| (h.clone(), *p))
            });
            if let Some((host, port)) = addr {
                to_connect.push((local_name.clone(), target_id.clone(), host, port));
            }
        }
        for (local_name, target_id, host, port) in to_connect {
            match Self::connect_sub(&mut self.subs, &local_name, &target_id, &host, port) {
                Ok(_) => {
                    self.pending_subs.remove(&local_name);
                }
                Err(e) => warn!("Failed to connect {} to {}: {}", local_name, target_id, e),
            }
        }
        Ok(())
    }

    /// 发布原始字节（不经过 serde/bincode，直接透传）。
    /// 适用于图像、点云等已编码的二进制数据（JPEG、压缩点云等）。
    /// 频率控制与 `publish_topic` 共享。
    pub fn publish_raw(&mut self, topic_key: &str, sub_topic: &str, payload: &[u8]) -> Result<()> {
        if self.publish_hz < 0 {
            return Ok(());
        }
        if self.publish_hz > 0 {
            let now = Instant::now();
            let min_interval = Duration::from_secs_f64(1.0 / self.publish_hz as f64);
            if let Some(last) = self.last_publish.get(topic_key) {
                if now.duration_since(*last) < min_interval {
                    return Ok(());
                }
            }
            self.last_publish.insert(topic_key.to_string(), now);
        }

        let socket = self.shared_pub.as_ref().ok_or_else(|| {
            RsCtrlError::Comms(format!("Pub key '{}' not initialized", topic_key))
        })?;

        let id_bytes = self.my_id.as_bytes();
        let topic_bytes = sub_topic.as_bytes();

        match socket.send_multipart(&[id_bytes, topic_bytes, payload], zmq::DONTWAIT) {
            Ok(_) => Ok(()),
            Err(e) if e == zmq::Error::EAGAIN => Ok(()),
            Err(e) => Err(RsCtrlError::Zmq(e)),
        }
    }

    /// 发布特定子话题 (Bincode 序列化)
    pub fn publish_topic<T: serde::Serialize>(
        &mut self,
        topic_key: &str,
        sub_topic: &str,
        data: &T,
    ) -> Result<()> {
        // 1) 频率控制：如设置了 publish_hz，则按最小间隔丢弃过快的发送请求
        if self.publish_hz < 0 {
            // 全局禁止发布
            return Ok(());
        }
        if self.publish_hz > 0 {
            let now = Instant::now();
            let min_interval = Duration::from_secs_f64(1.0 / self.publish_hz as f64);
            if let Some(last) = self.last_publish.get(topic_key) {
                if now.duration_since(*last) < min_interval {
                    // 超过频率上限，直接丢弃本次发送请求
                    return Ok(());
                }
            }
            self.last_publish.insert(topic_key.to_string(), now);
        }

        let socket = self.shared_pub.as_ref().ok_or_else(|| {
            RsCtrlError::Comms(format!("Pub key '{}' not initialized", topic_key))
        })?;

        let payload = bincode::serialize(data)?;

        let id_bytes = self.my_id.as_bytes();
        let topic_bytes = sub_topic.as_bytes();

        match socket.send_multipart(&[id_bytes, topic_bytes, &payload], zmq::DONTWAIT) {
            Ok(_) => Ok(()),
            Err(e) if e == zmq::Error::EAGAIN => Ok(()),
            Err(e) => Err(RsCtrlError::Zmq(e)),
        }
    }

    /// 接收原始字节 (由用户反序列化)
    /// 内部自动调用 tick()，无需在主循环手动调用。
    pub fn try_recv_raw(&mut self, local_name: &str) -> Result<Option<(String, Vec<u8>)>> {
        // 0) 自动驱动 pending 订阅连接（目标发现后自动建立）
        let _ = self.tick();

        // 1) 频率控制：如设置了 subscribe_hz，则按最小间隔限制轮询频率
        if self.subscribe_hz < 0 {
            // 全局禁止订阅/消费
            return Ok(None);
        }
        if self.subscribe_hz > 0 {
            let now = Instant::now();
            let min_interval = Duration::from_secs_f64(1.0 / self.subscribe_hz as f64);
            if let Some(last) = self.last_sub_poll.get(local_name) {
                if now.duration_since(*last) < min_interval {
                    return Ok(None);
                }
            }
            self.last_sub_poll.insert(local_name.to_string(), now);
        }

        // 如果订阅还没建立（例如仍在 pending_subs 中），返回 Ok(None) 表示当前没有可读数据
        let Some(sub_entry) = self.subs.get(local_name) else {
            return Ok(None);
        };

        match sub_entry.socket.recv_multipart(0) {
            Ok(frames) => {
                if frames.len() < 3 {
                    return Ok(None);
                }
                let sub_topic = String::from_utf8_lossy(&frames[1]).to_string();

                // 若为该本地订阅名配置了 sub_topic 过滤，只保留白名单内的 sub_topic。
                if let Some(entry) = self.subs.get(local_name) {
                    if !entry.topics.is_empty() && !entry.topics.contains(&sub_topic) {
                        return Ok(None);
                    }
                }

                let payload = frames[2].to_vec();
                Ok(Some((sub_topic, payload)))
            }
            Err(e) if e == zmq::Error::EAGAIN => Ok(None),
            Err(e) => {
                debug!("Recv error on {}: {}", local_name, e);
                Ok(None)
            }
        }
    }

    /// 辅助：直接接收并反序列化为特定类型 (如果知道具体话题)
    pub fn try_recv_specific<T: for<'de> serde::Deserialize<'de>>(
        &mut self,
        local_name: &str,
        target_sub: &str,
    ) -> Result<Option<T>> {
        if let Some((topic, bytes)) = self.try_recv_raw(local_name)? {
            if topic == target_sub {
                let data = bincode::deserialize(&bytes)?;
                return Ok(Some(data));
            }
        }
        Ok(None)
    }
}
