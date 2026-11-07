//! Microsoft Edge read-aloud engine backed by the `kothok-edge-tts` crate.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kothok_edge_tts::{EdgeTts, TtsEvent};
use serde::{Deserialize, Serialize};

use crate::audio::mp3_duration_ms;
use crate::sentence::Word;
use crate::voice::VoiceInfo;

const RETRY_DELAYS_MS: [u64; 3] = [500, 1500, 3000];
const EDGE_SAMPLE_RATE: u32 = 24000;
const CACHE_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone)]
pub struct EdgeAudio {
    pub audio: Vec<u8>,
    pub mime: &'static str,
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub words: Vec<Word>,
}

pub struct EdgeEngine {
    voices_cache: Option<PathBuf>,
    voices: Mutex<Vec<VoiceInfo>>,
}

#[derive(Serialize, Deserialize)]
struct VoiceCacheFile {
    fetched_at: u64,
    voices: Vec<VoiceInfo>,
}

impl EdgeEngine {
    pub fn new(voices_cache: Option<PathBuf>) -> Self {
        kothok_edge_tts::init_tls();
        let voices = voices_cache
            .as_deref()
            .and_then(load_cached_voices)
            .unwrap_or_default();
        Self {
            voices_cache,
            voices: Mutex::new(voices),
        }
    }

    pub fn voices(&self) -> Vec<VoiceInfo> {
        let mut voices = self
            .voices
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        voices.sort_by(|a, b| a.id.cmp(&b.id));
        voices
    }

    pub fn is_empty(&self) -> bool {
        self.voices
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_empty()
    }

    pub async fn refresh_voices(&self) -> Result<usize, String> {
        let raw = kothok_edge_tts::list_voices()
            .await
            .map_err(|err| err.to_string())?;
        let voices: Vec<VoiceInfo> = raw
            .into_iter()
            .map(|item| VoiceInfo {
                id: item.short_name().to_string(),
                name: item.friendly_name().to_string(),
                language: item.locale().to_string(),
                gender: item.gender().to_lowercase(),
                quality: "neural".into(),
                sample_rate: EDGE_SAMPLE_RATE,
                sample_url: String::new(),
                downloaded: true,
            })
            .collect();
        if let Some(path) = &self.voices_cache {
            let payload = VoiceCacheFile {
                fetched_at: now_secs(),
                voices: voices.clone(),
            };
            if let Ok(bytes) = serde_json::to_vec(&payload) {
                write_atomic(path, &bytes);
            }
        }
        let count = voices.len();
        *self.voices.lock().unwrap_or_else(|err| err.into_inner()) = voices;
        Ok(count)
    }

    pub async fn synthesize(
        &self,
        text: &str,
        voice_id: &str,
        rate: f32,
        pitch: i32,
    ) -> Result<EdgeAudio, String> {
        let rate = format_rate(rate);
        let pitch = format_pitch(pitch);
        let language = language_for_voice(voice_id);
        let mut last_error = String::new();
        for (attempt, delay) in std::iter::once(0)
            .chain(RETRY_DELAYS_MS.iter().copied())
            .enumerate()
        {
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            match EdgeTts
                .synthesize_with_pitch(text, voice_id, &rate, &pitch, &language)
                .await
            {
                Ok(events) => {
                    let mut audio = Vec::new();
                    let mut words = Vec::new();
                    for event in events {
                        match event {
                            TtsEvent::Audio(chunk) => audio.extend_from_slice(&chunk),
                            TtsEvent::WordBoundary {
                                offset,
                                duration,
                                text,
                            } => words.push(Word {
                                text,
                                start_ms: (offset / 10_000) as u32,
                                end_ms: ((offset + duration) / 10_000) as u32,
                            }),
                            TtsEvent::TurnEnd => {}
                        }
                    }
                    if audio.is_empty() {
                        last_error = "no audio received".into();
                        continue;
                    }
                    let duration_ms = mp3_duration_ms(&audio)
                        .or_else(|| words.last().map(|word| word.end_ms))
                        .unwrap_or(0);
                    return Ok(EdgeAudio {
                        audio,
                        mime: "audio/mpeg",
                        sample_rate: EDGE_SAMPLE_RATE,
                        duration_ms,
                        words,
                    });
                }
                Err(err) => {
                    last_error = err.to_string();
                    if !is_transient(&err) {
                        break;
                    }
                    log::warn!(
                        "edge synthesis attempt {} failed for {}: {}",
                        attempt + 1,
                        voice_id,
                        last_error
                    );
                }
            }
        }
        Err(format!("edge synthesis failed: {last_error}"))
    }
}

fn is_transient(err: &kothok_edge_tts::TtsError) -> bool {
    matches!(
        err,
        kothok_edge_tts::TtsError::Connect(_)
            | kothok_edge_tts::TtsError::Ws(_)
            | kothok_edge_tts::TtsError::Io(_)
            | kothok_edge_tts::TtsError::NoAudio
    )
}

fn format_rate(rate: f32) -> String {
    let percent = (((rate.clamp(0.5, 2.0) - 1.0) * 100.0).round() as i32).clamp(-100, 100);
    format!("{percent:+}%")
}

fn format_pitch(pitch: i32) -> String {
    format!("{:+}Hz", pitch.clamp(-100, 100))
}

fn language_for_voice(voice_id: &str) -> String {
    let mut parts = voice_id.split('-');
    match (parts.next(), parts.next()) {
        (Some(lang), Some(region)) if !lang.is_empty() && !region.is_empty() => {
            format!("{lang}-{region}")
        }
        _ => "en-US".into(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn load_cached_voices(path: &Path) -> Option<Vec<VoiceInfo>> {
    let payload: VoiceCacheFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if now_secs().saturating_sub(payload.fetched_at) > CACHE_TTL_SECS {
        return None;
    }
    if payload.voices.is_empty() {
        return None;
    }
    Some(payload.voices)
}

fn write_atomic(path: &Path, data: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
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

    #[test]
    fn formats_rate_percent() {
        assert_eq!(format_rate(1.0), "+0%");
        assert_eq!(format_rate(1.1), "+10%");
        assert_eq!(format_rate(0.9), "-10%");
        assert_eq!(format_rate(3.0), "+100%");
    }

    #[test]
    fn formats_pitch_hertz() {
        assert_eq!(format_pitch(0), "+0Hz");
        assert_eq!(format_pitch(20), "+20Hz");
        assert_eq!(format_pitch(-15), "-15Hz");
        assert_eq!(format_pitch(500), "+100Hz");
    }

    #[test]
    fn derives_language_from_voice() {
        assert_eq!(language_for_voice("ru-RU-DmitryNeural"), "ru-RU");
        assert_eq!(language_for_voice("bad"), "en-US");
    }

    #[test]
    fn loads_fresh_cache_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.json");
        let payload = VoiceCacheFile {
            fetched_at: now_secs(),
            voices: vec![VoiceInfo {
                id: "en-US-AriaNeural".into(),
                name: "Aria".into(),
                language: "en-US".into(),
                gender: "female".into(),
                quality: "neural".into(),
                sample_rate: EDGE_SAMPLE_RATE,
                sample_url: String::new(),
                downloaded: true,
            }],
        };
        write_atomic(&path, &serde_json::to_vec(&payload).unwrap());
        let engine = EdgeEngine::new(Some(path));
        assert_eq!(engine.voices().len(), 1);
    }
}
