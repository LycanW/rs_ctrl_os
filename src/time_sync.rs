use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

#[derive(Debug, Clone)]
pub struct TimeSyncState {
    pub is_synced: bool,
    pub offset_ms: i64,
    pub master_id: Option<String>,
}

pub struct TimeSynchronizer {
    state: Arc<RwLock<TimeSyncState>>,
}

impl TimeSynchronizer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(TimeSyncState {
                is_synced: false,
                offset_ms: 0,
                master_id: None,
            })),
        }
    }

    pub fn update_from_master(&self, master_id: &str, master_ts_ms: u64) {
        let local_ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let raw_offset = (local_ts_ms as i64) - (master_ts_ms as i64);

        if let Ok(mut state) = self.state.write() {
            if !state.is_synced {
                state.offset_ms = raw_offset;
                state.is_synced = true;
                state.master_id = Some(master_id.to_string());
                info!(
                    "⏱️ Time Synced: Master={}, Offset={}ms",
                    master_id, state.offset_ms
                );
            } else {
                // Low-pass filter
                state.offset_ms =
                    ((state.offset_ms as f64 * 0.9) + (raw_offset as f64 * 0.1)).round() as i64;
            }
        }
    }

    pub fn now_corrected_ms(&self) -> u64 {
        let state_guard = self.state.read().ok();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let Some(state) = state_guard else {
            return now;
        };

        // offset = local - master; subtract it to arrive at master time.
        now.saturating_add_signed(-state.offset_ms)
    }

    // Helper for examples to check status without exposing internals too much
    pub fn is_synced(&self) -> bool {
        self.state.read().map(|s| s.is_synced).unwrap_or(false)
    }
}
