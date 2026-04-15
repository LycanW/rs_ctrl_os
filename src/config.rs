use crate::error::{Result, RsCtrlError};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// 从 TOML 文件加载 rs_ctrl_os 必须的配置。
/// 返回 `(static_config, dynamic)`：框架静态配置 + 原始 dynamic 节（供用户按需反序列化）。
/// 
/// TOML 必须包含 `[static_config]`；`[dynamic]` 可选，缺失时返回空表。
/// 
/// # 示例
/// ```ignore
/// // 仅需 static_config
/// let (static_cfg, _) = rs_ctrl_os::load_config_rcos("config.toml")?;
///
/// // 需要 typed dynamic
/// let (static_cfg, dyn_val) = rs_ctrl_os::load_config_rcos("config.toml")?;
/// let dynamic: MyDynamicConfig = toml::from_str(&toml::to_string(&dyn_val)?)?;
/// ```
pub fn load_config_rcos(path: impl AsRef<Path>) -> Result<(StaticBase, toml::Value)> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .map_err(|e| RsCtrlError::Config(format!("Read failed: {}", e)))?;

    let val: toml::Value = toml::from_str(&content)
        .map_err(|e| RsCtrlError::Config(format!("Parse failed: {}", e)))?;

    let static_val = val
        .get("static_config")
        .ok_or_else(|| RsCtrlError::Config("Missing [static_config]".into()))?;
    let static_cfg: StaticBase = toml::from_str(
        &toml::to_string(static_val)
            .map_err(|e| RsCtrlError::Config(e.to_string()))?,
    )
    .map_err(|e| RsCtrlError::Config(format!("static_config parse failed: {}", e)))?;

    let dynamic = val
        .get("dynamic")
        .cloned()
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

    Ok((static_cfg, dynamic))
}

/// 从 TOML 文件加载配置，返回强类型 `(StaticBase, D)`。
/// 封装 `load_config_rcos` + dynamic 反序列化，业务无需手写 toml 转换。
///
/// # 示例
/// ```ignore
/// let (static_cfg, dynamic) = rs_ctrl_os::load_config_typed::<MyDynamicConfig>("config.toml")?;
/// ```
pub fn load_config_typed<D>(path: impl AsRef<Path>) -> Result<(StaticBase, D)>
where
    D: for<'de> Deserialize<'de>,
{
    let (static_cfg, dyn_val) = load_config_rcos(path)?;
    let dynamic: D = toml::from_str(
        &toml::to_string(&dyn_val).map_err(|e| RsCtrlError::Config(e.to_string()))?,
    )
    .map_err(|e| RsCtrlError::Config(format!("dynamic parse failed: {}", e)))?;
    Ok((static_cfg, dynamic))
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct StaticBase {
    pub my_id: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub is_master: bool,
    #[serde(default)]
    pub subscribers: HashMap<String, String>,
    /// 静态节点 fallback：node_id -> "host:port"。当 discovery 未找到目标时使用。
    /// 支持无多播环境（Docker、云等）。
    #[serde(default)]
    pub static_nodes: HashMap<String, String>,
    #[serde(default)]
    pub publishers: HashMap<String, String>, 
    /// 发布频率（Hz），节点级上限：
    /// - >0: 所有 topic 的 publish_topic 默认按该频率限速
    /// - =0: 动态频率（有多少发多快）
    /// - -1: 不发布（publish_topic 直接丢弃）
    pub publish_hz: i64,
    /// 订阅/处理频率（Hz），节点级上限：
    /// - >0: 所有 try_recv_* 默认按该频率限速
    /// - =0: 动态频率（按调用频率尝试收取）
    /// - -1: 不订阅/不消费（try_recv_* 恒返回 None）
    pub subscribe_hz: i64,
    /// 是否启用 dynamic 配置热更新（文件监听）。默认 true；置 false 时不创建 watcher，无额外开销。
    #[serde(default = "default_true")]
    pub dynamic_load_enable: bool,
}

#[derive(Deserialize)]
struct FullConfig<D> {
    static_config: StaticBase,
    dynamic: D,
}

pub struct ConfigManager<D>
where
    D: Clone + for<'de> Deserialize<'de> + Send + Sync + 'static,
{
    static_cfg: StaticBase,
    dynamic_data: Arc<RwLock<D>>,
    _watcher: Option<RecommendedWatcher>,
    config_path: PathBuf,
}

impl<D> ConfigManager<D>
where
    D: Clone + for<'de> Deserialize<'de> + Send + Sync + 'static,
{
    pub fn new(config_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(config_path)
            .map_err(|e| RsCtrlError::Config(format!("Read failed: {}", e)))?;

        let full_cfg: FullConfig<D> = toml::from_str(&content)
            .map_err(|e| RsCtrlError::Config(format!("Parse failed: {}", e)))?;

        let static_cfg = full_cfg.static_config.clone();
        let dynamic_data = Arc::new(RwLock::new(full_cfg.dynamic));

        let path_buf = config_path.to_path_buf();
        let watcher = if static_cfg.dynamic_load_enable {
            let dyn_clone = dynamic_data.clone();
            let path_clone = path_buf.clone();
            let target_name = path_clone.file_name().map(|n| n.to_owned());
            let mut w = RecommendedWatcher::new(
                move |res: notify::Result<notify::Event>| {
                    if let Ok(event) = res {
                        if let Some(ref name) = target_name {
                            if event.paths.iter().any(|p| p.file_name() == Some(name.as_ref())) {
                                Self::reload_dynamic(&path_clone, &dyn_clone);
                            }
                        }
                    }
                },
                Config::default(),
            )
            .map_err(|e| RsCtrlError::Config(format!("Watch init failed: {}", e)))?;
            w.watch(config_path, RecursiveMode::NonRecursive)
                .map_err(|e| RsCtrlError::Config(format!("Watch register failed: {}", e)))?;
            Some(w)
        } else {
            None
        };

        info!(
            "✅ Config loaded: ID={}, Master={}, dynamic_load_enable={}",
            static_cfg.my_id, static_cfg.is_master, static_cfg.dynamic_load_enable
        );

        Ok(Self {
            static_cfg,
            dynamic_data,
            _watcher: watcher,
            config_path: path_buf,
        })
    }

    fn reload_dynamic(path: &Path, data_lock: &Arc<RwLock<D>>) {
        // info!("🔄 Reloading dynamic config...");
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(val) = toml::from_str::<toml::Value>(&content) {
                if let Some(dyn_val) = val.get("dynamic") {
                    if let Ok(new_data) = dyn_val.clone().try_into::<D>() {
                        if let Ok(mut guard) = data_lock.write() {
                            *guard = new_data;
                            // info!("✨ Dynamic config updated.");
                            return;
                        } else {
                            warn!("⚠️ Dynamic config lock poisoned, skip update.");
                            return;
                        }
                    }
                }
            }
        }
        warn!("⚠️ Dynamic config reload failed.");
    }

    pub fn static_cfg(&self) -> &StaticBase { &self.static_cfg }
    pub fn get_dynamic_clone(&self) -> D {
        self.dynamic_data
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
    pub fn config_path(&self) -> &Path { &self.config_path }
}