use crate::error::{Result, RsCtrlError};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

#[derive(Debug, Deserialize, Clone)]
pub struct StaticBase {
    pub my_id: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub is_master: bool,
    #[serde(default)]
    pub subscribers: HashMap<String, String>, 
    #[serde(default)]
    pub publishers: HashMap<String, String>, 
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
    _watcher: RecommendedWatcher,
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
        let dyn_clone = dynamic_data.clone();
        let path_clone = path_buf.clone();
        let target_name = path_clone
            .file_name()
            .map(|n| n.to_owned());

        let mut watcher = RecommendedWatcher::new(
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

        watcher
            .watch(config_path, RecursiveMode::NonRecursive)
            .map_err(|e| RsCtrlError::Config(format!("Watch register failed: {}", e)))?;
        info!("✅ Config loaded: ID={}, Master={}", static_cfg.my_id, static_cfg.is_master);

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