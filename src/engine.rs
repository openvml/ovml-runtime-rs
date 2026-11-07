//! Engine registry, voice resolution, and cached synthesis orchestration.

use std::sync::{Arc, Mutex};

use async_stream::stream;
use futures_util::Stream;
use serde::Deserialize;

use crate::audio::{build_wav, duration_ms, wav_pcm};
use crate::cache::{AudioCache, CacheMeta};
use crate::config::Config;
use crate::edge::EdgeEngine;
use crate::piper::PiperEngine;
use crate::sentence::{split_sentences, word_timings, Word};
use crate::voice::VoiceInfo;

const MAX_SENTENCE_CHARS: usize = 240;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SynthesisRequest {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_rate")]
    pub rate: f32,
    #[serde(default)]
    pub pitch: i32,
}

fn default_rate() -> f32 {
    1.0
}

#[derive(Debug, Clone)]
pub struct SynthChunk {
    pub index: usize,
    pub text: String,
    pub engine: String,
    pub voice: String,
    pub audio: Vec<u8>,
    pub mime: String,
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub words: Vec<Word>,
    pub approx_words: bool,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Chunk(SynthChunk),
    Done {
        index: usize,
        engine: String,
        voice: String,
    },
}

#[derive(Debug, Clone)]
pub struct Synthesis {
    pub audio: Vec<u8>,
    pub mime: String,
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub words: Vec<Word>,
    pub engine: String,
    pub voice: String,
}

pub struct RuntimeEngine {
    config: Config,
    piper: Option<Arc<PiperEngine>>,
    edge: Option<Arc<EdgeEngine>>,
    cache: Arc<Mutex<AudioCache>>,
}

impl RuntimeEngine {
    pub fn new(config: Config) -> Result<Self, String> {
        let piper = PiperEngine::new(config.piper_catalog.as_deref(), config.piper_models_dir())
            .ok()
            .map(Arc::new);
        let edge = if config.edge_enabled {
            Some(Arc::new(EdgeEngine::new(Some(
                config.edge_voices_cache.clone(),
            ))))
        } else {
            None
        };
        let cache = AudioCache::new(
            config.cache_dir(),
            config.cache_mb.saturating_mul(1024 * 1024),
            config.cache_enabled,
        );
        Ok(Self {
            config,
            piper,
            edge,
            cache: Arc::new(Mutex::new(cache)),
        })
    }

