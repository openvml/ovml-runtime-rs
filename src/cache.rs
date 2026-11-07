//! Content-addressed audio cache with size-bounded pruning.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sentence::Word;

pub const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    pub text: String,
    pub mime: String,
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub words: Vec<Word>,
    pub approx_words: bool,
    pub engine: String,
    pub voice: String,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct CachedAudio {
    pub audio: Vec<u8>,
    pub meta: CacheMeta,
}

pub struct AudioCache {
    root: PathBuf,
    max_bytes: u64,
    enabled: bool,
    writes: u64,
}

impl AudioCache {
    pub fn new(root: PathBuf, max_bytes: u64, enabled: bool) -> Self {
        let cache = Self {
            root,
            max_bytes,
            enabled: enabled && max_bytes > 0,
            writes: 0,
        };
        if cache.enabled {
            let _ = fs::create_dir_all(&cache.root);
            cache.prune();
        }
        cache
    }

    pub fn key(&self, engine: &str, voice: &str, rate: f32, pitch: i32, text: &str) -> String {
        let payload = format!(
            "{}\x1f{}\x1f{:.3}\x1f{}\x1f{}\x1f{}",
            engine,
            voice,
            rate,
            pitch,
            text.chars().count(),
            text
        );
        hex::encode(Sha256::digest(payload.as_bytes()))
    }

    fn paths(&self, key: &str) -> (PathBuf, PathBuf) {
        (
            self.root.join(format!("{key}.bin")),
            self.root.join(format!("{key}.json")),
        )
    }

    pub fn get(&mut self, key: &str) -> Option<CachedAudio> {
        if !self.enabled {
            return None;
        }
        let (audio_path, meta_path) = self.paths(key);
        let audio = fs::read(audio_path).ok()?;
        let meta: CacheMeta = serde_json::from_slice(&fs::read(meta_path).ok()?).ok()?;
        Some(CachedAudio { audio, meta })
    }

    pub fn put(&mut self, key: &str, audio: &[u8], mut meta: CacheMeta) {
        if !self.enabled || audio.len() > MAX_ENTRY_BYTES {
            return;
        }
        if meta.created_at == 0 {
            meta.created_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_secs())
                .unwrap_or(0);
        }
        let (audio_path, meta_path) = self.paths(key);
        if let Ok(json) = serde_json::to_vec(&meta) {
            atomic_write(&audio_path, audio);
            atomic_write(&meta_path, &json);
        }
        self.writes += 1;
        if self.writes % 32 == 0 {
            self.prune();
        }
    }

    pub fn clear(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match path.extension().and_then(|value| value.to_str()) {
                Some("bin" | "json") => {
                    let _ = fs::remove_file(path);
                }
                _ => {}
            }
        }
    }

    pub fn len(&self) -> u64 {
        self.list_entries().iter().map(|entry| entry.size).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn list_entries(&self) -> Vec<Entry> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("bin") {
                    return None;
                }
                let meta = entry.metadata().ok()?;
                Some(Entry {
                    path,
                    size: meta.len(),
                    modified: meta.modified().ok(),
                })
            })
            .collect()
    }

    fn prune(&self) {
        if self.max_bytes == 0 {
            return;
        }
        let mut entries = self.list_entries();
        let total: u64 = entries.iter().map(|entry| entry.size).sum();
        if total <= self.max_bytes {
            return;
        }
        let target = self.max_bytes.saturating_mul(9) / 10;
        entries.sort_by_key(|entry| entry.modified);
        let mut current = total;
        for entry in entries {
            if current <= target {
                break;
            }
            if fs::remove_file(&entry.path).is_ok() {
                current = current.saturating_sub(entry.size);
                let _ = fs::remove_file(entry.path.with_extension("json"));
            }
        }
    }
}

struct Entry {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

fn atomic_write(path: &Path, data: &[u8]) {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    if fs::write(&tmp, data).is_ok() {
        let _ = fs::rename(&tmp, path);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> CacheMeta {
        CacheMeta {
            text: "hello".into(),
            mime: "audio/wav".into(),
            sample_rate: 22050,
            duration_ms: 500,
            words: Vec::new(),
            approx_words: true,
            engine: "piper".into(),
            voice: "en_US-amy-low".into(),
            created_at: 0,
        }
    }

    #[test]
    fn roundtrips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::new(dir.path().to_path_buf(), 1024 * 1024, true);
        let key = cache.key("piper", "voice", 1.0, 0, "hello");
        cache.put(&key, b"audio", meta());
        let cached = cache.get(&key).unwrap();
        assert_eq!(cached.audio, b"audio");
        assert_eq!(cached.meta.duration_ms, 500);
        cache.clear();
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn key_is_stable_and_voice_sensitive() {
        let cache = AudioCache::new(PathBuf::from("unused"), 0, false);
        let a = cache.key("piper", "a", 1.0, 0, "text");
        assert_eq!(a, cache.key("piper", "a", 1.0, 0, "text"));
        assert_ne!(a, cache.key("piper", "b", 1.0, 0, "text"));
    }

    #[test]
    fn disabled_cache_is_noop() {
        let mut cache = AudioCache::new(PathBuf::from("unused"), 0, false);
        let key = cache.key("piper", "a", 1.0, 0, "text");
        cache.put(&key, b"audio", meta());
        assert!(cache.get(&key).is_none());
    }
}
