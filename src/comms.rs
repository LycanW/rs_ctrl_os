use crate::discovery::ServiceRegistry;
use crate::error::{Result, RsCtrlError};
use crate::config::StaticBase;
use bincode;
use std::collections::{HashMap, HashSet};
use zmq::{Context, Socket};
use tracing::{info, warn, debug};
use once_cell::sync::Lazy;

static ZMQ_CONTEXT: Lazy<Context> = Lazy::new(|| Context::new());

struct SubSocket {
    socket: Socket,
    _topics: HashSet<String>,
}

pub struct PubSubManager {
    pubs: HashMap<String, Socket>,
    shared_pub: Option<Socket>,
    shared_pub_topics: HashSet<String>,
    subs: HashMap<String, SubSocket>,
    registry: ServiceRegistry,
    pending_subs: HashMap<String, String>,
    my_id: String,
}

impl PubSubManager {
    pub fn new(static_cfg: &StaticBase, registry: ServiceRegistry) -> Result<Self> {
        let pubs = HashMap::new();
        let mut subs = HashMap::new();
        let mut pending_subs = HashMap::new();

        let self_topics: HashSet<String> = static_cfg.publishers.iter()
            .filter(|(_, t)| *t == "self")
            .map(|(k, _)| k.clone())
            .collect();
        let shared_pub = if self_topics.is_empty() {
            None
        } else {
            let socket = ZMQ_CONTEXT.socket(zmq::PUB)?;
            let endpoint = format!("tcp://{}:{}", static_cfg.host, static_cfg.port);
            socket.set_sndhwm(1000)?;
            socket.bind(&endpoint)?;
            info!("📢 [PUB] bound to {} (topics: {:?})", endpoint, self_topics);
            Some(socket)
        };

        for (local_name, target_node_id) in &static_cfg.subscribers {
            if let Some((host, port)) = registry.get_address(target_node_id) {
                Self::connect_sub(&mut subs, local_name, target_node_id, &host, port)?;
            } else {
                warn!("⏳ [SUB] '{}' waiting for '{}'", local_name, target_node_id);
                pending_subs.insert(local_name.clone(), target_node_id.clone());
            }
        }

        Ok(Self {
            pubs,
            shared_pub,
            shared_pub_topics: self_topics,
            subs,
            registry,
            pending_subs,
            my_id: static_cfg.my_id.clone(),
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

        info!("🔗 [SUB] '{}' connected to {} (Target: {})", local_name, endpoint, target_id);
        subs.insert(local_name.to_string(), SubSocket { socket, _topics: HashSet::new() });
        Ok(())
    }

    pub fn tick(&mut self) -> Result<()> {
        let mut to_connect = Vec::new();
        for (local_name, target_id) in &self.pending_subs.clone() {
            if let Some((host, port)) = self.registry.get_address(target_id) {
                to_connect.push((local_name.clone(), target_id.clone(), host, port));
            }
        }
        for (local_name, target_id, host, port) in to_connect {
            match Self::connect_sub(&mut self.subs, &local_name, &target_id, &host, port) {
                Ok(_) => { self.pending_subs.remove(&local_name); }
                Err(e) => warn!("Failed to connect {} to {}: {}", local_name, target_id, e),
            }
        }
        Ok(())
    }

    /// 发布特定子话题 (Bincode 序列化)
    pub fn publish_topic<T: serde::Serialize>(&self, topic_key: &str, sub_topic: &str, data: &T) -> Result<()> {
        let socket = if self.shared_pub_topics.contains(topic_key) {
            self.shared_pub.as_ref()
                .ok_or_else(|| RsCtrlError::Comms(format!("Pub key '{}' not initialized", topic_key)))?
        } else {
            self.pubs.get(topic_key)
                .ok_or_else(|| RsCtrlError::Comms(format!("Pub key '{}' not found", topic_key)))?
        };

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
    pub fn try_recv_raw(&self, local_name: &str) -> Result<Option<(String, Vec<u8>)>> {
        // 如果订阅还没建立（例如仍在 pending_subs 中），返回 Ok(None) 表示当前没有可读数据
        let Some(sub_entry) = self.subs.get(local_name) else {
            return Ok(None);
        };

        match sub_entry.socket.recv_multipart(0) {
            Ok(frames) => {
                if frames.len() < 3 { return Ok(None); }
                let sub_topic = String::from_utf8_lossy(&frames[1]).to_string();
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
    pub fn try_recv_specific<T: for<'de> serde::Deserialize<'de>>(&self, local_name: &str, target_sub: &str) -> Result<Option<T>> {
        if let Some((topic, bytes)) = self.try_recv_raw(local_name)? {
            if topic == target_sub {
                let data = bincode::deserialize(&bytes)?;
                return Ok(Some(data));
            }
        }
        Ok(None)
    }
}