    pub fn engine_ids(&self) -> Vec<&'static str> {
        let mut ids = Vec::new();
        if self.piper.is_some() {
            ids.push("piper");
        }
        if self.edge.is_some() {
            ids.push("edge");
        }
        ids
    }

    pub fn has_engine(&self, engine: &str) -> bool {
        match engine {
            "piper" => self.piper.is_some(),
            "edge" => self.edge.is_some(),
            _ => false,
        }
    }

    pub fn piper_voices(&self) -> Vec<VoiceInfo> {
        self.piper
            .as_ref()
            .map(|engine| engine.voices())
            .unwrap_or_default()
    }

    pub async fn edge_voices(&self) -> Vec<VoiceInfo> {
        let Some(edge) = &self.edge else {
            return Vec::new();
        };
        if edge.is_empty() {
            match edge.refresh_voices().await {
                Ok(count) => log::info!("edge voice list loaded: {count}"),
                Err(err) => log::warn!("edge voice list unavailable: {err}"),
            }
        }
        edge.voices()
    }

    pub async fn refresh_edge_voices(&self) -> Result<usize, String> {
        match &self.edge {
            Some(edge) => edge.refresh_voices().await,
            None => Err("edge is not available".into()),
        }
    }

    async fn voices_for(&self, engine: &str) -> Vec<VoiceInfo> {
        match engine {
            "piper" => self.piper_voices(),
            "edge" => self.edge_voices().await,
            _ => Vec::new(),
        }
    }

    pub async fn resolve(
        &self,
        engine: Option<&str>,
        voice: Option<&str>,
        language: Option<&str>,
    ) -> Result<(String, VoiceInfo), String> {
        let mut engine = engine.map(str::to_string);
        if let Some(name) = &engine {
            if !self.has_engine(name) {
                return Err(format!("engine is not available: {name}"));
            }
        }
        if engine.is_none() {
            if let Some(voice) = voice {
                if self
                    .piper
                    .as_ref()
                    .map(|piper| piper.has_voice(voice))
                    .unwrap_or(false)
                {
                    engine = Some("piper".into());
                } else if self.edge_voices().await.iter().any(|item| item.id == voice) {
                    engine = Some("edge".into());
                }
            }
            if engine.is_none() {
                engine = Some(
                    if self.piper.is_some() {
                        "piper"
                    } else {
                        "edge"
                    }
                    .into(),
                );
            }
        }
        let engine = engine.unwrap_or_else(|| "piper".into());
        let available = self.voices_for(&engine).await;
        if available.is_empty() {
            return Err(format!("no voices available for {engine}"));
        }
        if let Some(voice) = voice {
            let info = available
                .into_iter()
                .find(|item| item.id == voice)
                .ok_or_else(|| format!("unknown voice: {voice}"))?;
            return Ok((engine, info));
        }
        let mut candidates = available;
        if let Some(language) = language {
            let base = language
                .replace('_', "-")
                .split('-')
                .next()
                .unwrap_or_default()
                .to_lowercase();
            let filtered: Vec<VoiceInfo> = candidates
                .iter()
                .filter(|item| {
                    item.language
                        .replace('_', "-")
                        .split('-')
                        .next()
                        .unwrap_or_default()
                        .eq_ignore_ascii_case(&base)
                })
                .cloned()
                .collect();
            if !filtered.is_empty() {
                candidates = filtered;
            }
        }
        candidates.sort_by_key(|item| quality_rank(&item.quality));
        let info = candidates
            .into_iter()
            .next()
            .ok_or_else(|| format!("no voices available for {engine}"))?;
        Ok((engine, info))
    }

    pub async fn ensure(&self, engine: &str, voice: &str) -> Result<(), String> {
        match engine {
            "piper" => {
                let piper = self
                    .piper
                    .clone()
                    .ok_or_else(|| "piper is not available".to_string())?;
                let voice = voice.to_string();
                tokio::task::spawn_blocking(move || piper.ensure(&[voice]))
                    .await
                    .map_err(|err| err.to_string())?
            }
            "edge" => {
                if self.edge.is_none() {
                    return Err("edge is not available".into());
                }
                if self.edge_voices().await.is_empty() {
                    return Err("edge voice list is empty".into());
                }
                Ok(())
            }
            other => Err(format!("unknown engine: {other}")),
        }
    }

    pub fn stream(
        &self,
        request: SynthesisRequest,
    ) -> impl Stream<Item = Result<StreamEvent, String>> + '_ {
        stream! {
            let text = request.text.trim().to_string();
            if text.is_empty() {
                yield Err("text is required".into());
                return;
            }
            let (engine, info) = match self
                .resolve(
                    request.engine.as_deref(),
                    request.voice.as_deref(),
                    request.language.as_deref(),
                )
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    yield Err(err);
                    return;
                }
            };
            let sentences = split_sentences(&text, MAX_SENTENCE_CHARS);
            if sentences.is_empty() {
                yield Err("no sentences produced".into());
                return;
            }
            let rate = request.rate.clamp(0.5, 2.0);
            for (index, sentence) in sentences.iter().enumerate() {
                let key = {
                    let cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
                    cache.key(&engine, &info.id, rate, request.pitch, sentence)
                };
                let cached = {
                    let mut cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
                    cache.get(&key)
                };
                if let Some(cached) = cached {
                    yield Ok(StreamEvent::Chunk(SynthChunk {
                        index,
                        text: cached.meta.text,
                        engine: engine.clone(),
                        voice: info.id.clone(),
                        audio: cached.audio,
                        mime: cached.meta.mime,
                        sample_rate: cached.meta.sample_rate,
                        duration_ms: cached.meta.duration_ms,
                        words: cached.meta.words,
                        approx_words: cached.meta.approx_words,
                    }));
                    continue;
                }
                let produced = if engine == "piper" {
                    self.synthesize_piper(sentence, &info.id, rate).await
                } else {
                    self.synthesize_edge(sentence, &info.id, rate, request.pitch).await
                };
                let (audio, mime, sample_rate, duration, words, approx_words) = match produced {
                    Ok(value) => value,
                    Err(err) => {
                        yield Err(err);
                        return;
                    }
                };
                let meta = CacheMeta {
                    text: sentence.clone(),
                    mime: mime.clone(),
                    sample_rate,
                    duration_ms: duration,
                    words: words.clone(),
                    approx_words,
                    engine: engine.clone(),
                    voice: info.id.clone(),
                    created_at: 0,
                };
                {
                    let mut cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
                    cache.put(&key, &audio, meta);
                }
                yield Ok(StreamEvent::Chunk(SynthChunk {
                    index,
                    text: sentence.clone(),
                    engine: engine.clone(),
                    voice: info.id.clone(),
                    audio,
                    mime,
                    sample_rate,
                    duration_ms: duration,
                    words,
                    approx_words,
                }));
            }
            yield Ok(StreamEvent::Done {
                index: sentences.len(),
                engine,
                voice: info.id,
            });
        }
    }

    async fn synthesize_piper(
        &self,
        sentence: &str,
        voice: &str,
        rate: f32,
    ) -> Result<(Vec<u8>, String, u32, u32, Vec<Word>, bool), String> {
        let piper = self
            .piper
            .clone()
            .ok_or_else(|| "piper is not available".to_string())?;
        let sentence = sentence.to_string();
        let words_text = sentence.clone();
        let voice = voice.to_string();
        let result = tokio::task::spawn_blocking(move || piper.synthesize(&sentence, &voice, rate))
            .await
            .map_err(|err| err.to_string())??;
        let words = word_timings(&words_text, result.duration_ms);
        Ok((
            result.audio,
            result.mime.to_string(),
            result.sample_rate,
            result.duration_ms,
            words,
            true,
        ))
    }

    async fn synthesize_edge(
        &self,
        sentence: &str,
        voice: &str,
        rate: f32,
        pitch: i32,
    ) -> Result<(Vec<u8>, String, u32, u32, Vec<Word>, bool), String> {
        let edge = self
            .edge
            .clone()
            .ok_or_else(|| "edge is not available".to_string())?;
        let result = edge.synthesize(sentence, voice, rate, pitch).await?;
        Ok((
            result.audio,
            result.mime.to_string(),
            result.sample_rate,
            result.duration_ms,
            result.words,
            false,
        ))
    }

    pub async fn synthesize_all(&self, request: SynthesisRequest) -> Result<Synthesis, String> {
        use futures_util::StreamExt;

        let mut stream = Box::pin(self.stream(request));
        let mut chunks: Vec<SynthChunk> = Vec::new();
        let mut engine = String::new();
        let mut voice = String::new();
        while let Some(item) = stream.next().await {
            match item? {
                StreamEvent::Chunk(chunk) => {
                    if engine.is_empty() {
                        engine = chunk.engine.clone();
                        voice = chunk.voice.clone();
                    }
                    chunks.push(chunk);
                }
                StreamEvent::Done {
                    engine: done_engine,
                    voice: done_voice,
                    ..
                } => {
                    engine = done_engine;
                    voice = done_voice;
                }
            }
        }
        if chunks.is_empty() {
            return Err("no audio produced".into());
        }
        let mime = chunks[0].mime.clone();
        let sample_rate = chunks[0].sample_rate;
        let audio = if mime.contains("wav") {
            let mut pcm = Vec::new();
            for chunk in &chunks {
                if let Some((data, _)) = wav_pcm(&chunk.audio) {
                    pcm.extend_from_slice(&data);
                }
            }
            build_wav(&pcm, sample_rate)
        } else {
            chunks
                .iter()
                .flat_map(|chunk| chunk.audio.clone())
                .collect()
        };
        let mut words = Vec::new();
        let mut offset = 0u32;
        for chunk in &chunks {
            for word in &chunk.words {
                words.push(Word {
                    text: word.text.clone(),
                    start_ms: word.start_ms.saturating_add(offset),
                    end_ms: word.end_ms.saturating_add(offset),
                });
            }
            offset = offset.saturating_add(chunk.duration_ms);
        }
        let total = duration_ms(&audio, &mime).unwrap_or(offset);
        Ok(Synthesis {
            audio,
            mime,
            sample_rate,
            duration_ms: total,
            words,
            engine,
            voice,
        })
    }

    pub fn clear_cache(&self) {
        let cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
        cache.clear();
    }

    pub fn cache_bytes(&self) -> u64 {
        let cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
        cache.len()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn health(&self) -> serde_json::Value {
        serde_json::json!({
            "engines": self.engine_ids(),
            "piper_voices": self.piper_voices().len(),
            "edge_voices_cached": self.edge_voices_len(),
            "concurrency": self.config.concurrency,
            "cache_mb": self.config.cache_mb,
            "cache_bytes": self.cache_bytes(),
        })
    }

    fn edge_voices_len(&self) -> usize {
        self.edge
            .as_ref()
            .map(|edge| edge.voices().len())
            .unwrap_or(0)
    }
}

fn quality_rank(quality: &str) -> u8 {
    match quality {
        "medium" => 0,
        "high" => 1,
        "low" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let dir = std::env::temp_dir().join(format!("ovml-engine-{}", uuid::Uuid::new_v4()));
        let mut config = Config::default();
        config.data_dir = dir;
        config.edge_enabled = false;
        config.cache_mb = 8;
        config.ensure_dirs().unwrap();
        config
    }

    #[tokio::test]
    async fn resolves_default_piper_voice_by_language() {
        let engine = RuntimeEngine::new(test_config()).unwrap();
        let (name, voice) = engine.resolve(None, None, Some("ru-RU")).await.unwrap();
        assert_eq!(name, "piper");
        assert!(voice.language.to_lowercase().starts_with("ru"));
    }

    #[tokio::test]
    async fn rejects_unknown_voice() {
        let engine = RuntimeEngine::new(test_config()).unwrap();
        assert!(engine
            .resolve(Some("piper"), Some("nope"), None)
            .await
            .is_err());
    }

    #[test]
    fn quality_order_prefers_medium() {
        assert!(quality_rank("medium") < quality_rank("high"));
        assert!(quality_rank("high") < quality_rank("low"));
    }
}
