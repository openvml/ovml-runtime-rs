//! Piper voice catalog, model download, and local ONNX synthesis.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use piper_rs::Piper;
use serde::Deserialize;

use crate::audio::{build_wav, pcm_f32_to_i16, wav_duration_ms};
use crate::voice::VoiceInfo;

const DOWNLOAD_BASES: [(&str, &str); 2] = [
    (
        "rhasspy",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main",
    ),
    (
        "mirror",
        "https://huggingface.co/diffusionstudio/piper-voices/resolve/main",
    ),
];
const MIN_MODEL_BYTES: u64 = 1_000_000;
const MAX_LOADED_VOICES: usize = 4;
const BUNDLED_CATALOG: &str = include_str!("../data/piper_voices.json");

#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    voice_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    lang: String,
    #[serde(default)]
    gender: String,
    #[serde(default)]
    quality: String,
    #[serde(default)]
    sample_rate: u32,
    #[serde(default)]
    sample_url: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    path: String,
}

#[derive(Debug, Clone)]
pub struct PiperAudio {
    pub audio: Vec<u8>,
    pub mime: &'static str,
    pub sample_rate: u32,
    pub duration_ms: u32,
}

struct LoadedVoice {
    voice: Arc<Mutex<Piper>>,
}

pub struct PiperEngine {
    catalog: HashMap<String, CatalogEntry>,
    models_dir: PathBuf,
    loaded: Mutex<Vec<(String, LoadedVoice)>>,
    load_lock: Mutex<()>,
    voice_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl PiperEngine {
    pub fn new(catalog_path: Option<&Path>, models_dir: PathBuf) -> Result<Self, String> {
        let raw = match catalog_path.filter(|path| path.is_file()) {
            Some(path) => fs::read_to_string(path)
                .map_err(|err| format!("read piper catalog {}: {err}", path.display()))?,
            None => BUNDLED_CATALOG.to_string(),
        };
        let items: Vec<CatalogEntry> =
            serde_json::from_str(&raw).map_err(|err| format!("parse piper catalog: {err}"))?;
        let catalog = items
            .into_iter()
            .map(|entry| (entry.voice_id.clone(), entry))
            .collect();
        fs::create_dir_all(&models_dir).map_err(|err| err.to_string())?;
        Ok(Self {
            catalog,
            models_dir,
            loaded: Mutex::new(Vec::new()),
            load_lock: Mutex::new(()),
            voice_locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn catalog_len(&self) -> usize {
        self.catalog.len()
    }

    pub fn has_voice(&self, voice_id: &str) -> bool {
        self.catalog.contains_key(voice_id)
    }

    pub fn voices(&self) -> Vec<VoiceInfo> {
        self.catalog
            .values()
            .map(|entry| {
                let (onnx, _) = self.model_paths(&entry.voice_id);
                VoiceInfo {
                    id: entry.voice_id.clone(),
                    name: if entry.name.is_empty() {
                        entry.voice_id.clone()
                    } else {
                        entry.name.clone()
                    },
                    language: entry.lang.clone(),
                    gender: if entry.gender.is_empty() {
                        "neutral".into()
                    } else {
                        entry.gender.clone()
                    },
                    quality: entry.quality.clone(),
                    sample_rate: entry.sample_rate,
                    sample_url: entry.sample_url.clone(),
                    downloaded: model_is_ready(&onnx),
                }
            })
            .collect()
    }

    fn model_paths(&self, voice_id: &str) -> (PathBuf, PathBuf) {
        let onnx = self.models_dir.join(format!("{voice_id}.onnx"));
        let config = self.models_dir.join(format!("{voice_id}.onnx.json"));
        (onnx, config)
    }

    fn voice_lock(&self, voice_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self
            .voice_locks
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        locks
            .entry(voice_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn cached_voice(&self, voice_id: &str) -> Option<Arc<Mutex<Piper>>> {
        let mut loaded = self.loaded.lock().unwrap_or_else(|err| err.into_inner());
        let index = loaded.iter().position(|(id, _)| id == voice_id)?;
        let entry = loaded.remove(index);
        let voice = Arc::clone(&entry.1.voice);
        loaded.push(entry);
        Some(voice)
    }

    fn voice(&self, voice_id: &str) -> Result<Arc<Mutex<Piper>>, String> {
        if let Some(voice) = self.cached_voice(voice_id) {
            return Ok(voice);
        }
        let voice_lock = self.voice_lock(voice_id);
        let _guard = voice_lock.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(voice) = self.cached_voice(voice_id) {
            return Ok(voice);
        }
        let _load_guard = self.load_lock.lock().unwrap_or_else(|err| err.into_inner());
        let (onnx, config) = self.ensure_model(voice_id)?;
        let piper = Piper::new(&onnx, &config).map_err(|err| err.to_string())?;
        let voice = Arc::new(Mutex::new(piper));
        let mut loaded = self.loaded.lock().unwrap_or_else(|err| err.into_inner());
        loaded.push((
            voice_id.to_string(),
            LoadedVoice {
                voice: Arc::clone(&voice),
            },
        ));
        while loaded.len() > MAX_LOADED_VOICES {
            loaded.remove(0);
        }
        Ok(voice)
    }

    pub fn ensure_model(&self, voice_id: &str) -> Result<(PathBuf, PathBuf), String> {
        let entry = self
            .catalog
            .get(voice_id)
            .ok_or_else(|| format!("unknown piper voice: {voice_id}"))?;
        let (onnx, config) = self.model_paths(voice_id);
        if model_is_ready(&onnx) && config_is_valid(&config) {
            return Ok((onnx, config));
        }

        let base = DOWNLOAD_BASES
            .iter()
            .find(|(name, _)| *name == entry.source)
            .map(|(_, base)| *base)
            .unwrap_or(DOWNLOAD_BASES[0].1);
        let relative = if entry.path.is_empty() {
            format!("{voice_id}.onnx")
        } else {
            entry.path.clone()
        };
        let model_url = format!("{}/{}", base.trim_end_matches('/'), relative);
        let config_url = format!("{model_url}.json");
        download(&model_url, &onnx)?;
        download(&config_url, &config)?;
        Ok((onnx, config))
    }

    pub fn ensure(&self, voice_ids: &[String]) -> Result<(), String> {
        for voice_id in voice_ids {
            self.ensure_model(voice_id)?;
        }
        Ok(())
    }

    pub fn synthesize(&self, text: &str, voice_id: &str, rate: f32) -> Result<PiperAudio, String> {
        let voice = self.voice(voice_id)?;
        let length_scale = if (rate - 1.0).abs() > 1e-3 {
            Some(1.0 / rate.clamp(0.5, 2.0))
        } else {
            None
        };
        let mut guard = voice.lock().unwrap_or_else(|err| err.into_inner());
        let (samples, sample_rate) = guard
            .create(text, false, None, length_scale, None, None)
            .map_err(|err| err.to_string())?;
        if samples.is_empty() || sample_rate == 0 {
            return Err("piper produced no audio".into());
        }
        let pcm = pcm_f32_to_i16(&samples);
        let wav = build_wav(&pcm, sample_rate);
        let duration_ms = wav_duration_ms(&wav).unwrap_or(0);
        Ok(PiperAudio {
            audio: wav,
            mime: "audio/wav",
            sample_rate,
            duration_ms,
        })
    }
}

fn model_is_ready(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > MIN_MODEL_BYTES)
        .unwrap_or(false)
}

fn config_is_valid(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .is_some()
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let tmp = PathBuf::from(format!("{}.part", dest.display()));
    let result = (|| -> Result<(), String> {
        let response = ureq::get(url)
            .set("User-Agent", "ovml-runtime/0.1")
            .timeout(Duration::from_secs(300))
            .call()
            .map_err(|err| format!("download {url}: {err}"))?;
        let mut reader = response.into_reader();
        let mut file = fs::File::create(&tmp).map_err(|err| err.to_string())?;
        io::copy(&mut reader, &mut file).map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;
        fs::rename(&tmp, dest).map_err(|err| err.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_loads() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PiperEngine::new(None, dir.path().to_path_buf()).unwrap();
        assert_eq!(engine.catalog_len(), 170);
        assert!(engine.has_voice("en_US-amy-low"));
        let voices = engine.voices();
        assert_eq!(voices.len(), 170);
    }

    #[test]
    fn model_paths_use_voice_id() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PiperEngine::new(None, dir.path().to_path_buf()).unwrap();
        let (onnx, config) = engine.model_paths("en_US-amy-low");
        assert_eq!(onnx.file_name().unwrap(), "en_US-amy-low.onnx");
        assert_eq!(config.file_name().unwrap(), "en_US-amy-low.onnx.json");
    }
}
