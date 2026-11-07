//! Runtime configuration and on-disk paths.

use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

pub const DEFAULT_PORT: u16 = 47821;
pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_CACHE_MB: u64 = 512;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub data_dir: PathBuf,
    pub concurrency: usize,
    pub cache_mb: u64,
    pub cache_enabled: bool,
    pub piper_catalog: Option<PathBuf>,
    pub edge_voices_cache: PathBuf,
    pub edge_enabled: bool,
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn default_concurrency() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2);
    (cores.saturating_sub(1)).clamp(1, 4)
}

pub fn default_data_dir() -> PathBuf {
    if let Some(value) = env_string("OVML_RUNTIME_DATA") {
        return PathBuf::from(value);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openvml")
        .join("runtime")
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = default_data_dir();
        Self {
            host: env_string("OVML_RUNTIME_HOST").unwrap_or_else(|| DEFAULT_HOST.to_string()),
            port: env_string("OVML_RUNTIME_PORT")
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            token: env_string("OVML_RUNTIME_TOKEN").unwrap_or_default(),
            edge_voices_cache: data_dir.join("edge_voices.json"),
            data_dir,
            concurrency: env_string("OVML_RUNTIME_CONCURRENCY")
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(default_concurrency)
                .max(1),
            cache_mb: env_string("OVML_RUNTIME_CACHE_MB")
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_CACHE_MB),
            cache_enabled: true,
            piper_catalog: env_string("OVML_PIPER_CATALOG").map(PathBuf::from),
            edge_enabled: true,
        }
    }
}

impl Config {
    pub fn piper_models_dir(&self) -> PathBuf {
        self.data_dir.join("models").join("piper")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn discovery_path(&self) -> PathBuf {
        self.data_dir.join("runtime.json")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(self.piper_models_dir())?;
        fs::create_dir_all(self.cache_dir())?;
        Ok(())
    }

    pub fn load_or_create_token(&mut self) -> std::io::Result<()> {
        if !self.token.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.data_dir)?;
        let token_file = self.data_dir.join("token");
        if let Ok(value) = fs::read_to_string(&token_file) {
            let value = value.trim();
            if !value.is_empty() {
                self.token = value.to_string();
                return Ok(());
            }
        }
        let value = Uuid::new_v4().simple().to_string();
        fs::write(&token_file, &value)?;
        set_private_permissions(&token_file);
        self.token = value;
        Ok(())
    }

    pub fn from_args(args: &[String]) -> Result<Self, String> {
        let mut config = Self::default();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let mut next_value = || {
                iter.next()
                    .cloned()
                    .ok_or_else(|| format!("missing value for {arg}"))
            };
            match arg.as_str() {
                "--host" => config.host = next_value()?,
                "--port" => {
                    config.port = next_value()?
                        .parse()
                        .map_err(|_| "invalid --port".to_string())?
                }
                "--token" => config.token = next_value()?,
                "--data-dir" => config.data_dir = PathBuf::from(next_value()?),
                "--concurrency" => {
                    config.concurrency = next_value()?
                        .parse::<usize>()
                        .map_err(|_| "invalid --concurrency".to_string())?
                        .max(1)
                }
                "--cache-mb" => {
                    config.cache_mb = next_value()?
                        .parse()
                        .map_err(|_| "invalid --cache-mb".to_string())?
                }
                "--no-cache" => config.cache_enabled = false,
                "--no-edge" => config.edge_enabled = false,
                "--piper-catalog" => config.piper_catalog = Some(PathBuf::from(next_value()?)),
                "--version" | "-h" | "--help" => {}
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        config.edge_voices_cache = config.data_dir.join("edge_voices.json");
        config.ensure_dirs().map_err(|err| err.to_string())?;
        config
            .load_or_create_token()
            .map_err(|err| err.to_string())?;
        Ok(config)
    }
}

pub fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let dir = tempfile::tempdir().unwrap();
        let args = vec![
            "--port".to_string(),
            "50000".to_string(),
            "--data-dir".to_string(),
            dir.path().display().to_string(),
            "--no-edge".to_string(),
            "--cache-mb".to_string(),
            "64".to_string(),
        ];
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 50000);
        assert!(!config.edge_enabled);
        assert_eq!(config.cache_mb, 64);
        assert!(!config.token.is_empty());
        assert!(config.piper_models_dir().is_dir());
        assert!(config.cache_dir().is_dir());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(Config::from_args(&["--nope".to_string()]).is_err());
    }
}
