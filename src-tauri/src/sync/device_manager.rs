use std::collections::{HashMap, HashSet};
use std::sync::Arc;


use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub device_id: String,
    pub public_key_b64: String,
    pub paired_at: i64,
    pub revoked: bool,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRevokeResult {
    pub device_id: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub public_key_b64: String,
    pub paired_at: i64,
    pub revoked: bool,
    pub revoked_at: Option<i64>,
}

impl From<&PairedDevice> for DeviceInfo {
    fn from(d: &PairedDevice) -> Self {
        Self {
            device_id: d.device_id.clone(),
            public_key_b64: d.public_key_b64.clone(),
            paired_at: d.paired_at,
            revoked: d.revoked,
            revoked_at: d.revoked_at,
        }
    }
}

pub struct DeviceManager {
    devices: HashMap<String, PairedDevice>,
    revoked_ids: HashSet<String>,
}

impl DeviceManager {
    pub fn new(_conn: Arc<parking_lot::Mutex<rusqlite::Connection>>) -> Self {
        Self {
            devices: HashMap::new(),
            revoked_ids: HashSet::new(),
        }
    }

    pub fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        Ok(self.devices.values().map(|d| DeviceInfo::from(d)).collect())
    }

    pub fn register_device(&mut self, device_id: String, public_key_b64: String, paired_at: i64) {
        let device = PairedDevice {
            device_id: device_id.clone(),
            public_key_b64,
            paired_at,
            revoked: false,
            revoked_at: None,
        };
        self.devices.insert(device_id, device);
    }

    pub fn revoke_device(&mut self, device_id: &str) -> DeviceRevokeResult {
        match self.devices.get_mut(device_id) {
            Some(device) => {
                if device.revoked {
                    DeviceRevokeResult {
                        device_id: device_id.to_string(),
                        success: false,
                        error: Some("device already revoked".to_string()),
                    }
                } else {
                    let now = chrono::Utc::now().timestamp();
                    device.revoked = true;
                    device.revoked_at = Some(now);
                    self.revoked_ids.insert(device_id.to_string());
                    info!(target: "nine_snake.device_manager", device_id, "device revoked");
                    DeviceRevokeResult {
                        device_id: device_id.to_string(),
                        success: true,
                        error: None,
                    }
                }
            }
            None => DeviceRevokeResult {
                device_id: device_id.to_string(),
                success: false,
                error: Some("device not found".to_string()),
            },
        }
    }

    pub fn is_device_revoked(&self, device_id: &str) -> bool {
        self.revoked_ids.contains(device_id)
    }

    pub fn list_paired_devices(&self) -> Vec<&PairedDevice> {
        self.devices.values().collect()
    }

    pub fn list_active_devices(&self) -> Vec<&PairedDevice> {
        self.devices.values().filter(|d| !d.revoked).collect()
    }

    pub fn get_device(&self, device_id: &str) -> Option<&PairedDevice> {
        self.devices.get(device_id)
    }

    pub fn validate_device(&self, device_id: &str) -> Result<(), String> {
        if self.is_device_revoked(device_id) {
            warn!(target: "nine_snake.device_manager", device_id, "revoked device attempted communication");
            Err("device has been revoked".to_string())
        } else if !self.devices.contains_key(device_id) {
            Err("device not found".to_string())
        } else {
            Ok(())
        }
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self {
            devices: HashMap::new(),
            revoked_ids: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mgr() -> DeviceManager {
        let conn = Arc::new(parking_lot::Mutex::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ));
        DeviceManager::new(conn)
    }

    #[test]
    fn register_and_list() {
        let mut mgr = test_mgr();
        mgr.register_device("dev-1".into(), "pubkey1".into(), 1000);
        mgr.register_device("dev-2".into(), "pubkey2".into(), 2000);
        assert_eq!(mgr.list_paired_devices().len(), 2);
        assert_eq!(mgr.list_active_devices().len(), 2);
    }

    #[test]
    fn revoke_device() {
        let mut mgr = test_mgr();
        mgr.register_device("dev-1".into(), "pubkey1".into(), 1000);
        let result = mgr.revoke_device("dev-1");
        assert!(result.success);
        assert!(mgr.is_device_revoked("dev-1"));
        assert_eq!(mgr.list_active_devices().len(), 0);
    }

    #[test]
    fn revoke_already_revoked() {
        let mut mgr = test_mgr();
        mgr.register_device("dev-1".into(), "pubkey1".into(), 1000);
        mgr.revoke_device("dev-1");
        let result = mgr.revoke_device("dev-1");
        assert!(!result.success);
    }

    #[test]
    fn revoke_nonexistent() {
        let mut mgr = test_mgr();
        let result = mgr.revoke_device("ghost");
        assert!(!result.success);
    }

    #[test]
    fn validate_revoked_device() {
        let mut mgr = test_mgr();
        mgr.register_device("dev-1".into(), "pubkey1".into(), 1000);
        mgr.revoke_device("dev-1");
        assert!(mgr.validate_device("dev-1").is_err());
    }

    #[test]
    fn validate_active_device() {
        let mut mgr = test_mgr();
        mgr.register_device("dev-1".into(), "pubkey1".into(), 1000);
        assert!(mgr.validate_device("dev-1").is_ok());
    }
}