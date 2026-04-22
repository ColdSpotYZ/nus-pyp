use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::models::{APP_IDENTIFIER, SESSION_STORE_FILE};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub cookie_header: String,
    pub source_url: String,
    pub saved_at_epoch_ms: u128,
}

impl SessionSnapshot {
    pub fn new(cookie_header: String, source_url: String) -> Self {
        Self {
            cookie_header,
            source_url,
            saved_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default(),
        }
    }
}

pub fn session_store_path() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Unable to determine the application data directory.".to_string())?;
    let app_dir = data_dir.join(APP_IDENTIFIER);
    fs::create_dir_all(&app_dir).map_err(|error| error.to_string())?;
    Ok(app_dir.join(SESSION_STORE_FILE))
}

pub fn load_session_snapshot() -> Result<SessionSnapshot, String> {
    let path = session_store_path()?;
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

pub fn save_session_snapshot(snapshot: &SessionSnapshot) -> Result<PathBuf, String> {
    let path = session_store_path()?;
    let contents = serde_json::to_string(snapshot).map_err(|error| error.to_string())?;
    fs::write(&path, contents).map_err(|error| error.to_string())?;
    Ok(path)
}

pub fn clear_session_snapshot() -> Result<(), String> {
    let path = session_store_path()?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SessionSnapshot;

    #[test]
    fn creates_snapshot_with_timestamp() {
        let snapshot = SessionSnapshot::new(
            "foo=bar".to_string(),
            "https://digitalgems.nus.edu.sg/browse/collection/31".to_string(),
        );

        assert_eq!(snapshot.cookie_header, "foo=bar");
        assert!(snapshot.saved_at_epoch_ms > 0);
    }
}
