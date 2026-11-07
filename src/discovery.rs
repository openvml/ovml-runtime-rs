//! Loopback port selection and discovery file lifecycle.

use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::{set_private_permissions, Config};
use crate::VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub schema: u8,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
    pub engines: Vec<String>,
    pub started_at: u64,
}

pub fn find_free_port(host: &str, preferred: u16, attempts: u16) -> io::Result<u16> {
    for offset in 0..=attempts {
        let port = preferred.saturating_add(offset);
        if TcpListener::bind((host, port)).is_ok() {
            return Ok(port);
        }
    }
    let listener = TcpListener::bind((host, 0))?;
    Ok(listener.local_addr()?.port())
}

pub fn write_discovery(config: &Config, port: u16, engines: Vec<String>) -> io::Result<PathBuf> {
    let payload = Discovery {
        schema: 1,
        host: config.host.clone(),
        port,
        token: config.token.clone(),
        pid: std::process::id(),
        version: VERSION.to_string(),
        engines,
        started_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0),
    };
    let path = config.discovery_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&payload)?)?;
    set_private_permissions(&tmp);
    fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn remove_discovery(config: &Config) {
    let path = config.discovery_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    if value.get("pid").and_then(|pid| pid.as_u64()) == Some(u64::from(std::process::id())) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port() {
        let port = find_free_port("127.0.0.1", 47821, 20).unwrap();
        assert!(port >= 47821);
    }

    #[test]
    fn writes_and_removes_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.data_dir = dir.path().to_path_buf();
        config.ensure_dirs().unwrap();
        config.load_or_create_token().unwrap();
        let path = write_discovery(&config, 47821, vec!["piper".into()]).unwrap();
        let value: Discovery = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value.port, 47821);
        assert_eq!(value.engines, vec!["piper"]);
        remove_discovery(&config);
        assert!(!path.exists());
    }
}
