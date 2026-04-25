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

// --- RPC binary envelope ---
// Wraps user payload inside a 10-byte header for request/response correlation.
// Wire format inside payload frame: [magic: 1B][type: 1B][request_id: u64 LE]

const RPC_MAGIC: u8 = 0x52; // 'R'
const RPC_MSG_REQUEST: u8 = 0x01;
const RPC_MSG_RESPONSE: u8 = 0x02;
const RPC_HEADER_LEN: usize = 10;

fn build_rpc_header(msg_type: u8, request_id: u64) -> [u8; RPC_HEADER_LEN] {
    let mut hdr = [0u8; RPC_HEADER_LEN];
    hdr[0] = RPC_MAGIC;
    hdr[1] = msg_type;
    hdr[2..RPC_HEADER_LEN].copy_from_slice(&request_id.to_le_bytes());
    hdr
}

fn parse_rpc_header(data: &[u8]) -> Option<(u8, u64)> {
    if data.len() < RPC_HEADER_LEN || data[0] != RPC_MAGIC {
        return None;
    }
    let msg_type = data[1];
    if msg_type != RPC_MSG_REQUEST && msg_type != RPC_MSG_RESPONSE {
        return None;
    }
    let rid = u64::from_le_bytes(data[2..RPC_HEADER_LEN].try_into().unwrap());
    Some((msg_type, rid))
}

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
        // Validate: only "self" publishers are supported.
        for (topic_key, target) in &static_cfg.publishers {
            if target != "self" {
                return Err(RsCtrlError::Config(format!(
                    "publisher '{}' has target '{}' — only \"self\" is supported",
                    topic_key, target
                )));
            }
        }

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
        socket.set_rcvtimeo(0)?;
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

    fn trim_stale_rate_entries(map: &mut HashMap<String, Instant>, now: Instant) {
        if map.len() > 64 {
            map.retain(|_, v| now.duration_since(*v) < Duration::from_secs(60));
        }
    }

    /// Core send: builds 3-frame multipart and sends on the PUB socket.
    /// When `bypass_rate` is true, the publish_hz rate limiter is skipped.
    fn send_raw_inner(
        &mut self,
        topic_key: &str,
        sub_topic: &str,
        payload: &[u8],
        bypass_rate: bool,
    ) -> Result<()> {
        if self.publish_hz < 0 {
            return Ok(());
        }
        if self.publish_hz > 0 && !bypass_rate {
            let now = Instant::now();
            let min_interval = Duration::from_secs_f64(1.0 / self.publish_hz as f64);
            if let Some(last) = self.last_publish.get(topic_key) {
                if now.duration_since(*last) < min_interval {
                    return Ok(());
                }
            }
            self.last_publish.insert(topic_key.to_string(), now);
            Self::trim_stale_rate_entries(&mut self.last_publish, now);
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

    /// 发布原始字节（不经过 serde/bincode，直接透传）。
    /// 适用于图像、点云等已编码的二进制数据（JPEG、压缩点云等）。
    /// 频率控制与 `publish_topic` 共享。
    pub fn publish_raw(&mut self, topic_key: &str, sub_topic: &str, payload: &[u8]) -> Result<()> {
        self.send_raw_inner(topic_key, sub_topic, payload, false)
    }

    /// 发布特定子话题 (Bincode 序列化)
    pub fn publish_topic<T: serde::Serialize>(
        &mut self,
        topic_key: &str,
        sub_topic: &str,
        data: &T,
    ) -> Result<()> {
        let payload = bincode::serialize(data)?;
        self.send_raw_inner(topic_key, sub_topic, &payload, false)
    }

    /// Core receive: performs tick + rate limiting + ZMQ recv + topic filter.
    /// Returns all 3 frames: (sender_id, sub_topic, payload).
    fn try_recv_inner(&mut self, local_name: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let _ = self.tick();

        if self.subscribe_hz < 0 {
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
            Self::trim_stale_rate_entries(&mut self.last_sub_poll, now);
        }

        let Some(sub_entry) = self.subs.get(local_name) else {
            return Ok(None);
        };

        match sub_entry.socket.recv_multipart(0) {
            Ok(frames) => {
                if frames.len() < 3 {
                    return Ok(None);
                }
                let sender_id = String::from_utf8_lossy(&frames[0]).to_string();
                let sub_topic = String::from_utf8_lossy(&frames[1]).to_string();

                if let Some(entry) = self.subs.get(local_name) {
                    if !entry.topics.is_empty() && !entry.topics.contains(&sub_topic) {
                        return Ok(None);
                    }
                }

                let payload = frames[2].to_vec();
                Ok(Some((sender_id, sub_topic, payload)))
            }
            Err(e) if e == zmq::Error::EAGAIN => Ok(None),
            Err(e) => {
                debug!("Recv error on {}: {}", local_name, e);
                Ok(None)
            }
        }
    }

    /// 接收原始字节 (由用户反序列化)
    /// 返回 (sender_id, sub_topic, payload)，其中 sender_id 是发布者的 node_id。
    /// 内部自动调用 tick()，无需在主循环手动调用。
    pub fn try_recv_raw(
        &mut self,
        local_name: &str,
    ) -> Result<Option<(String, String, Vec<u8>)>> {
        self.try_recv_inner(local_name)
    }

    /// 辅助：直接接收并反序列化为特定类型 (如果知道具体话题)
    pub fn try_recv_specific<T: for<'de> serde::Deserialize<'de>>(
        &mut self,
        local_name: &str,
        target_sub: &str,
    ) -> Result<Option<T>> {
        if let Some((_sender, topic, bytes)) = self.try_recv_raw(local_name)? {
            if topic == target_sub {
                let data = bincode::deserialize(&bytes)?;
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    // --- RPC methods: request-response on top of PUB/SUB ---
    //
    // RPC messages use a 10-byte binary envelope prepended to the payload:
    //   [magic: 'R'][type: 0x01=req/0x02=res][request_id: u64 LE]
    //
    // These methods **bypass** the publish_hz rate limiter so that
    // imperative commands (e.g. emergency stop) are never silently dropped.

    /// 发布 RPC 请求。自动绕过发布频率限制。
    pub fn publish_request(
        &mut self,
        topic_key: &str,
        sub_topic: &str,
        request_id: u64,
        payload: &[u8],
    ) -> Result<()> {
        let header = build_rpc_header(RPC_MSG_REQUEST, request_id);
        let mut buf = Vec::with_capacity(RPC_HEADER_LEN + payload.len());
        buf.extend_from_slice(&header);
        buf.extend_from_slice(payload);
        self.send_raw_inner(topic_key, sub_topic, &buf, true)
    }

    /// 发布 RPC 响应。自动绕过发布频率限制。
    pub fn publish_response(
        &mut self,
        topic_key: &str,
        sub_topic: &str,
        request_id: u64,
        payload: &[u8],
    ) -> Result<()> {
        let header = build_rpc_header(RPC_MSG_RESPONSE, request_id);
        let mut buf = Vec::with_capacity(RPC_HEADER_LEN + payload.len());
        buf.extend_from_slice(&header);
        buf.extend_from_slice(payload);
        self.send_raw_inner(topic_key, sub_topic, &buf, true)
    }

    /// 接收 RPC 请求。
    /// 返回 `(sender_id, request_id, sub_topic, payload)`。
    /// 非 RPC 请求的消息会被静默丢弃，请通过 sub_topic 分离普通流量和 RPC 流量。
    pub fn try_recv_request(
        &mut self,
        local_name: &str,
    ) -> Result<Option<(String, u64, String, Vec<u8>)>> {
        if let Some((sender, sub_topic, raw)) = self.try_recv_inner(local_name)? {
            if let Some((msg_type, rid)) = parse_rpc_header(&raw) {
                if msg_type == RPC_MSG_REQUEST {
                    let payload = raw[RPC_HEADER_LEN..].to_vec();
                    return Ok(Some((sender, rid, sub_topic, payload)));
                }
            }
        }
        Ok(None)
    }

    /// 接收 RPC 响应。
    /// 返回 `(sender_id, request_id, sub_topic, payload)`。
    /// 非 RPC 响应的消息会被静默丢弃。
    pub fn try_recv_response(
        &mut self,
        local_name: &str,
    ) -> Result<Option<(String, u64, String, Vec<u8>)>> {
        if let Some((sender, sub_topic, raw)) = self.try_recv_inner(local_name)? {
            if let Some((msg_type, rid)) = parse_rpc_header(&raw) {
                if msg_type == RPC_MSG_RESPONSE {
                    let payload = raw[RPC_HEADER_LEN..].to_vec();
                    return Ok(Some((sender, rid, sub_topic, payload)));
                }
            }
        }
        Ok(None)
    }
}
